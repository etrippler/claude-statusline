//! Number and time formats.

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
    }
}
