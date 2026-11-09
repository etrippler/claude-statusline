//! What this binary keeps between runs, all in /tmp: a stale-while-revalidate
//! cache for the slow lookups, and the session handoff from the main line to
//! the subagent rows.

use crate::{ci::Pr, git::Tracking, model::Effort, now, proc::spawn_detached};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use std::{
    collections::HashMap,
    fs, io,
    path::{Path, PathBuf},
    process,
};

/// Worker subcommands: this binary, detached, refreshing one cache entry.
pub const REFRESH_PR: &str = "--refresh-pr";
pub const REFRESH_DIFF: &str = "--refresh-diff";

const PR_MAX_AGE: f64 = 30.0;
/// How long a claimed refresh blocks another: past each worker's own timeouts
/// (gh 5s; merge-base 2s + diff 30s), so a hung one can't pile up successors.
const PR_CLAIM: f64 = 10.0;
const DIFF_CLAIM: f64 = 35.0;
/// Entries untouched this long are dropped on the next write.
const RETAIN: f64 = 86_400.0;

fn tmp(name: &str) -> PathBuf {
    format!("/tmp/claude-statusline-rs-{name}").into()
}

/// Missing or malformed files read as the default: config and state may not exist yet.
pub fn read_json<T: DeserializeOwned + Default>(p: &Path) -> T {
    fs::read(p).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default()
}

/// Write-then-rename, so concurrent readers never see a torn file.
pub fn write_atomic(p: &Path, contents: &str) -> io::Result<()> {
    let tmp = p.with_extension(format!("{}.tmp", process::id()));
    fs::write(&tmp, contents)?;
    fs::rename(&tmp, p)
}

pub fn write_json(p: &Path, v: &impl Serialize) -> io::Result<()> {
    write_atomic(p, &serde_json::to_string(v)?)
}

/// Lines added/removed since the merge-base with the default branch.
#[derive(Clone, Copy, Serialize, Deserialize)]
pub struct Diff {
    pub added: u64,
    pub removed: u64,
    /// Seconds the last `git diff` took.
    #[serde(default)]
    pub took: f64,
}

impl Diff {
    /// Freshness scales with cost: ~20× the last run, so a 50ms diff refreshes
    /// every second and a 3s one every minute.
    fn max_age(&self) -> f64 {
        (self.took * 20.0).clamp(1.0, 60.0)
    }
}

/// A value and when it was refreshed — or, while a worker holds the claim,
/// when that claim lapses (a future time).
#[derive(Clone, Serialize, Deserialize)]
struct Entry<T> {
    t: f64,
    v: Option<T>,
}

#[derive(Default, Serialize, Deserialize)]
#[serde(default)]
struct Entries {
    pr: HashMap<String, Entry<Pr>>,
    diff: HashMap<String, Entry<Diff>>,
    tracking: HashMap<String, Entry<Tracking>>,
}

impl Entries {
    fn load() -> Self {
        read_json(&tmp("cache.json"))
    }

    fn is_empty(&self) -> bool {
        self.pr.is_empty() && self.diff.is_empty() && self.tracking.is_empty()
    }

    /// Lay `changes` over a fresh read, not over our earlier snapshot: a
    /// worker may have landed a result since, and rewriting the whole file
    /// from the snapshot would erase it.
    fn merge(changes: Entries) {
        let mut e = Self::load();
        e.pr.extend(changes.pr);
        e.diff.extend(changes.diff);
        e.tracking.extend(changes.tracking);
        let cutoff = now() - RETAIN;
        e.pr.retain(|_, e| e.t > cutoff);
        e.diff.retain(|_, e| e.t > cutoff);
        e.tracking.retain(|_, e| e.t > cutoff);
        let _ = write_json(&tmp("cache.json"), &e);
    }
}

fn pr_key(owner: &str, name: &str, branch: &str) -> String {
    format!("{owner}/{name}:{branch}")
}

fn diff_key(top: &Path, branch: &str) -> String {
    format!("{}:{branch}", top.display())
}

/// Read once per render; `commit` writes what changed, then starts the refreshes.
pub struct Cache {
    entries: Entries,
    changed: Entries,
    refreshes: Vec<Vec<String>>,
}

impl Cache {
    pub fn load() -> Self {
        Cache { entries: Entries::load(), changed: Entries::default(), refreshes: Vec::new() }
    }

    pub fn pr(&mut self, owner: &str, name: &str, branch: &str) -> Option<Pr> {
        let key = pr_key(owner, name, branch);
        self.serve(|e| &mut e.pr, key, PR_MAX_AGE, PR_CLAIM, &[REFRESH_PR, branch, owner, name])
    }

    pub fn diff(&mut self, top: &Path, branch: &str, cwd: &Path) -> Option<Diff> {
        let key = diff_key(top, branch);
        let max_age = self.entries.diff.get(&key).and_then(|e| e.v.as_ref()).map_or(0.0, Diff::max_age);
        self.serve(|e| &mut e.diff, key, max_age, DIFF_CLAIM, &[REFRESH_DIFF, &cwd.to_string_lossy()])
    }

    pub fn tracking(&self, key: &str) -> Option<&Tracking> {
        self.entries.tracking.get(key)?.v.as_ref()
    }

    pub fn set_tracking(&mut self, key: String, t: Tracking) {
        let entry = Entry { t: now(), v: Some(t) };
        self.entries.tracking.insert(key.clone(), entry.clone());
        self.changed.tracking.insert(key, entry);
    }

    /// The cached value. Once it's older than `max_age` (or missing), claim it
    /// for `claim` seconds and queue `refresh`; the claim is what keeps
    /// concurrent sessions single-flight, and the worker's write releases it.
    fn serve<T: Clone>(
        &mut self,
        map: impl Fn(&mut Entries) -> &mut HashMap<String, Entry<T>>,
        key: String,
        max_age: f64,
        claim: f64,
        refresh: &[&str],
    ) -> Option<T> {
        let entry = map(&mut self.entries).entry(key.clone()).or_insert(Entry { t: f64::NEG_INFINITY, v: None });
        let value = entry.v.clone();
        if now() - entry.t > max_age {
            entry.t = now() + claim;
            let claimed = entry.clone();
            map(&mut self.changed).insert(key, claimed);
            self.refreshes.push(refresh.iter().map(|&s| s.to_owned()).collect());
        }
        value
    }

    /// Stamps land on disk before the workers start.
    pub fn commit(self) {
        if !self.changed.is_empty() {
            Entries::merge(self.changed);
        }
        for args in &self.refreshes {
            spawn_detached(args);
        }
    }

    pub fn put_pr(owner: &str, name: &str, branch: &str, pr: Option<Pr>) {
        let mut e = Entries::default();
        e.pr.insert(pr_key(owner, name, branch), Entry { t: now(), v: pr });
        Entries::merge(e);
    }

    pub fn put_diff(top: &Path, branch: &str, diff: Diff) {
        let mut e = Entries::default();
        e.diff.insert(diff_key(top, branch), Entry { t: now(), v: Some(diff) });
        Entries::merge(e);
    }
}

/// Subagent rows carry no session model or effort; the main line has both,
/// live (mid-session /model and /effort included), and leaves them here.
#[derive(Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Session {
    pub model: String,
    pub effort: Option<Effort>,
}

impl Session {
    fn path(id: &str) -> Option<PathBuf> {
        let id: String = id.chars().filter(|c| c.is_alphanumeric() || matches!(c, '_' | '-')).collect();
        (!id.is_empty()).then(|| tmp(&format!("session-{id}.json")))
    }

    pub fn load(id: &str) -> Session {
        Self::path(id).map(|p| read_json(&p)).unwrap_or_default()
    }

    /// Written only on change: most renders are no-ops.
    pub fn save(&self, id: &str) {
        if let Some(p) = Self::path(id)
            && read_json::<Session>(&p) != *self
        {
            let _ = write_json(&p, self);
        }
    }
}
