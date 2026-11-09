//! Repo state for the main line, mostly without running git.
//!
//! - repo, worktree, subdir and branch are file reads from `.git`, the way git
//!   itself discovers them;
//! - upstream ahead/behind and the default branch take one `git for-each-ref`,
//!   cached until the ref files it depends on change — so a steady-state render
//!   spawns nothing;
//! - the full-tree diff only ever runs in a detached worker.

use crate::{
    proc::output,
    store::{Cache, Diff},
};
use serde::{Deserialize, Serialize};
use std::{
    env,
    ffi::OsStr,
    fmt::Write,
    fs,
    path::{Path, PathBuf},
    process::Command,
    sync::OnceLock,
    time::{Duration, Instant, UNIX_EPOCH},
};

const TIMEOUT: Duration = Duration::from_secs(2);

/// `git -C dir …`, lock-free: a statusline must never take the index lock —
/// `git diff` opportunistically rewrites the index, and sessions contending on
/// it turn a slow render into a stuck one.
fn git(dir: &Path, args: impl IntoIterator<Item = impl AsRef<OsStr>>) -> Command {
    let mut cmd = Command::new(binary());
    cmd.arg("-C").arg(dir).args(args).env("GIT_OPTIONAL_LOCKS", "0");
    cmd
}

/// Apple's /usr/bin/git is an xcrun shim that costs ~10ms a call; go straight
/// to the toolchain it would pick.
fn binary() -> &'static Path {
    static BIN: OnceLock<PathBuf> = OnceLock::new();
    BIN.get_or_init(|| {
        let on_path = env::var_os("PATH")
            .and_then(|p| env::split_paths(&p).map(|d| d.join("git")).find(|p| p.is_file()))
            .unwrap_or_else(|| "git".into());
        if cfg!(target_os = "macos") && on_path == Path::new("/usr/bin/git") {
            let dev = env::var_os("DEVELOPER_DIR").map_or_else(|| "/var/db/xcode_select_link".into(), PathBuf::from);
            let real = dev.join("usr/bin/git");
            if real.is_file() {
                return real;
            }
        }
        on_path
    })
}

fn file_name(p: &Path) -> String {
    p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()
}

#[derive(Clone, Serialize, Deserialize)]
pub struct Upstream {
    name: String,
    pub ahead: u32,
    pub behind: u32,
}

/// One `for-each-ref`, and the ref-file fingerprint it's valid for.
#[derive(Clone, Default, Serialize, Deserialize)]
pub struct Tracking {
    fingerprint: String,
    upstream: Option<Upstream>,
    default_branch: Option<String>,
}

pub struct Status {
    /// The main checkout's directory name, shared by all its worktrees.
    pub name: String,
    pub worktree: Option<String>,
    pub subdir: Option<String>,
    pub branch: Option<String>,
    pub default_branch: Option<String>,
    pub upstream: Option<Upstream>,
    pub diff: Option<Diff>,
}

struct Repo {
    top: PathBuf,
    git_dir: PathBuf,
    common_dir: PathBuf,
}

impl Repo {
    fn discover(cwd: &Path) -> Option<Repo> {
        cwd.ancestors().find_map(|dir| {
            let dot = dir.join(".git");
            let git_dir = if dot.is_dir() {
                dot
            } else {
                dir.join(fs::read_to_string(&dot).ok()?.strip_prefix("gitdir:")?.trim())
            };
            let common_dir = match fs::read_to_string(git_dir.join("commondir")) {
                Ok(rel) => git_dir.join(rel.trim()).canonicalize().ok()?,
                Err(_) => git_dir.clone(),
            };
            Some(Repo { top: dir.into(), git_dir, common_dir })
        })
    }

    fn branch(&self) -> Option<String> {
        let head = fs::read_to_string(self.git_dir.join("HEAD")).ok()?;
        match head.trim().strip_prefix("ref: refs/heads/")? {
            // reftable repos keep a placeholder HEAD; ask git
            ".invalid" => output(&mut git(&self.top, ["branch", "--show-current"]), TIMEOUT),
            b => Some(b.to_owned()),
        }
        .filter(|b| !b.is_empty())
    }

    /// Every file a ref update rewrites (git writes a lock file, then renames it
    /// into place): the loose refs we read, their packed fallback, the config
    /// that names the upstream, and reftable's table list.
    fn fingerprint(&self, branch: &str, upstream: Option<&str>) -> String {
        let c = &self.common_dir;
        let mut fp = String::new();
        let mut file = |p: PathBuf| {
            let stamp =
                fs::metadata(&p).and_then(|m| m.modified()).ok().and_then(|t| t.duration_since(UNIX_EPOCH).ok());
            let _ = write!(fp, "{}|", stamp.map_or(0, |d| d.as_nanos()));
        };
        file(self.git_dir.join("HEAD"));
        file(c.join(format!("refs/heads/{branch}")));
        file(c.join("refs/remotes/origin/HEAD"));
        file(c.join("packed-refs"));
        file(c.join("config"));
        file(c.join("reftable/tables.list"));
        if let Some(u) = upstream {
            file(c.join(format!("refs/remotes/{u}")));
            file(c.join(format!("refs/heads/{u}")));
        }
        fp
    }

    /// Upstream ahead/behind (None when unset or gone) and origin's default branch.
    fn tracking(&self, branch: &str, cache: &mut Cache) -> Tracking {
        let key = self.git_dir.to_string_lossy().into_owned();
        let cached = cache.tracking(&key);
        // Taken before the query: if refs move mid-query, the next render sees a
        // new fingerprint and asks again, rather than trusting stale counts.
        let fingerprint = self.fingerprint(branch, cached.and_then(|t| t.upstream.as_ref()).map(|u| u.name.as_str()));
        if let Some(t) = cached.filter(|t| t.fingerprint == fingerprint) {
            return t.clone();
        }

        let head = format!("refs/heads/{branch}");
        let format = "--format=%(refname)\t%(upstream:short)\t%(upstream:track,nobracket)\t%(symref:lstrip=3)";
        let out = output(&mut git(&self.top, ["for-each-ref", format, &head, "refs/remotes/origin/HEAD"]), TIMEOUT);
        let mut t = Tracking { fingerprint, ..Tracking::default() };
        for line in out.as_deref().unwrap_or_default().lines() {
            match line.split('\t').collect::<Vec<_>>()[..] {
                [r, up, track, _] if r == head && !up.is_empty() && track != "gone" => {
                    let count = |what| track.split(", ").find_map(|p| p.strip_prefix(what)?.trim().parse().ok());
                    let (ahead, behind) = (count("ahead").unwrap_or(0), count("behind").unwrap_or(0));
                    t.upstream = Some(Upstream { name: up.to_owned(), ahead, behind });
                }
                ["refs/remotes/origin/HEAD", _, _, d] if !d.is_empty() => t.default_branch = Some(d.to_owned()),
                _ => {}
            }
        }
        cache.set_tracking(key, t.clone());
        t
    }
}

pub fn status(cwd: &Path, cache: &mut Cache) -> Option<Status> {
    let cwd = cwd.canonicalize().ok()?;
    let repo = Repo::discover(&cwd)?;
    let name = if repo.common_dir.file_name() == Some(".git".as_ref()) {
        repo.common_dir.parent().map(file_name).unwrap_or_default()
    } else {
        file_name(&repo.top) // submodule or bare layout
    };
    let branch = repo.branch();
    let tracking = branch.as_deref().map(|b| repo.tracking(b, cache)).unwrap_or_default();
    // The full-tree shortstat is the one expensive call on a big repo — never in
    // the render path.
    let diff = branch.as_deref().and_then(|b| cache.diff(&repo.top, b, &cwd));
    Some(Status {
        name,
        worktree: (repo.git_dir != repo.common_dir).then(|| file_name(&repo.top)),
        subdir: cwd.strip_prefix(&repo.top).ok().filter(|p| !p.as_os_str().is_empty()).map(|p| p.display().to_string()),
        branch,
        default_branch: tracking.default_branch,
        upstream: tracking.upstream,
        diff,
    })
}

/// Worker: working tree + branch commits vs the default-branch merge-base — the
/// size of the eventual PR, independent of session boundaries.
pub fn refresh_diff(cwd: &Path) {
    let Some(repo) = Repo::discover(cwd) else { return };
    let Some(branch) = repo.branch() else { return };
    let run = |args: &[&str], secs| output(&mut git(cwd, args), Duration::from_secs(secs));
    let started = Instant::now();
    let stat = run(&["merge-base", "origin/HEAD", "HEAD"], 2)
        .and_then(|base| run(&["diff", "--shortstat", &base], 30))
        .unwrap_or_default();
    let took = started.elapsed().as_secs_f64();
    // "3 files changed, 10 insertions(+), 2 deletions(-)"
    let count = |word| {
        stat.split(", ")
            .find_map(|p| p.trim().split_once(' ').filter(|(_, w)| w.starts_with(word))?.0.parse().ok())
            .unwrap_or(0)
    };
    Cache::put_diff(&repo.top, &branch, Diff { added: count("insertion"), removed: count("deletion"), took });
}
