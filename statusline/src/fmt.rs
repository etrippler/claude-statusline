//! Number and time formats shared by both status lines.

/// Compact en-US counts, as Claude Code prints them: 950, 12.3k, 2.5m.
pub fn tokens(n: f64) -> String {
    [(1e9, "b"), (1e6, "m"), (1e3, "k")]
        .into_iter()
        .find(|&(div, _)| n >= div)
        .map_or_else(|| n.to_string(), |(div, suf)| format!("{:.1}{suf}", n / div))
}

pub fn cost(usd: f64) -> String {
    if usd >= 0.01 {
        format!("${usd:.2}")
    } else if usd > 0.0 {
        format!("${usd:.4}")
    } else {
        "$0.00".into()
    }
}

/// Matches the built-in: 42s, 3m 12s, 1h 2m 3s, 1d 2h 3m.
pub fn elapsed(ms: f64) -> String {
    let secs = (ms.max(0.0) / 1000.0).round() as u64;
    let (d, h, m, s) = (secs / 86_400, secs / 3_600 % 24, secs / 60 % 60, secs % 60);
    if d > 0 {
        format!("{d}d {h}h {m}m")
    } else if h > 0 {
        format!("{h}h {m}m {s}s")
    } else if m > 0 {
        format!("{m}m {s}s")
    } else {
        format!("{s}s")
    }
}

/// Local wall-clock time, minutes dropped on the hour: 7PM, 7:30PM.
pub fn clock(epoch: f64) -> String {
    let t = epoch as libc::time_t;
    // SAFETY: localtime_r only writes the tm we hand it.
    let tm = unsafe {
        let mut tm = std::mem::zeroed::<libc::tm>();
        libc::localtime_r(&raw const t, &raw mut tm);
        tm
    };
    let hour = if tm.tm_hour % 12 == 0 { 12 } else { tm.tm_hour % 12 };
    let ampm = if tm.tm_hour < 12 { "AM" } else { "PM" };
    match tm.tm_min {
        0 => format!("{hour}{ampm}"),
        m => format!("{hour}:{m:02}{ampm}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats() {
        assert_eq!(tokens(950.0), "950");
        assert_eq!(tokens(12_345.0), "12.3k");
        assert_eq!(cost(0.004_567), "$0.0046");
        assert_eq!(cost(0.0), "$0.00");
        assert_eq!(elapsed(125_300.0), "2m 5s");
        assert_eq!(elapsed(3_725_000.0), "1h 2m 5s");
        assert_eq!(elapsed(90_061_000.0), "1d 1h 1m");
    }
}
