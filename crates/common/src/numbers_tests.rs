use super::*;
use proto::Budget;

/// 2026-10-17 14:30:00 UTC, a Saturday.
const NOW: u64 = 1_792_247_400;
const DAY: u64 = 86_400;
const UTC: Zone = Zone::Utc;

#[test]
fn tokens_bands() {
    for (n, want) in [
        (0, "0"),
        (999, "999"),
        (1_000, "1.0k"),
        (4_500, "4.5k"),
        (9_999, "9.9k"),
        (10_000, "10k"),
        (45_000, "45k"),
        (999_999, "999k"),
        (1_000_000, "1.0M"),
        (1_250_000, "1.2M"),
    ] {
        assert_eq!(tokens(n), want, "tokens({n})");
    }
    assert_eq!(tokens(u64::MAX), "18446744073709.5M");
}

#[test]
fn duration_formats() {
    for (secs, want) in [
        (0, "0s"),
        (59, "59s"),
        (60, "1m 00s"),
        (2472, "41m 12s"),
        (3599, "59m 59s"),
        (3600, "1h 00m"),
        (3900, "1h 05m"),
        (7199, "1h 59m"),
        (86_399, "23h 59m"),
        (86_400, "1d 00h"),
        (273_600, "3d 04h"),
    ] {
        assert_eq!(duration(secs), want, "duration({secs})");
    }
    assert_eq!(duration(u64::MAX), "213503982334601d 07h");
}

#[test]
fn duration_coarse_floors_to_one_unit() {
    for (secs, want) in [
        (0, "0s"),
        (42, "42s"),
        (119, "1m"),
        (2472, "41m"),
        (7199, "1h"),
        (86_400, "1d"),
        (259_200, "3d"),
    ] {
        assert_eq!(duration_coarse(secs), want, "duration_coarse({secs})");
    }
    assert_eq!(duration_coarse(u64::MAX), "213503982334601d");
}

#[test]
fn duration_ms_cases() {
    for (ms, want) in [
        (0, "0.0s"),
        (4_250, "4.2s"),
        (9_999, "9.9s"),
        (10_000, "10s"),
        (1_200_000, "20m 00s"),
        (u32::MAX, "unknown"),
    ] {
        assert_eq!(duration_ms(ms), want, "duration_ms({ms})");
    }
}

#[test]
fn ago_cases() {
    assert_eq!(ago(0), "just now");
    assert_eq!(ago(9), "just now");
    assert_eq!(ago(10), "10s ago");
    assert_eq!(ago(190), "3m 10s ago");
}

#[test]
fn clock_cases() {
    // Offset 0 through each zone: the label differs, the digits do not.
    assert_eq!(clock(NOW, NOW, Zone::Utc), "14:30 UTC");
    assert_eq!(clock(NOW, NOW, Zone::Local(0)), "14:30 +00:00");

    let east = Zone::Local(7200);
    let west = Zone::Local(-18_000);
    // Same local day, the instant 12:30 UTC is 14:30 at +02:00.
    assert_eq!(clock(NOW - 7200, NOW, east), "14:30 +02:00");
    assert_eq!(clock(NOW, NOW, east), "16:30 +02:00");
    assert_eq!(clock(NOW, NOW, west), "09:30 -05:00");

    // One day before (Fri) and five days after (Thu); 2026-10-17 is a Saturday.
    assert_eq!(clock(NOW - DAY, NOW, UTC), "Fri 14:30 UTC");
    assert_eq!(clock(NOW + 5 * DAY, NOW, UTC), "Thu 14:30 UTC");
    // The edges of "within six days".
    assert_eq!(clock(NOW - 6 * DAY, NOW, UTC), "Sun 14:30 UTC");
    assert_eq!(clock(NOW + 6 * DAY, NOW, UTC), "Fri 14:30 UTC");

    // The same UTC day but the previous local day names its weekday.
    let early = NOW - 13 * 3600; // 01:30 UTC on the 17th
    assert_eq!(clock(early, NOW, UTC), "01:30 UTC");
    assert_eq!(clock(early, NOW, west), "Fri 20:30 -05:00");
    // And the next local day.
    let late = NOW + 9 * 3600; // 23:30 UTC on the 17th
    assert_eq!(clock(late, NOW, east), "Sun 01:30 +02:00");

    // Seven days away and a year away: the date.
    assert_eq!(clock(NOW - 7 * DAY, NOW, UTC), "2026-10-10 14:30 UTC");
    assert_eq!(clock(NOW + 7 * DAY, NOW, UTC), "2026-10-24 14:30 UTC");
    assert_eq!(clock(NOW - 365 * DAY, NOW, west), "2025-10-17 09:30 -05:00");

    // `at` after `now`, by minutes: still the same day.
    assert_eq!(clock(NOW + 600, NOW, UTC), "14:40 UTC");

    // Extremes: exact, never a panic (`at` clamps to i64::MAX, offsets saturate).
    assert_eq!(clock(u64::MAX, NOW, UTC), "292277026596-12-04 15:30 UTC");
    assert_eq!(clock(NOW, u64::MAX, UTC), "2026-10-17 14:30 UTC");
    assert_eq!(
        clock(u64::MAX, u64::MAX, Zone::Local(i32::MAX)),
        "15:30 +596523:14"
    );
    assert_eq!(
        clock_full(u64::MAX, Zone::Local(i32::MIN)),
        "292277026528-11-16 12:15 -596523:14"
    );
    assert_eq!(clock(0, 0, Zone::Local(-18_000)), "19:00 -05:00");
    assert_eq!(clock(100, 100, Zone::Local(i32::MIN)), "20:47 -596523:14");
}

#[test]
fn clock_full_is_always_the_long_form() {
    assert_eq!(clock_full(NOW, UTC), "2026-10-17 14:30 UTC");
    assert_eq!(clock_full(NOW, Zone::Local(0)), "2026-10-17 14:30 +00:00");
    assert_eq!(
        clock_full(NOW, Zone::Local(7200)),
        "2026-10-17 16:30 +02:00"
    );
    assert_eq!(
        clock_full(NOW, Zone::Local(-18_000)),
        "2026-10-17 09:30 -05:00"
    );
    assert_eq!(
        clock_full(NOW, Zone::Local(19_800)),
        "2026-10-17 20:00 +05:30"
    );
    assert_eq!(clock_full(0, UTC), "1970-01-01 00:00 UTC");
    // Before the epoch in the zone's own day.
    assert_eq!(clock_full(0, Zone::Local(-3600)), "1969-12-31 23:00 -01:00");
}

/// Ruling R17: a date alone, in a zone, labelled the way a clock time is.
#[test]
fn date_is_labelled_with_its_zone() {
    assert_eq!(date(NOW, UTC), "2026-10-17 UTC");
    assert_eq!(date(NOW, Zone::Local(0)), "2026-10-17 +00:00");
    // 14:30 UTC is already the next day at +10:00; 15 h earlier at -01:00 is the day
    // before.
    assert_eq!(date(NOW, Zone::Local(36_000)), "2026-10-18 +10:00");
    assert_eq!(
        date(NOW - 15 * 3600, Zone::Local(-3600)),
        "2026-10-16 -01:00"
    );
    assert_eq!(date(0, Zone::Local(-3600)), "1969-12-31 -01:00");
    assert_eq!(date(u64::MAX, UTC), "292277026596-12-04 UTC");
}

#[test]
fn fill_and_context_text() {
    assert_eq!(fill(183_400, 200_000), Fill::Percent(91));
    assert_eq!(fill(200_000, 200_000), Fill::Percent(100));
    assert_eq!(fill(200_001, 200_000), Fill::Over);
    assert_eq!(fill(5, 0), Fill::NoWindow);
    assert_eq!(fill(u64::MAX, 1), Fill::Over);
    // A u64 multiply by 100 would overflow in each of these.
    assert_eq!(fill(u64::MAX, u64::MAX), Fill::Percent(100));
    assert_eq!(fill(u64::MAX / 2, u64::MAX), Fill::Percent(49));
    assert_eq!(fill(u64::MAX - 1, u64::MAX), Fill::Percent(99));
    assert_eq!(
        context_text(183_400, Some(200_000)),
        "context 91% full (183k / 200k)"
    );
    assert_eq!(
        context_text(450_000, Some(200_000)),
        "context over 100% (450k / 200k window)"
    );
    assert_eq!(
        context_text(183_400, Some(0)),
        "context 183k (window unknown)"
    );
    assert_eq!(context_text(183_400, None), "context 183k (window unknown)");
    assert_eq!(
        context_text(200_000, Some(200_000)),
        "context 100% full (200k / 200k)"
    );
}

#[test]
fn busy_cases() {
    assert_eq!(busy(2, 3), "2/3");
    assert_eq!(busy(3, 3), "3/3");
    assert_eq!(busy(4, 3), "4/3 (over)");
    assert_eq!(busy(0, 0), "0/0");
}

#[test]
fn count_cases() {
    assert_eq!(count(0, "task", "tasks"), "0 tasks");
    assert_eq!(count(1, "task", "tasks"), "1 task");
    assert_eq!(count(2, "task", "tasks"), "2 tasks");
    assert_eq!(count(1, "round", "rounds"), "1 round");
    assert_eq!(count(u64::MAX, "run", "runs"), format!("{} runs", u64::MAX));
}

#[test]
fn budget_text_cases() {
    let b = Budget {
        tool_calls: 40,
        minutes: 20,
        tokens: Some(200_000),
    };
    assert_eq!(budget_text(&b), "40 calls · 20 min · 200k tok");
    let b = Budget { tokens: None, ..b };
    assert_eq!(budget_text(&b), "40 calls · 20 min");
}

#[test]
fn unknown_is_a_word() {
    assert_eq!(UNKNOWN, "unknown");
}
