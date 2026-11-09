//! The CI badge: Claude Code's own PR rollup when it has a verdict, else ours
//! from `gh`, refreshed in the background.

use crate::{
    ansi::{green, link, red, yellow},
    git::Status,
    proc::output,
    settings::claude_dir,
    store::{Cache, read_json},
};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::{process::Command, time::Duration};

#[derive(Clone, Copy, Serialize, Deserialize)]
pub enum Ci {
    Failure,
    Pending,
    Success,
}

/// A PR with at least one check.
#[derive(Clone, Serialize, Deserialize)]
pub struct Pr {
    ci: Ci,
    url: String,
}

pub fn badge(owner: &str, name: &str, git: &Status, cc_pr_url: Option<&str>, cache: &mut Cache) -> Option<String> {
    let branch = git.branch.as_deref()?;
    if git.default_branch.as_deref() == Some(branch) || git.upstream.is_none() {
        return None;
    }
    if let Some(url) = cc_pr_url
        && let Some(ci) = from_claude_code(url)
    {
        return Some(render(ci, url));
    }
    // Claude Code reports no PR on non-token hosts and unseen branches — own the fetch.
    cache.pr(owner, name, branch).map(|pr| render(pr.ci, &pr.url))
}

fn render(ci: Ci, url: &str) -> String {
    let badge = match ci {
        Ci::Failure => red("CI ✗"),
        Ci::Pending => yellow("CI ●"),
        Ci::Success => green("CI ✓"),
    };
    link(Some(url), badge)
}

/// Claude Code persists its own GraphQL PR rollup keyed by URL — share it.
fn from_claude_code(url: &str) -> Option<Ci> {
    let all: Map<String, Value> = read_json(&claude_dir().join("gh-pr-status-cache.json"));
    let checks = all.get(url)?.get("checks")?;
    let any = |k: &str| checks.get(k).and_then(Value::as_f64).is_some_and(|n| n > 0.0);
    [("failed", Ci::Failure), ("pending", Ci::Pending), ("passed", Ci::Success)]
        .into_iter()
        .find_map(|(k, ci)| any(k).then_some(ci))
}

/// Worker: one `gh pr view`, written back to the cache.
pub fn refresh(branch: &str, owner: &str, name: &str) {
    Cache::put_pr(owner, name, branch, fetch(branch, &format!("{owner}/{name}")));
}

fn fetch(branch: &str, slug: &str) -> Option<Pr> {
    let args = ["pr", "view", branch, "--repo", slug, "--json", "url,statusCheckRollup"];
    let d: Value = serde_json::from_str(&output(Command::new("gh").args(args), Duration::from_secs(5))?).ok()?;
    // statusCheckRollup mixes CheckRun {status, conclusion} and StatusContext
    // {state} nodes; read both or commit-status failures pass silently.
    let checks = d["statusCheckRollup"].as_array().filter(|c| !c.is_empty())?;
    let any = |key: &str, values: &[&str]| checks.iter().any(|c| c[key].as_str().is_some_and(|v| values.contains(&v)));
    let ci = if any("conclusion", &["FAILURE", "ERROR"]) || any("state", &["FAILURE", "ERROR"]) {
        Ci::Failure
    } else if any("status", &["IN_PROGRESS", "PENDING", "QUEUED"]) || any("state", &["PENDING", "EXPECTED"]) {
        Ci::Pending
    } else {
        Ci::Success
    };
    Some(Pr { ci, url: d["url"].as_str()?.to_owned() })
}
