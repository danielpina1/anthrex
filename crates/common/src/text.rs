//! Text helpers the daemon's reports and the client both print (decision 10). Pure.
//! `budget_text` keeps its current output until milestone 9.19.6 switches it to
//! `numbers::budget_text`.

use proto::Budget;

/// Moved to [`crate::numbers`] (milestone 9.19); the one `tokens`.
pub use crate::numbers::tokens;

/// `<calls> calls <minutes>m`, then ` <tokens> tok` (M8b's `k`/`M` notation) when set.
pub fn budget_text(b: &Budget) -> String {
    let mut text = format!("{} calls {}m", b.tool_calls, b.minutes);
    if let Some(tokens) = b.tokens {
        text.push_str(&format!(" {} tok", self::tokens(tokens)));
    }
    text
}

/// `YYYY-MM-DD HH:MM:SSZ` in UTC, days-from-civil arithmetic (Howard Hinnant's
/// algorithm), no new dependency.
pub fn format_utc(unix: u64) -> String {
    let days = (unix / 86_400) as i64;
    let secs = unix % 86_400;
    let (y, m, d) = civil_from_days(days);
    let (h, mi, s) = (secs / 3600, (secs % 3600) / 60, secs % 60);
    format!("{y:04}-{m:02}-{d:02} {h:02}:{mi:02}:{s:02}Z")
}

/// `HH:MM` of a Unix time, in UTC: the one clock the daemon's prompts and ask context,
/// `anthrex run status` and the run inspector print for one instant (milestone 9.13
/// ruling C17).
pub fn hh_mm(at: u64) -> String {
    format!("{:02}:{:02}", at % 86_400 / 3600, at % 3600 / 60)
}

fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as u64;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    let y = if m <= 2 { y + 1 } else { y };
    (y, m, d)
}

#[cfg(test)]
mod tests {
    #[test]
    fn hh_mm_is_utc_hours_and_minutes() {
        assert_eq!(super::hh_mm(0), "00:00");
        // 2026-10-12 14:07:59Z.
        assert_eq!(super::hh_mm(1_791_814_079), "14:07");
        assert_eq!(super::hh_mm(86_399), "23:59");
        assert_eq!(super::hh_mm(86_400 + 61), "00:01");
    }
}
