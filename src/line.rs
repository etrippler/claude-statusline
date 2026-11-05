//! The main status line: location on the left, session on the right,
//! account-level meta right-justified.

use crate::{
    ansi::{BOLD, blue, bold_cyan, cyan, dim, green, justify, link, magenta, red, white, yellow},
    ci,
    fmt::{clock, cost, tokens},
    git::{self, Status},
    model::Effort,
    now,
    settings::{Settings, Toggle, Toggles},
    store::{Cache, Session},
};
use serde::Deserialize;
use std::{env, path::PathBuf};

/// The slice of Claude Code's statusline JSON we read. Nulls and empty strings
/// are pruned before this sees them, so absent-or-empty is always the default.
#[derive(Deserialize, Default)]
#[serde(default)]
pub struct Input {
    session_id: Option<String>,
    workspace: Workspace,
    model: Model,
    effort: Level,
    output_style: Name,
    context_window: ContextWindow,
    cost: Cost,
    prompt_cache: PromptCache,
    rate_limits: RateLimits,
    version: Option<String>,
    pr: Url,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct Workspace {
    current_dir: Option<PathBuf>,
    project_dir: Option<PathBuf>,
    repo: Option<Repo>,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct Repo {
    host: String,
    owner: String,
    name: String,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct Model {
    id: String,
    display_name: Option<String>,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct Level {
    level: Option<Effort>,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct Name {
    name: Option<String>,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct Url {
    url: Option<String>,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct ContextWindow {
    context_window_size: Option<f64>,
    current_usage: Usage,
    used_percentage: f64,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct Usage {
    input_tokens: f64,
    cache_creation_input_tokens: f64,
    cache_read_input_tokens: f64,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct Cost {
    total_cost_usd: f64,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct PromptCache {
    caching_observed: bool,
    warm: bool,
    ttl: Option<String>,
    expires_at: f64,
    recache_tokens_if_cold: f64,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct RateLimits {
    five_hour: Option<Window>,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct Window {
    used_percentage: f64,
    resets_at: Option<f64>,
}

pub fn render(input: &Input, on: &Toggles) -> String {
    let ws = &input.workspace;
    let cwd = ws.current_dir.clone().unwrap_or_else(|| env::current_dir().unwrap_or_default());
    let project = ws.project_dir.as_ref().unwrap_or(&cwd);
    if let Some(id) = &input.session_id {
        Session { model: input.model.id.clone(), effort: input.effort.level.clone() }.save(id);
    }
    let mut cache = Cache::load();
    let git = git::status(&cwd, &mut cache);
    let repo = ws.repo.as_ref().filter(|r| !r.host.is_empty() && !r.owner.is_empty());

    let left = git
        .as_ref()
        .map(|g| {
            let url =
                repo.zip(g.branch.as_ref()).map(|(r, b)| format!("https://{}/{}/{}/tree/{b}", r.host, r.owner, r.name));
            location(g, url.as_deref(), on)
        })
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| bold_cyan(cwd.file_name().map_or("?".into(), |n| n.to_string_lossy())));
    let mut parts = vec![left];

    if on.on(Toggle::Ci)
        && let (Some(r), Some(g)) = (repo, &git)
    {
        parts.extend(ci::badge(&r.owner, &r.name, g, input.pr.url.as_deref(), &mut cache));
    }
    cache.commit();

    if on.on(Toggle::Model) {
        let name = input.model.display_name.as_deref().unwrap_or("Claude");
        let name = name.strip_prefix("Claude ").unwrap_or(name).replace(" context)", ")");
        let mut s = match &input.effort.level {
            Some(e) => blue(format_args!("{name} {BOLD}{}", e.letter())),
            None => blue(name),
        };
        if let Some(style) = input.output_style.name.as_deref().filter(|s| !s.eq_ignore_ascii_case("default")) {
            s += " ";
            s += &dim(format_args!("~ {style}"));
        }
        parts.push(s);
    }

    let cw = &input.context_window;
    let window = cw.context_window_size.filter(|&n| n > 0.0).unwrap_or(200_000.0);
    let used = Some(cw.current_usage.total())
        .filter(|&n| n > 0.0)
        // Antigravity (agy) sends no current_usage, only the percentage
        .unwrap_or_else(|| (cw.used_percentage * window / 100.0).round());
    if on.on(Toggle::Percent) {
        parts.push(yellow(format_args!("{:.1}%", used * 100.0 / window)));
    }
    if on.on(Toggle::Context) && used > 0.0 {
        // soft limit (autocompact) in white, hard limit dimmed
        let k = |n: f64| (n / 1000.0).round();
        let at = Settings::load(project).autocompact_at(&input.model.id, window);
        let mut s = white(format_args!("{:.1}/{}", used / 1000.0, k(at.unwrap_or(window))));
        if at.is_some() {
            s += &dim(format_args!("/{}", k(window)));
        }
        parts.push(s);
    }
    if on.on(Toggle::Cost) {
        parts.push(green(cost(input.cost.total_cost_usd)));
    }

    let sep = format!(" {} ", dim("|"));
    let line = parts.join(&sep);
    // Account-level meta right-justifies. Claude Code's statusline area is a few
    // cells narrower than COLUMNS; overshooting gets the tail truncated to "…".
    let meta: Vec<String> = [
        (Toggle::Cache, prompt_cache(&input.prompt_cache)),
        (Toggle::Limits, limits(&input.rate_limits)),
        (Toggle::Version, input.version.as_ref().map(|v| dim(format_args!("v{v}")))),
    ]
    .into_iter()
    .filter_map(|(t, s)| s.filter(|_| on.on(t)))
    .collect();
    if meta.is_empty() {
        return line;
    }
    let cols = env::var("COLUMNS").ok().and_then(|c| c.parse().ok());
    justify(&line, &meta.join(&sep), cols, 4, &sep)
}

/// "repo ⎇ worktree/subdir on  branch ↑1 ↓2 Δ+10/-3"
fn location(g: &Status, url: Option<&str>, on: &Toggles) -> String {
    // When the branch just restates the worktree name, the worktree chip absorbs
    // it: no "on  branch", and the hyperlink moves to the chip.
    fn norm(s: &str) -> &str {
        ["worktree-", "worktree/", "wt-", "wt/"].iter().find_map(|p| s.strip_prefix(p)).unwrap_or(s)
    }
    let redundant = matches!((&g.branch, &g.worktree), (Some(b), Some(w)) if norm(b) == norm(w));
    let mut parts = Vec::new();
    if on.on(Toggle::Repo) && !g.name.is_empty() {
        let mut s = bold_cyan(&g.name);
        if let Some(wt) = &g.worktree {
            s += " ";
            s += &link(url.filter(|_| redundant), cyan(format_args!("\u{2387} {wt}")));
        }
        if let Some(sub) = &g.subdir {
            s += &cyan(format_args!("/{sub}"));
        }
        parts.push(s);
    }
    if on.on(Toggle::Branch)
        && let Some(branch) = &g.branch
    {
        if !redundant {
            if !parts.is_empty() {
                parts.push(dim("on"));
            }
            parts.push(link(url, magenta(format_args!("\u{e0a0} {branch}"))));
        }
        // quiet working-state markers; only behind/no-upstream go yellow
        match &g.upstream {
            Some(u) => {
                parts.extend((u.ahead > 0).then(|| dim(format_args!("↑{}", u.ahead))));
                parts.extend((u.behind > 0).then(|| yellow(format_args!("↓{}", u.behind))));
            }
            None => parts.push(yellow("(local)")),
        }
        if let Some(d) = g.diff.filter(|d| on.on(Toggle::Diff) && d.added + d.removed > 0) {
            parts.push(dim(format_args!("Δ+{}/-{}", d.added, d.removed)));
        }
    }
    parts.join(" ")
}

impl Usage {
    fn total(&self) -> f64 {
        self.input_tokens + self.cache_creation_input_tokens + self.cache_read_input_tokens
    }
}

/// "cache 42m" while warm; "cold ↻78k" = what the next turn re-writes.
fn prompt_cache(pc: &PromptCache) -> Option<String> {
    if !pc.caching_observed {
        return None;
    }
    let left = pc.expires_at - now();
    if !pc.warm || left <= 0.0 {
        let mut s = red("cold");
        if pc.recache_tokens_if_cold > 0.0 {
            s += " ";
            s += &dim(format_args!("↻{}", tokens(pc.recache_tokens_if_cold)));
        }
        return Some(s);
    }
    let ttl = if pc.ttl.as_deref() == Some("1h") { 3600.0 } else { 300.0 };
    let paint = if left > ttl / 2.0 {
        green
    } else if left > f64::min(300.0, ttl / 5.0) {
        yellow
    } else {
        red // 1h: red under 5m
    };
    Some(dim("cache ") + &paint(format_args!("{}m", (left / 60.0).round().max(1.0))))
}

fn limits(rl: &RateLimits) -> Option<String> {
    let w = rl.five_hour.as_ref().filter(|w| w.used_percentage > 0.0)?;
    let pct = w.used_percentage;
    let paint = if pct >= 80.0 {
        red
    } else if pct >= 50.0 {
        yellow
    } else {
        green
    };
    let mut s = paint(format_args!("{}%", pct.round()));
    if let Some(reset) = w.resets_at {
        s += " ";
        s += &dim(format_args!("@ {}", clock(reset)));
    }
    Some(s)
}
