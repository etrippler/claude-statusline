//! Model ids and effort levels, in the terse forms the status lines print.

use serde::{Deserialize, Serialize};

/// "claude-opus-4-8[1m]" → "claude-opus-4-8"
pub fn base(id: &str) -> &str {
    id.strip_suffix("m]")
        .map(|r| r.trim_end_matches(|c: char| c.is_ascii_digit()))
        .filter(|r| r.len() + 2 < id.len())
        .and_then(|r| r.strip_suffix('['))
        .unwrap_or(id)
}

pub fn same(a: &str, b: &str) -> bool {
    !a.is_empty() && base(a) == base(b)
}

fn capitalize(word: &str) -> String {
    let mut cs = word.chars();
    cs.next().map(|c| c.to_uppercase().chain(cs.flat_map(char::to_lowercase)).collect()).unwrap_or_default()
}

/// "claude-fable-5-1[1m]" → "F5.1"; "claude-custom-thing" → "Custom Thing"
pub fn short(id: &str) -> String {
    let base = base(id);
    let toks = base.strip_prefix("claude-").unwrap_or(base).split('-').filter(|t| !t.is_empty());
    let is_num = |t: &&str| t.bytes().all(|b| b.is_ascii_digit());
    let ver: Vec<&str> = toks.clone().filter(is_num).filter(|t| t.len() != 8).collect(); // 8 digits: a date stamp
    let mut words = toks.filter(|t| t.bytes().any(|b| b.is_ascii_alphabetic()));
    match words.next() {
        Some(first) if !ver.is_empty() => capitalize(first).chars().take(1).chain(ver.join(".").chars()).collect(),
        first => first.into_iter().chain(words).map(capitalize).collect::<Vec<_>>().join(" "),
    }
}

/// low|medium|high|xhigh|max, or a thinking-token budget.
#[derive(Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Effort {
    Budget(f64),
    Level(String),
}

impl Effort {
    fn label(&self, names: [&str; 5]) -> String {
        match self {
            Effort::Budget(n) => format!("{}k", (n / 1000.0).round()),
            Effort::Level(l) => ["low", "medium", "high", "xhigh", "max"]
                .iter()
                .position(|x| x == l)
                .map_or_else(|| l.clone(), |i| names[i].into()),
        }
    }

    /// Main line: L M H XH MAX
    pub fn letter(&self) -> String {
        self.label(["L", "M", "H", "XH", "MAX"])
    }

    /// Subagent rows: lo med hi xh max
    pub fn short(&self) -> String {
        self.label(["lo", "med", "hi", "xh", "max"])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids() {
        assert_eq!(short("claude-fable-5-1[1m]"), "F5.1");
        assert_eq!(short("claude-opus-4-1-20250805"), "O4.1");
        assert_eq!(short("claude-3-5-sonnet-20241022"), "S3.5");
        assert_eq!(short("custom-thing"), "Custom Thing");
        assert_eq!(base("x[1m]"), "x");
        assert_eq!(base("x[m]"), "x[m]");
        assert!(same("claude-opus-5-5[1m]", "claude-opus-5-5"));
        assert!(!same("", ""));
    }
}
