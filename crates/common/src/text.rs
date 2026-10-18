//! Text helpers the daemon's reports and the client both print (decision 10). Pure.

/// Moved to [`crate::numbers`] (milestone 9.19); the one `tokens` and the one
/// `budget_text` (`40 calls · 20 min · 200k tok`).
pub use crate::numbers::{budget_text, tokens};

/// `YYYY-MM-DD HH:MM:SSZ` in UTC, days-from-civil arithmetic (Howard Hinnant's
/// algorithm), no new dependency.
pub fn format_utc(unix: u64) -> String {
    let days = (unix / 86_400) as i64;
    let secs = unix % 86_400;
    let (y, m, d) = crate::numbers::civil_from_days(days);
    let (h, mi, s) = (secs / 3600, (secs % 3600) / 60, secs % 60);
    format!("{y:04}-{m:02}-{d:02} {h:02}:{mi:02}:{s:02}Z")
}

#[cfg(test)]
mod tests {
    #[test]
    fn format_utc_is_the_long_utc_stamp() {
        assert_eq!(super::format_utc(0), "1970-01-01 00:00:00Z");
        assert_eq!(super::format_utc(1_791_814_079), "2026-10-12 14:07:59Z");
        assert_eq!(super::format_utc(1_792_247_400), "2026-10-17 14:30:00Z");
    }
}
