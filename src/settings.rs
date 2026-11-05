//! Our own toggles, and the slice of Claude Code's settings cascade we read.

use crate::{
    model::{self, Effort},
    store::{read_json, write_json},
};
use serde::Deserialize;
use serde_json::{Map, Value};
use std::{
    env, fs, io,
    path::{Path, PathBuf},
};

pub fn home() -> PathBuf {
    env::var_os("HOME").map_or_else(|| "/".into(), PathBuf::from)
}

pub fn claude_dir() -> PathBuf {
    home().join(".claude")
}

pub fn expand_tilde(p: &str) -> PathBuf {
    match p.strip_prefix("~") {
        Some(rest) if rest.is_empty() || rest.starts_with('/') => home().join(rest.trim_start_matches('/')),
        _ => p.into(),
    }
}

#[derive(Clone, Copy, PartialEq)]
pub enum Toggle {
    Repo,
    Branch,
    Diff,
    Ci,
    Model,
    Context,
    Percent,
    Cost,
    Cache,
    Limits,
    Version,
    /// Gates the hourly self-update rather than a segment.
    Update,
}

impl Toggle {
    pub const ALL: [Toggle; 12] = {
        use Toggle::*;
        [Repo, Branch, Diff, Ci, Model, Context, Percent, Cost, Cache, Limits, Version, Update]
    };
    const NAMES: [&str; 12] =
        ["repo", "branch", "diff", "ci", "model", "context", "percent", "cost", "cache", "limits", "version", "update"];

    pub fn name(self) -> &'static str {
        Self::NAMES[self as usize]
    }

    pub fn parse(s: &str) -> Option<Toggle> {
        Self::NAMES.iter().position(|&n| n == s).map(|i| Self::ALL[i])
    }

    pub fn segments() -> impl Iterator<Item = Toggle> {
        Self::ALL.into_iter().filter(|&t| t != Toggle::Update)
    }
}

/// Every toggle defaults on; the file stores only the ones switched off.
pub struct Toggles(Map<String, Value>);

impl Toggles {
    pub fn path() -> PathBuf {
        env::var("CLAUDE_STATUSLINE_CONFIG")
            .ok()
            .filter(|s| !s.is_empty())
            .map_or_else(|| claude_dir().join("statusline.json"), |p| expand_tilde(&p))
    }

    pub fn load() -> Self {
        Toggles(read_json(&Self::path()))
    }

    pub fn on(&self, t: Toggle) -> bool {
        self.0.get(t.name()).and_then(Value::as_bool).unwrap_or(true)
    }

    pub fn overridden(&self, t: Toggle) -> bool {
        self.0.contains_key(t.name())
    }

    pub fn set(t: Toggle, on: bool) -> io::Result<()> {
        let mut map = Self::load().0;
        map.shift_remove(t.name());
        if !on {
            map.insert(t.name().into(), false.into());
        }
        if !map.is_empty() {
            return write_json(&Self::path(), &map);
        }
        match fs::remove_file(Self::path()) {
            Err(e) if e.kind() != io::ErrorKind::NotFound => Err(e),
            _ => Ok(()),
        }
    }
}

/// user < project < local, merged per key; `modelSettings[model]` beats the plain key.
#[derive(Default)]
pub struct Settings {
    merged: Map<String, Value>,
    models: Map<String, Value>,
}

impl Settings {
    pub fn load(project: &Path) -> Self {
        let mut s = Settings::default();
        for p in [
            claude_dir().join("settings.json"),
            project.join(".claude/settings.json"),
            project.join(".claude/settings.local.json"),
        ] {
            let mut file: Map<String, Value> = read_json(&p);
            if let Some(Value::Object(m)) = file.remove("modelSettings") {
                s.models.extend(m);
            }
            s.merged.extend(file);
        }
        s
    }

    fn get(&self, key: &str, model: &str) -> Option<&Value> {
        let model = model::base(model);
        let set = |v: &&Value| !v.is_null() && v.as_str() != Some("");
        // Longest key wins: "claude-opus-5" must not shadow "claude-opus-5-5".
        self.models
            .iter()
            .filter(|(k, _)| model.strip_prefix(k.as_str()).is_some_and(|r| r.is_empty() || r.starts_with('-')))
            .filter_map(|(k, v)| Some((k, v.get(key).filter(set)?)))
            .max_by_key(|(k, _)| k.len())
            .map(|(_, v)| v)
            .or_else(|| self.merged.get(key).filter(set))
    }

    pub fn effort(&self, model: &str) -> Option<Effort> {
        self.get("effortLevel", model).and_then(|v| Effort::deserialize(v).ok())
    }

    /// Token count where autocompact kicks in, or None when it's off or sits at the window.
    pub fn autocompact_at(&self, model: &str, window: f64) -> Option<f64> {
        // /config writes the global switch to ~/.claude.json
        let enabled = self.get("autoCompactEnabled", model).map_or_else(claude_json_autocompact, Value::as_bool);
        if enabled == Some(false) {
            return None;
        }
        let at = self.get("autoCompactWindow", model).and_then(Value::as_f64).filter(|&n| n > 0.0).unwrap_or(window);
        let at = env::var("CLAUDE_AUTOCOMPACT_PCT_OVERRIDE")
            .ok()
            .and_then(|p| p.parse::<f64>().ok())
            .map_or(at, |pct| at.min(window * pct / 100.0));
        (at > 0.0 && at < window).then_some(at)
    }
}

/// ~/.claude.json is Claude Code's whole state file; deserialize the one key and skip the rest.
fn claude_json_autocompact() -> Option<bool> {
    #[derive(Deserialize, Default)]
    struct ClaudeJson {
        #[serde(rename = "autoCompactEnabled")]
        auto_compact_enabled: Option<bool>,
    }
    read_json::<ClaudeJson>(&home().join(".claude.json")).auto_compact_enabled
}
