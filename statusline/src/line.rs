//! The main status line: location on the left, session on the right,
//! account-level meta right-justified.

use crate::{
    ansi::{BOLD, blue, bold_cyan, cyan, dim, green, justify, link, magenta, red, white, yellow},
    ci,
    fmt::{clock, cost, tokens},
    git::{self, Remote, Status},
    model::Effort,
    now,
    settings::{Settings, Toggle, Toggles},
    store::Cache,
};
use serde::Deserialize;
use std::{collections::HashMap, env, path::PathBuf};

/// The slice of Claude Code / Antigravity's statusline JSON we read. Nulls and
/// empty strings are pruned before this sees them, so absent-or-empty is always
/// the default.
#[derive(Deserialize, Default)]
#[serde(default)]
pub struct Input {
    product: Option<String>,
    workspace: Workspace,
    model: Model,
    effort: Level,
    output_style: Name,
    context_window: ContextWindow,
    cost: Cost,
    prompt_cache: PromptCache,
    rate_limits: RateLimits,
    quota: HashMap<String, QuotaBucket>,
    version: Option<String>,
    terminal_width: Option<usize>,
    pr: Url,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct Workspace {
    current_dir: Option<PathBuf>,
    project_dir: Option<PathBuf>,
    repo: Option<Remote>,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct Model {
    id: String,
    display_name: Option<String>,
    /// Antigravity nests effort inside `model`.
    effort: Option<Effort>,
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
    current_usage: CurrentUsage,
    used_percentage: f64,
}

#[derive(Deserialize)]
#[serde(untagged)]
enum CurrentUsage {
    Tokens(f64),
    Breakdown(Usage),
}

impl Default for CurrentUsage {
    fn default() -> Self {
        Self::Tokens(0.0)
    }
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
    total_cost_usd: Option<f64>,
    /// Antigravity pay-as-you-go / estimated spend fields.
    total_usd: Option<f64>,
    estimated: Option<f64>,
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

#[derive(Deserialize, Default)]
#[serde(default)]
struct QuotaBucket {
    remaining_fraction: Option<f64>,
    reset_in_seconds: Option<f64>,
    disabled: bool,
}

pub fn render(input: &Input, on: &Toggles) -> String {
    let is_agy = input.product.as_deref() == Some("antigravity");
    let ws = &input.workspace;
    let cwd = ws.current_dir.clone().unwrap_or_else(|| env::current_dir().unwrap_or_default());
    let project = ws.project_dir.as_ref().unwrap_or(&cwd);
    let effort = input.effort.level.as_ref().or(input.model.effort.as_ref());
    let mut cache = Cache::load();
    let git = git::status(&cwd, &mut cache);
    let repo = ws
        .repo
        .as_ref()
        .filter(|r| !r.host.is_empty() && !r.owner.is_empty() && !r.name.is_empty())
        .or_else(|| git.as_ref().and_then(|g| g.remote.as_ref()));

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
        let fallback = if is_agy { "Antigravity" } else { "Claude" };
        let name = input.model.display_name.as_deref().unwrap_or(fallback);
        let name = name.strip_prefix("Claude ").unwrap_or(name).replace(" context)", ")");
        let name = if effort.is_some() { strip_effort_suffix(&name) } else { name.as_str() };
        let mut s = match effort {
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
        let at = (!is_agy).then(|| Settings::load(project).autocompact_at(&input.model.id, window)).flatten();
        let mut s = white(format_args!("{:.1}/{}", used / 1000.0, k(at.unwrap_or(window))));
        if at.is_some() {
            s += &dim(format_args!("/{}", k(window)));
        }
        parts.push(s);
    }
    if on.on(Toggle::Cost)
        && let Some(usd) = input.cost.usd().or_else(|| (!is_agy).then_some(0.0))
    {
        parts.push(green(cost(usd)));
    }

    let sep = format!(" {} ", dim("|"));
    let line = parts.join(&sep);
    // Account-level meta right-justifies. Claude Code's statusline area is a few
    // cells narrower than COLUMNS; overshooting gets the tail truncated to "…".
    let meta: Vec<String> = [
        (Toggle::Cache, prompt_cache(&input.prompt_cache)),
        (Toggle::Limits, limits(&input.rate_limits, &input.quota, &input.model.id)),
        (Toggle::Version, input.version.as_ref().map(|v| dim(format_args!("v{v}")))),
    ]
    .into_iter()
    .filter_map(|(t, s)| s.filter(|_| on.on(t)))
    .collect();
    if meta.is_empty() {
        return line;
    }
    let cols =
        input.terminal_width.filter(|&c| c > 0).or_else(|| env::var("COLUMNS").ok().and_then(|c| c.parse().ok()));
    let margin = if is_agy { 2 } else { 4 };
    justify(&line, &meta.join(&sep), cols, margin, &sep)
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

impl CurrentUsage {
    fn total(&self) -> f64 {
        match self {
            Self::Tokens(n) => *n,
            Self::Breakdown(u) => u.input_tokens + u.cache_creation_input_tokens + u.cache_read_input_tokens,
        }
    }
}

impl Cost {
    fn usd(&self) -> Option<f64> {
        self.total_cost_usd.or(self.total_usd).or(self.estimated)
    }
}

/// "Gemini 4 Neon (Medium)" → "Gemini 4 Neon" when we're appending the bold effort letter.
fn strip_effort_suffix(name: &str) -> &str {
    let Some(rest) = name.strip_suffix(')') else { return name };
    let Some((head, tail)) = rest.rsplit_once(" (") else { return name };
    if ["low", "medium", "high", "xhigh", "max"].iter().any(|l| tail.eq_ignore_ascii_case(l)) { head } else { name }
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

fn quota_window(quota: &HashMap<String, QuotaBucket>, model_id: &str) -> Option<Window> {
    if quota.is_empty() {
        return None;
    }
    let family = if model_id.to_ascii_lowercase().contains("gemini") { "gemini" } else { "3p" };
    let active = |b: &&QuotaBucket| !b.disabled && b.remaining_fraction.is_some();
    let bucket = quota
        .get(&format!("{family}-5h"))
        .filter(active)
        .or_else(|| quota.get(&format!("{family}-weekly")).filter(active))
        .or_else(|| quota.iter().find(|(k, b)| k.ends_with("-5h") && active(b)).map(|(_, b)| b))
        .or_else(|| {
            quota
                .values()
                .filter(active)
                .min_by(|a, b| a.remaining_fraction.unwrap_or(1.0).total_cmp(&b.remaining_fraction.unwrap_or(1.0)))
        })?;
    let rem = bucket.remaining_fraction?.clamp(0.0, 1.0);
    let used_percentage = (1.0 - rem) * 100.0;
    let resets_at = bucket.reset_in_seconds.filter(|&s| s > 0.0).map(|s| now() + s);
    Some(Window { used_percentage, resets_at })
}

fn limits(rl: &RateLimits, quota: &HashMap<String, QuotaBucket>, model_id: &str) -> Option<String> {
    let w = rl
        .five_hour
        .as_ref()
        .map(|w| Window { used_percentage: w.used_percentage, resets_at: w.resets_at })
        .or_else(|| quota_window(quota, model_id))
        .filter(|w| w.used_percentage > 0.0)?;
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

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(raw: &str) -> Input {
        let mut v: serde_json::Value = serde_json::from_str(raw).unwrap();
        crate::prune(&mut v);
        serde_json::from_value(v).unwrap()
    }

    #[test]
    fn effort_suffix_stripping() {
        assert_eq!(strip_effort_suffix("Gemini 4 Neon (Medium)"), "Gemini 4 Neon");
        assert_eq!(strip_effort_suffix("Gemini 4 Neon (XHigh)"), "Gemini 4 Neon");
        assert_eq!(strip_effort_suffix("Opus 4.8 (1M)"), "Opus 4.8 (1M)");
    }

    #[test]
    fn agy_quota_selects_active_family() {
        let input = parse(
            r#"{
                "product": "antigravity",
                "model": {"id": "Gemini 4 Neon (Medium)", "display_name": "Gemini 4 Neon (Medium)", "effort": "medium"},
                "quota": {
                    "3p-5h": {"remaining_fraction": 0.5, "reset_in_seconds": 3600},
                    "gemini-5h": {"remaining_fraction": 0.96996, "reset_in_seconds": 17745},
                    "gemini-weekly": {"remaining_fraction": 0.97948, "reset_in_seconds": 579442}
                }
            }"#,
        );
        let w = quota_window(&input.quota, &input.model.id).unwrap();
        assert!((w.used_percentage - 3.004).abs() < 0.01);
        assert!(w.resets_at.is_some());

        let w3p = quota_window(&input.quota, "claude-opus-4-8").unwrap();
        assert!((w3p.used_percentage - 50.0).abs() < 0.01);
    }

    #[test]
    fn agy_render_payload() {
        let input = parse(
            r#"{
                "product": "antigravity",
                "model": {"id": "Gemini 4 Neon (Medium)", "display_name": "Gemini 4 Neon (Medium)", "effort": "medium"},
                "workspace": {"current_dir": "/tmp", "project_dir": "/tmp"},
                "version": "1.5.13",
                "context_window": {
                    "context_window_size": 1048576,
                    "used_percentage": 14.24,
                    "current_usage": null
                },
                "quota": {
                    "gemini-5h": {"remaining_fraction": 0.96996, "reset_in_seconds": 17745}
                },
                "terminal_width": 120
            }"#,
        );
        let out = render(&input, &Toggles::default());
        assert!(out.contains("Gemini 4 Neon \x1b[1mM"));
        assert!(!out.contains("(Medium)"));
        assert!(!out.contains("$0.00"));
        assert!(out.contains("14.2%"));
        assert!(out.contains("3%"));
        assert!(out.contains("v1.5.13"));
    }

    #[test]
    fn agy_numeric_usage_and_cost() {
        let input = parse(
            r#"{
                "product": "antigravity",
                "model": {"id": "Gemini 4", "display_name": "Gemini 4"},
                "workspace": {"current_dir": "/tmp", "project_dir": "/tmp"},
                "context_window": {"context_window_size": 1000000, "current_usage": 12500},
                "cost": {"total_usd": 0.42}
            }"#,
        );
        let out = render(&input, &Toggles::default());
        assert!(out.contains("$0.42"));
        assert!(out.contains("12.5/1000"));
    }
}
