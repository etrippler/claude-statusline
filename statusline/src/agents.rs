//! Subagent rows. Claude Code sends every visible row as one JSON object on
//! stdin; each `{"id", "content"}` line printed overrides that row, and rows we
//! skip keep the built-in rendering.

use crate::{
    ansi::{dim, green, justify, red, width, yellow},
    fmt::{elapsed, tokens},
    model::{self, Effort},
    now,
    settings::Settings,
    store::Session,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{env, fmt::Write, path::Path};

#[derive(Deserialize, Default)]
#[serde(default)]
pub struct Input {
    session_id: Option<String>,
    cwd: Option<String>,
    columns: Option<usize>,
    tasks: Vec<Task>,
}

#[derive(Deserialize, Default)]
#[serde(default, rename_all = "camelCase")]
struct Task {
    id: Value,
    model: Option<String>,
    effort: Option<Effort>,
    agent_type: Option<String>,
    name: Option<String>,
    cwd: Option<String>,
    status: Option<String>,
    start_time: Option<f64>,
    token_count: Option<f64>,
    label: Option<String>,
    description: Option<String>,
}

#[derive(Serialize)]
struct Row<'a> {
    id: &'a Value,
    content: String,
}

pub fn render(input: &Input) -> Vec<String> {
    let cwd = input.cwd.as_deref().unwrap_or_default();
    let project = env::var("CLAUDE_PROJECT_DIR").ok().filter(|p| !p.is_empty()).unwrap_or_else(|| cwd.into());
    let session = input.session_id.as_deref().map(Session::load).unwrap_or_default();
    let mut settings = None; // only read if some row inherits its effort
    let cols = input.columns.filter(|&c| c > 0); // 0: width unknown
    let now_ms = now() * 1000.0;
    input
        .tasks
        .iter()
        .filter_map(|t| {
            let model = t.model.as_deref()?; // unresolved → keep the default row
            // Inherited effort: the live session's when it's the session's model.
            let effort = t
                .effort
                .clone()
                .or_else(|| session.effort.clone().filter(|_| model::same(model, &session.model)))
                .or_else(|| settings.get_or_insert_with(|| Settings::load(Path::new(&project))).effort(model));
            let content = row(t, model, effort.as_ref(), &session.model, cwd, cols, now_ms);
            serde_json::to_string(&Row { id: &t.id, content }).ok()
        })
        .collect()
}

/// "agent (type) ⎇ wt F5.1 hi · ✓ label        3m 12s · ↓ 12.3k tokens"
fn row(
    t: &Task,
    model: &str,
    effort: Option<&Effort>,
    session_model: &str,
    cwd: &str,
    cols: Option<usize>,
    now_ms: f64,
) -> String {
    const SEP: &str = " · ";
    // Agent first, then a terse spec: the model only when it isn't the session's.
    let name = t.name.as_deref().unwrap_or_default();
    let typ = t.agent_type.as_deref().filter(|&t| t != "general-purpose").unwrap_or_default();
    let mut head = match (name, typ) {
        ("", "") => "agent".to_owned(),
        (n, "") => n.to_owned(),
        ("", t) => t.to_owned(),
        (n, t) if n.eq_ignore_ascii_case(t) => n.to_owned(),
        (n, t) => format!("{n} ({t})"),
    };
    if let Some(dir) = t.cwd.as_deref().filter(|d| !cwd.is_empty() && *d != cwd) {
        // isolated worktree
        let base = Path::new(dir).file_name().map_or(dir.into(), |n| n.to_string_lossy());
        let _ = write!(head, " \u{2387} {base}");
    }
    let spec = [(!model::same(model, session_model)).then(|| model::short(model)), effort.map(Effort::short)];
    for s in spec.into_iter().flatten() {
        let _ = write!(head, " {s}");
    }

    let status = t.status.as_deref().unwrap_or("running");
    let mark = match status {
        "completed" => Some(green("✓")),
        "failed" => Some(red("✗")),
        "killed" => Some(yellow("✗")),
        _ => None,
    };
    // No end time in the payload: a finished row's elapsed would keep counting.
    let elapsed = t.start_time.filter(|_| status == "running").map(|s| elapsed(now_ms - s));
    let count = t.token_count.filter(|&n| n > 0.0).map(|n| format!("↓ {} tokens", tokens(n)));
    let right = [elapsed, count].into_iter().flatten().collect::<Vec<_>>().join(SEP);

    let label = t.label.as_deref().or(t.description.as_deref()).unwrap_or_default();
    let mark_w = if mark.is_some() { 2 } else { 0 };
    let room = cols.map_or(usize::MAX, |c| c.saturating_sub(width(&head) + width(SEP) + mark_w + width(&right) + 2));
    let mut out = dim(&head);
    if !label.is_empty() && room > 8 {
        let label: String =
            if width(label) > room { label.chars().take(room - 1).chain(['…']).collect() } else { label.to_owned() };
        out += &dim(SEP);
        if let Some(m) = &mark {
            let _ = write!(out, "{m} ");
        }
        out += &dim(label);
    } else if let Some(m) = &mark {
        let _ = write!(out, " {m}");
    }
    if right.is_empty() { out } else { justify(&out, &dim(&right), cols, 0, &dim(SEP)) }
}
