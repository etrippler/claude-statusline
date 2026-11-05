//! Claude Code status lines — main session line + subagent rows — in one
//! binary. `statusline --help` for commands.
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
use proc::self_path;
use serde::de::DeserializeOwned;
use serde_json::{Map, Value, json};
use settings::{Toggle, Toggles, claude_dir, expand_tilde, home};
use std::{
    env, fs, io,
    os::unix::fs::PermissionsExt,
    path::Path,
    process::{Command, ExitCode, Stdio},
    time::{SystemTime, UNIX_EPOCH},
};
use store::{REFRESH_DIFF, REFRESH_PR, read_json, write_atomic};

const RELEASES: &str = "https://github.com/etrippler/claude-statusline/releases/download/latest";

pub fn now() -> f64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0.0, |d| d.as_secs_f64())
}

// --------------------------------------------------------------------- update

/// The repo this binary was built in, when it's running from there: a clone
/// updates through git, not releases.
fn checkout() -> Option<&'static Path> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    (self_path().starts_with(root) && root.join(".git").exists()).then_some(root)
}

/// Never automatic: you run it when you want new code.
fn update() -> Result<(), String> {
    if let Some(root) = checkout() {
        let ok = |cmd: &mut Command| cmd.current_dir(root).status().is_ok_and(|s| s.success());
        return (ok(Command::new("git").args(["pull", "--ff-only"]))
            && ok(Command::new("cargo").args(["build", "--release"])))
        .then_some(())
        .ok_or_else(|| format!("update failed in {}", root.display()));
    }
    let url = format!("{RELEASES}/statusline-{}-{}", env::consts::OS, env::consts::ARCH);
    let new = Command::new("curl")
        .args(["-fsSL", "--max-time", "30", &url])
        .output()
        .ok()
        .filter(|o| o.status.success() && !o.stdout.is_empty())
        .ok_or(format!("could not fetch {url}"))?
        .stdout;
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
    let segments: Vec<_> = Toggle::ALL.map(Toggle::name).into();
    format!(
        "claude-statusline — Claude Code status lines in one binary

usage: statusline [command]     (no command: render the main line; JSON on stdin)

  subagent      render subagent task rows (JSON on stdin)
  toggles       list the segments and whether each is on
  on|off KEY    flip a segment persistently
  update        a clone: git pull + cargo build; otherwise: download the latest release
  install [SETTINGS]   point a settings.json at this binary — default ~/.claude/settings.json,
                       or a project's .claude/settings.json

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

fn install(target: Option<&str>) -> Result<(), String> {
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
        ["install", target @ ..] => install(target.first().copied())?,
        ["toggles", ..] => list_toggles(),
        [cmd @ ("on" | "off"), key, ..] => toggle(key, *cmd == "on")?,
        ["on" | "off"] => return Err("usage: statusline on|off KEY".into()),
        [REFRESH_PR, branch, owner, name, ..] => ci::refresh(branch, owner, name),
        [REFRESH_DIFF, cwd, ..] => git::refresh_diff(Path::new(cwd)),
        ["subagent", ..] => {
            for row in agents::render(&read_stdin()?) {
                println!("{row}");
            }
        }
        _ => println!("{}", line::render(&read_stdin()?, &Toggles::load())),
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
