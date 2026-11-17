//! Model ids and effort levels, in the terse forms the status line prints.

use serde::Deserialize;

/// "claude-opus-4-8[1m]" → "claude-opus-4-8"
pub fn base(id: &str) -> &str {
    id.strip_suffix("m]")
        .map(|r| r.trim_end_matches(|c: char| c.is_ascii_digit()))
        .filter(|r| r.len() + 2 < id.len())
        .and_then(|r| r.strip_suffix('['))
        .unwrap_or(id)
}

/// low|medium|high|xhigh|max, or a thinking-token budget.
#[derive(Deserialize)]
#[serde(untagged)]
pub enum Effort {
    Budget(f64),
    Level(String),
}

impl Effort {
    /// L M H XH MAX
    pub fn letter(&self) -> String {
        match self {
            Effort::Budget(n) => format!("{}k", (n / 1000.0).round()),
            Effort::Level(l) => ["low", "medium", "high", "xhigh", "max"]
                .iter()
                .position(|x| x == l)
                .map_or_else(|| l.clone(), |i| ["L", "M", "H", "XH", "MAX"][i].into()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids() {
        assert_eq!(base("x[1m]"), "x");
        assert_eq!(base("x[m]"), "x[m]");
    }
}
