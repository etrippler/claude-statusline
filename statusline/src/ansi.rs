//! SGR colors, OSC 8 hyperlinks, and the width a terminal actually renders.

use std::fmt::Display;

macro_rules! colors {
    ($($name:ident = $code:literal),* $(,)?) => {
        $(pub fn $name(s: impl Display) -> String { format!("\x1b[{}m{s}\x1b[0m", $code) })*
    };
}

colors!(dim = "2", red = "31", green = "32", yellow = "33", blue = "34", magenta = "35", cyan = "36", white = "37");
colors!(bold_cyan = "1;36");

pub const BOLD: &str = "\x1b[1m";

pub fn link(url: Option<&str>, s: String) -> String {
    match url {
        Some(url) => format!("\x1b]8;;{url}\x07{s}\x1b]8;;\x07"),
        None => s,
    }
}

/// Visible width: SGR sequences and OSC 8 wrappers take no cells.
pub fn width(s: &str) -> usize {
    let mut rest = s;
    let mut n = 0;
    while let Some(c) = rest.chars().next() {
        if let Some(r) = rest.strip_prefix("\x1b[") {
            let params = r.find(|c: char| !c.is_ascii_digit() && c != ';').unwrap_or(r.len());
            if r[params..].starts_with('m') {
                rest = &r[params + 1..];
                continue;
            }
        } else if let Some(r) = rest.strip_prefix("\x1b]8;;")
            && let Some(end) = r.find('\x07')
        {
            rest = &r[end + 1..];
            continue;
        }
        n += 1;
        rest = &rest[c.len_utf8()..];
    }
    n
}

/// `right` flush against the right edge, `margin` cells short of `cols`; just
/// `left{sep}right` when the width is unknown or there's no room.
pub fn justify(left: &str, right: &str, cols: Option<usize>, margin: usize, sep: &str) -> String {
    let pad = cols.and_then(|c| c.checked_sub(margin + width(left) + width(right))).filter(|&p| p > 0);
    match pad {
        Some(p) => format!("{left}{:p$}{right}", ""),
        None => format!("{left}{sep}{right}"),
    }
}
