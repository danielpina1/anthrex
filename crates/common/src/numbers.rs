//! Every number anthrex shows, formatted once (milestone 9.19, decisions 1-4, 9 and 10).
//! Pure: integers and proto types in, `String` out; nothing here reads a clock, an
//! environment variable or a file. The viewer's zone offset is passed in by the caller.

use proto::Budget;

/// A value anthrex does not have, in a labelled field. A table cell prints `-`. Never `0`.
pub const UNKNOWN: &str = "unknown";

const MINUTE: u64 = 60;
const HOUR: u64 = 3_600;
const DAY: u64 = 86_400;

/// Tokens as `<n>`, `<n>.<d>k` under 10k, `<n>k`, or `<n>.<d>M`, rounded down.
pub fn tokens(n: u64) -> String {
    match n {
        0..1_000 => n.to_string(),
        1_000..10_000 => format!("{}.{}k", n / 1_000, n % 1_000 / 100),
        10_000..1_000_000 => format!("{}k", n / 1_000),
        _ => format!("{}.{}M", n / 1_000_000, n % 1_000_000 / 100_000),
    }
}

/// `<calls> calls · <minutes> min`, then ` · <tokens> tok` when a token ceiling is set.
pub fn budget_text(b: &Budget) -> String {
    let mut text = format!("{} calls · {} min", b.tool_calls, b.minutes);
    if let Some(ceiling) = b.tokens {
        text.push_str(&format!(" · {} tok", tokens(ceiling)));
    }
    text
}

/// A span in two units, the second zero-padded, floored: `42s`, `41m 12s`, `1h 05m`,
/// `3d 04h`.
pub fn duration(secs: u64) -> String {
    if secs < MINUTE {
        format!("{secs}s")
    } else if secs < HOUR {
        format!("{}m {:02}s", secs / MINUTE, secs % MINUTE)
    } else if secs < DAY {
        format!("{}h {:02}m", secs / HOUR, secs % HOUR / MINUTE)
    } else {
        format!("{}d {:02}h", secs / DAY, secs % DAY / HOUR)
    }
}

/// The largest unit only, floored, for fixed-width cells: `42s`, `41m`, `1h`, `3d`.
pub fn duration_coarse(secs: u64) -> String {
    if secs < MINUTE {
        format!("{secs}s")
    } else if secs < HOUR {
        format!("{}m", secs / MINUTE)
    } else if secs < DAY {
        format!("{}h", secs / HOUR)
    } else {
        format!("{}d", secs / DAY)
    }
}

/// A span in milliseconds: tenths of a second under 10 s (`4.2s`), else [`duration`];
/// `u32::MAX`, the daemon's saturated value, is [`UNKNOWN`].
pub fn duration_ms(ms: u32) -> String {
    if ms == u32::MAX {
        UNKNOWN.to_string()
    } else if ms < 10_000 {
        format!("{}.{}s", ms / 1_000, ms % 1_000 / 100)
    } else {
        duration(u64::from(ms / 1_000))
    }
}

/// How long since: `just now` under 10 s, else `<duration> ago`.
pub fn ago(secs: u64) -> String {
    if secs < 10 {
        "just now".to_string()
    } else {
        format!("{} ago", duration(secs))
    }
}

/// The zone a clock time is printed in. `Utc` prints `UTC`; `Local(offset_secs)` always
/// prints `+HH:MM` / `-HH:MM`, so `Local(0)` prints `+00:00`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Zone {
    Utc,
    Local(i32),
}

impl Zone {
    fn offset(self) -> i64 {
        match self {
            Zone::Utc => 0,
            Zone::Local(offset) => i64::from(offset),
        }
    }

    fn label(self) -> String {
        match self {
            Zone::Utc => "UTC".to_string(),
            Zone::Local(offset) => {
                let sign = if offset < 0 { '-' } else { '+' };
                let abs = offset.unsigned_abs();
                format!("{sign}{:02}:{:02}", abs / 3600, abs % 3600 / 60)
            }
        }
    }
}

/// A Unix time in `zone`: whole days since the epoch and seconds into that day.
fn local_parts(at: u64, zone: Zone) -> (i64, u64) {
    let shifted = i64::try_from(at)
        .unwrap_or(i64::MAX)
        .saturating_add(zone.offset());
    (
        shifted.div_euclid(DAY as i64),
        shifted.rem_euclid(DAY as i64) as u64,
    )
}

/// A clock time relative to `now`, always labelled with its zone: `14:30 +02:00` on
/// `now`'s day (in `zone`), `Thu 14:30 +02:00` within six days either side, else
/// `2026-10-12 14:30 +02:00`.
pub fn clock(at: u64, now: u64, zone: Zone) -> String {
    let (day, secs) = local_parts(at, zone);
    let (today, _) = local_parts(now, zone);
    let (h, m) = (secs / 3600, secs % 3600 / 60);
    let label = zone.label();
    match day.saturating_sub(today).unsigned_abs() {
        0 => format!("{h:02}:{m:02} {label}"),
        1..=6 => format!("{} {h:02}:{m:02} {label}", weekday(day)),
        _ => full(day, secs, &label),
    }
}

/// `2026-10-12 14:30 +02:00`: the long form of [`clock`], whatever `now` is.
pub fn clock_full(at: u64, zone: Zone) -> String {
    let (day, secs) = local_parts(at, zone);
    full(day, secs, &zone.label())
}

fn full(day: i64, secs: u64, label: &str) -> String {
    let (y, mo, d) = civil_from_days(day);
    format!(
        "{y:04}-{mo:02}-{d:02} {:02}:{:02} {label}",
        secs / 3600,
        secs % 3600 / 60
    )
}

/// The weekday of a day count since 1970-01-01, a Thursday.
fn weekday(day: i64) -> &'static str {
    const NAMES: [&str; 7] = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];
    NAMES[(day + 4).rem_euclid(7) as usize]
}

/// Howard Hinnant's days-from-civil algorithm, inverted; pure integer arithmetic.
pub(crate) fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as u64;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = (if mp < 10 { mp + 3 } else { mp - 9 }) as u32;
    (if m <= 2 { y + 1 } else { y }, m, d)
}

/// How full a context window is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Fill {
    /// 0 to 100, floored.
    Percent(u8),
    /// Past the window.
    Over,
    /// No window, or a zero one.
    NoWindow,
}

/// `tokens` against `window`: never a percentage past 100.
pub fn fill(tokens: u64, window: u64) -> Fill {
    if window == 0 {
        Fill::NoWindow
    } else if tokens > window {
        Fill::Over
    } else {
        Fill::Percent((u128::from(tokens) * 100 / u128::from(window)) as u8)
    }
}

/// `context 91% full (183k / 200k)`, `context over 100% (450k / 200k window)`, or
/// `context 183k (window unknown)`.
pub fn context_text(used: u64, window: Option<u64>) -> String {
    let window = window.unwrap_or(0);
    match fill(used, window) {
        Fill::Percent(p) => format!("context {p}% full ({} / {})", tokens(used), tokens(window)),
        Fill::Over => format!(
            "context over 100% ({} / {} window)",
            tokens(used),
            tokens(window)
        ),
        Fill::NoWindow => format!("context {} (window unknown)", tokens(used)),
    }
}

/// Busy slots over allowed: `2/3`, or `4/3 (over)` after a limit was lowered mid-run.
pub fn busy(busy: u32, max: u32) -> String {
    if busy > max {
        format!("{busy}/{max} (over)")
    } else {
        format!("{busy}/{max}")
    }
}

/// `1 task`, `2 tasks`.
pub fn count(n: u64, one: &str, many: &str) -> String {
    format!("{n} {}", if n == 1 { one } else { many })
}

#[cfg(test)]
#[path = "numbers_tests.rs"]
mod tests;
