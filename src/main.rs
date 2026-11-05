//! Claude Code status lines — main session line + subagent rows — in one
//! self-updating binary. `statusline --help` for commands.
//!
//! The branch glyph is Powerline (U+E0A0): use a Powerline-capable font.

mod agents;
mod ansi;
mod ci;
mod fmt;
mod git;
mod line;
mod model;
mod proc;
mod settings;
mod store;

use ansi::{dim, green, red};
use proc::{self_path, spawn_detached};
use serde::de::DeserializeOwned;
use serde_json::{Map, Value, json};
use settings::{Toggle, Toggles, claude_dir, expand_tilde, home};
use std::{
    env, fs, io,
    os::unix::fs::PermissionsExt,
    path::Path,
    process::{Command, ExitCode, Stdio},
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use store::{REFRESH_DIFF, REFRESH_PR, read_json, write_atomic};

const RELEASES: &str = "https://github.com/etrippler/claude-statusline/releases/download/latest";
const UPDATE_STAMP: &str = "/tmp/claude-statusline-rs-update-stamp";

pub fn now() -> f64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0.0, |d| d.as_secs_f64())
}

// ---------------------------------------------------------------- self-update

/// A binary built inside this repo is a dev copy: git is authoritative.
fn in_checkout() -> bool {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    self_path().starts_with(root) && root.join(".git").exists()
}

/// Rides along on every render: at most hourly, a detached `update`.
fn maybe_update() {
    let fresh = fs::metadata(UPDATE_STAMP)
        .and_then(|m| m.modified())
        .is_ok_and(|t| t.elapsed().is_ok_and(|age| age < Duration::from_secs(3600)));
    if fresh || in_checkout() || !Toggles::load().on(Toggle::Update) {
        return;
    }
    touch(UPDATE_STAMP); // before spawning, so concurrent sessions stay single-flight
    spawn_detached(["update"]);
}

fn touch(p: &str) {
    let _ = fs::OpenOptions::new().create(true).append(true).open(p).and_then(|f| f.set_modified(SystemTime::now()));
}

fn update() -> Result<(), String> {
    if in_checkout() {
        return Err("this copy lives in a git checkout — use git pull && cargo build --release".into());
    }
    let url = format!("{RELEASES}/statusline-{}-{}", env::consts::OS, env::consts::ARCH);
    let new = Command::new("curl")
        .args(["-fsSL", "--max-time", "30", &url])
        .output()
        .ok()
        .filter(|o| o.status.success() && !o.stdout.is_empty())
        .ok_or(format!("could not fetch {url}"))?
        .stdout;
    touch(UPDATE_STAMP);
    let me = self_path();
    if fs::read(&me).is_ok_and(|cur| cur == new) {
        println!("already up to date");
        return Ok(());
    }
    let tmp = me.with_extension("new");
    fs::write(&tmp, &new)
        .and_then(|()| fs::set_permissions(&tmp, fs::Permissions::from_mode(0o755)))
        .map_err(|e| format!("{}: {e}", tmp.display()))?;
    // A binary that can't run on this machine never gets installed.
    if !Command::new(&tmp).arg("--help").stdout(Stdio::null()).status().is_ok_and(|s| s.success()) {
        let _ = fs::remove_file(&tmp);
        return Err(format!("{url} does not run here; keeping the current binary"));
    }
    // Atomic: in-flight renders keep running the old inode.
    fs::rename(&tmp, &me).map_err(|e| format!("{}: {e}", me.display()))?;
    println!("updated {}", me.display());
    Ok(())
}

// ------------------------------------------------------------------- commands

fn usage() -> String {
    let segments: Vec<_> = Toggle::segments().map(Toggle::name).collect();
    format!(
        "claude-statusline — Claude Code status lines in one self-updating binary

usage: statusline [command]     (no command: render the main line; JSON on stdin)

  subagent      render subagent task rows (JSON on stdin)
  toggles       list toggles: the segments, plus 'update' (the hourly background check)
  on|off KEY    flip a toggle persistently
  update        fetch the latest release now (works even with 'update' toggled off)
  install [SETTINGS]   point a settings.json at this binary — default ~/.claude/settings.json,
                       or a project's .claude/settings.json (--no-update: also 'off update')

segments: {}
config:   {} — set $CLAUDE_STATUSLINE_CONFIG to relocate
",
        segments.join(" "),
        Toggles::path().display()
    )
}

fn toggle(key: &str, on: bool) -> Result<(), String> {
    let t = Toggle::parse(key).ok_or(format!("unknown toggle {key:?} — see: statusline toggles"))?;
    Toggles::set(t, on).map_err(|e| format!("{}: {e}", Toggles::path().display()))?;
    println!("{key}: {}", if on { "on" } else { "off" });
    Ok(())
}

fn list_toggles() {
    let toggles = Toggles::load();
    for t in Toggle::ALL {
        let state = if toggles.on(t) { green("on ") } else { red("off") };
        let note = if toggles.overridden(t) { dim("  (override)") } else { String::new() };
        println!("  {state} {}{note}", t.name());
    }
}

fn install(target: Option<&str>, no_update: bool) -> Result<(), String> {
    let path = target.map_or_else(|| claude_dir().join("settings.json"), expand_tilde);
    let fail = |e: io::Error| format!("{}: {e}", path.display());
    if let Some(dir) = path.parent().filter(|d| !d.as_os_str().is_empty()) {
        fs::create_dir_all(dir).map_err(fail)?;
    }
    let me = self_path();
    let cmd = me.strip_prefix(home()).map_or_else(|_| me.display().to_string(), |rel| format!("~/{}", rel.display()));
    let mut settings: Map<String, Value> = read_json(&path);
    settings
        .insert("statusLine".into(), json!({"type": "command", "command": cmd, "padding": 0, "refreshInterval": 1}));
    settings.insert("subagentStatusLine".into(), json!({"type": "command", "command": format!("{cmd} subagent")}));
    let body = serde_json::to_string_pretty(&settings).map_err(|e| e.to_string())? + "\n";
    write_atomic(&path, &body).map_err(fail)?;
    println!("{} statusLine + subagentStatusLine → {cmd}", path.display());
    if in_checkout() {
        println!("(running from a git checkout: auto-update stays off; rebuild with cargo)");
    }
    if no_update {
        toggle("update", false)?;
    }
    Ok(())
}

/// Nulls and empty strings mean "not set" everywhere in Claude Code's JSON;
/// dropping them up front lets `#[serde(default)]` cover every field.
fn prune(v: &mut Value) {
    match v {
        Value::Object(m) => {
            m.retain(|_, v| !v.is_null() && v.as_str() != Some(""));
            m.values_mut().for_each(prune);
        }
        Value::Array(a) => a.iter_mut().for_each(prune),
        _ => {}
    }
}

fn read_stdin<T: DeserializeOwned>() -> Result<T, String> {
    let mut v: Value = serde_json::from_reader(io::stdin().lock()).map_err(|e| format!("stdin: {e}"))?;
    prune(&mut v);
    serde_json::from_value(v).map_err(|e| format!("stdin: {e}"))
}

fn run(args: &[&str]) -> Result<(), String> {
    match args {
        ["-h" | "--help" | "help", ..] => print!("{}", usage()),
        ["update", ..] => update()?,
        ["install", rest @ ..] => {
            let target = rest.iter().find(|a| **a != "--no-update").copied();
            install(target, rest.contains(&"--no-update"))?;
        }
        ["toggles", ..] => list_toggles(),
        [cmd @ ("on" | "off"), key, ..] => toggle(key, *cmd == "on")?,
        ["on" | "off"] => return Err("usage: statusline on|off KEY".into()),
        [REFRESH_PR, branch, owner, name, ..] => ci::refresh(branch, owner, name),
        [REFRESH_DIFF, cwd, ..] => git::refresh_diff(Path::new(cwd)),
        ["subagent", ..] => {
            maybe_update();
            for row in agents::render(&read_stdin()?) {
                println!("{row}");
            }
        }
        _ => {
            maybe_update();
            println!("{}", line::render(&read_stdin()?, &Toggles::load()));
        }
    }
    Ok(())
}

fn main() -> ExitCode {
    let args: Vec<String> = env::args().skip(1).collect();
    let args: Vec<&str> = args.iter().map(String::as_str).collect();
    match run(&args) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("{e}");
            ExitCode::FAILURE
        }
    }
}
