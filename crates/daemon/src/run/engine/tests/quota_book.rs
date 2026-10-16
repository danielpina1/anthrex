//! Milestone 9.16 decisions 6–8 and 10: the pure `QuotaBook`. Every time is passed in;
//! the book never reads a clock.

use proto::Runtime::{Claude, Codex, Shell};
use proto::brakes::QuotaWindow::{FiveHour, SevenDay};
use proto::brakes::{BrakeTripInfo, Permille, QuotaInfo, QuotaState, QuotaWindow, QuotaWindowInfo};

use crate::headless::SessionEvent;
use crate::headless::claude_stream::ClaudeStream;
use crate::headless::quota::QuotaSample;
use crate::run::engine::quota_book::{BrakeConfig, QuotaBook, Trip, Verdict};
use crate::transcript::fill::codex_limits;

const SCOUT_280: &str =
    include_str!("../../../../tests/fixtures/headless/claude-2.1.280-scout.jsonl");
const CODEX_155: &str = include_str!("../../../../tests/fixtures/transcripts/codex-0.155.0.jsonl");

/// U11's defaults, enabled (the engine's own default is off, ruling C3).
fn on() -> BrakeConfig {
    BrakeConfig {
        quota_brake: true,
        ..BrakeConfig::default()
    }
}

fn sample(windows: &[(QuotaWindow, u16, Option<u64>)]) -> QuotaSample {
    QuotaSample {
        windows: windows
            .iter()
            .map(|(w, used, resets)| (*w, Permille(*used), *resets))
            .collect(),
        rejected: None,
    }
}

fn braked_until(verdict: Verdict) -> Option<u64> {
    match verdict {
        Verdict::Braked(trip) => Some(trip.until),
        _ => None,
    }
}

#[test]
fn engine_default_is_off_and_config_keys_convert() {
    assert_eq!(
        BrakeConfig::default(),
        BrakeConfig {
            quota_brake: false,
            pause_5h_percent: 90,
            pause_7d_percent: 95,
            fresh_secs: 600,
        }
    );
    assert_eq!(BrakeConfig::from(config::BrakeKeys::default()), on());
}

#[test]
fn reading_over_threshold_trips_until_resets_at() {
    let cfg = on();
    let mut book = QuotaBook::default();
    book.record(Claude, &sample(&[(SevenDay, 960, Some(2000))]), 1000, &cfg);
    assert_eq!(
        book.verdict(Claude, 1500, &cfg),
        Verdict::Braked(Trip {
            window: SevenDay,
            used: Permille(960),
            threshold_percent: 95,
            until: 2000,
            read_at: 1000,
        })
    );
    let after = book.verdict(Claude, 2000, &cfg);
    assert!(
        matches!(after, Verdict::Allowed | Verdict::Blind),
        "{after:?}"
    );
    assert!(!matches!(
        book.verdict(Claude, 1999, &cfg),
        Verdict::Braked(_)
    ));
}

#[test]
fn brake_releases_at_resets_at_without_a_new_reading() {
    let cfg = on();
    let mut book = QuotaBook::default();
    book.record(Claude, &sample(&[(FiveHour, 900, Some(5000))]), 1000, &cfg);
    assert_eq!(braked_until(book.verdict(Claude, 4999, &cfg)), Some(5000));
    // No reading arrived since the trip: it is long stale, and it still releases.
    assert_eq!(book.verdict(Claude, 5000, &cfg), Verdict::Blind);
    assert_eq!(book.verdict(Claude, 6000, &cfg), Verdict::Blind);
}

#[test]
fn stale_reading_does_not_release_a_trip() {
    let cfg = BrakeConfig {
        fresh_secs: 600,
        ..on()
    };
    let mut book = QuotaBook::default();
    book.record(
        Claude,
        &sample(&[(SevenDay, 990, Some(10_000))]),
        1000,
        &cfg,
    );
    assert_eq!(braked_until(book.verdict(Claude, 5000, &cfg)), Some(10_000));
    assert_eq!(book.info(5000, &cfg)[0].state, QuotaState::Braked);
}

#[test]
fn fresher_reading_below_threshold_releases() {
    let cfg = on();
    let mut book = QuotaBook::default();
    book.record(Claude, &sample(&[(SevenDay, 960, Some(9000))]), 1000, &cfg);
    // An older reading (before the trip) never clears it.
    book.record(Claude, &sample(&[(SevenDay, 930, Some(9000))]), 900, &cfg);
    assert_eq!(braked_until(book.verdict(Claude, 1050, &cfg)), Some(9000));
    // Nor does one about the other window.
    book.record(Claude, &sample(&[(FiveHour, 100, Some(3000))]), 1050, &cfg);
    assert_eq!(braked_until(book.verdict(Claude, 1060, &cfg)), Some(9000));
    // A newer one of that window below the threshold does.
    book.record(Claude, &sample(&[(SevenDay, 930, Some(9000))]), 1100, &cfg);
    assert_eq!(book.verdict(Claude, 1100, &cfg), Verdict::Allowed);
}

#[test]
fn missing_resets_at_uses_window_length() {
    let cfg = on();
    let mut book = QuotaBook::default();
    book.record(Claude, &sample(&[(FiveHour, 950, None)]), 1000, &cfg);
    assert_eq!(
        braked_until(book.verdict(Claude, 1001, &cfg)),
        Some(1000 + 18_000)
    );
}

/// Task 3's review I2: a reset in milliseconds, or garbage, cannot hold the brake past
/// the window's own length from the reading.
#[test]
fn trip_until_is_clamped_to_the_window_length() {
    let cfg = on();
    let mut book = QuotaBook::default();
    let millis = 1_790_332_405_000;
    book.record(
        Claude,
        &sample(&[
            (FiveHour, 950, Some(millis)),
            (SevenDay, 990, Some(u64::MAX)),
        ]),
        1000,
        &cfg,
    );
    let info = &book.info(1001, &cfg)[0];
    assert_eq!(info.trip.as_ref().map(|t| t.until), Some(1000 + 604_800));
    book.record(Codex, &sample(&[(FiveHour, 950, Some(millis))]), 1000, &cfg);
    assert_eq!(
        braked_until(book.verdict(Codex, 1001, &cfg)),
        Some(1000 + 18_000)
    );
    assert_eq!(book.verdict(Codex, 19_000, &cfg), Verdict::Blind);
    // A reset in the past releases at once (fail open).
    let mut book = QuotaBook::default();
    book.record(Claude, &sample(&[(FiveHour, 950, Some(10))]), 2000, &cfg);
    assert_eq!(book.verdict(Claude, 2000, &cfg), Verdict::Allowed);
}

#[test]
fn rejected_counts_as_full() {
    let cfg = on();
    let mut book = QuotaBook::default();
    let mut s = sample(&[(FiveHour, 200, Some(4000)), (SevenDay, 100, Some(9000))]);
    s.rejected = Some(FiveHour);
    book.record(Claude, &s, 1000, &cfg);
    assert_eq!(
        book.verdict(Claude, 1001, &cfg),
        Verdict::Braked(Trip {
            window: FiveHour,
            used: Permille::FULL,
            threshold_percent: 90,
            until: 4000,
            read_at: 1000,
        })
    );
}

#[test]
fn runtimes_are_independent() {
    let cfg = on();
    let mut book = QuotaBook::default();
    book.record(Codex, &sample(&[(SevenDay, 990, Some(9000))]), 1000, &cfg);
    book.record(Claude, &sample(&[(SevenDay, 100, Some(9000))]), 1000, &cfg);
    assert_eq!(braked_until(book.verdict(Codex, 1001, &cfg)), Some(9000));
    assert_eq!(book.verdict(Claude, 1001, &cfg), Verdict::Allowed);
    // A shell has no quota.
    assert_eq!(book.verdict(Shell, 1001, &cfg), Verdict::Allowed);
}

#[test]
fn stale_or_absent_is_blind_and_disabled_is_allowed() {
    let cfg = BrakeConfig {
        fresh_secs: 600,
        ..on()
    };
    let mut book = QuotaBook::default();
    assert_eq!(book.verdict(Claude, 1000, &cfg), Verdict::Blind);
    book.record(Claude, &sample(&[(SevenDay, 100, Some(9000))]), 1000, &cfg);
    assert_eq!(book.verdict(Claude, 1500, &cfg), Verdict::Allowed);
    assert_eq!(book.verdict(Claude, 1600, &cfg), Verdict::Allowed);
    assert_eq!(book.verdict(Claude, 1700, &cfg), Verdict::Blind);

    let off = BrakeConfig {
        quota_brake: false,
        ..cfg
    };
    let mut book = QuotaBook::default();
    assert_eq!(book.verdict(Claude, 1000, &off), Verdict::Allowed);
    book.record(Claude, &sample(&[(SevenDay, 990, Some(9000))]), 1000, &off);
    for now in [1001, 5000, 100_000] {
        assert_eq!(book.verdict(Claude, now, &off), Verdict::Allowed);
    }
    // Turned on, the stored reading trips.
    book.reconfigure(&cfg);
    assert_eq!(braked_until(book.verdict(Claude, 1001, &cfg)), Some(9000));
}

#[test]
fn reconfigure_rederives_trips() {
    let at_95 = on();
    let mut book = QuotaBook::default();
    book.record(
        Claude,
        &sample(&[(SevenDay, 940, Some(9000))]),
        1000,
        &at_95,
    );
    assert_eq!(book.verdict(Claude, 1001, &at_95), Verdict::Allowed);
    let at_93 = BrakeConfig {
        pause_7d_percent: 93,
        ..at_95
    };
    book.reconfigure(&at_93);
    assert_eq!(
        book.verdict(Claude, 1001, &at_93),
        Verdict::Braked(Trip {
            window: SevenDay,
            used: Permille(940),
            threshold_percent: 93,
            until: 9000,
            read_at: 1000,
        })
    );

    let mut book = QuotaBook::default();
    book.record(Codex, &sample(&[(SevenDay, 960, Some(9000))]), 1000, &at_95);
    assert_eq!(braked_until(book.verdict(Codex, 1001, &at_95)), Some(9000));
    let at_97 = BrakeConfig {
        pause_7d_percent: 97,
        ..at_95
    };
    book.reconfigure(&at_97);
    assert_eq!(book.verdict(Codex, 1001, &at_97), Verdict::Allowed);
}

#[test]
fn allowed_warning_at_94_percent_seven_day_does_not_brake_at_95() {
    let line = SCOUT_280
        .lines()
        .find(|l| l.contains(r#""type":"rate_limit_event""#))
        .unwrap();
    let events = ClaudeStream::default().parse_line(line);
    let [SessionEvent::Quota(s)] = events.as_slice() else {
        panic!("not one Quota");
    };
    let cfg = on();
    let now = 1_790_400_000;
    let mut book = QuotaBook::default();
    book.record(Claude, s, now, &cfg);
    assert_eq!(book.verdict(Claude, now + 1, &cfg), Verdict::Allowed);
    assert_eq!(
        book.info(now + 1, &cfg)[0],
        QuotaInfo {
            runtime: Claude,
            state: QuotaState::Fresh,
            windows: vec![
                QuotaWindowInfo {
                    window: FiveHour,
                    used: Permille(140),
                    resets_at: Some(1_790_375_400),
                },
                QuotaWindowInfo {
                    window: SevenDay,
                    used: Permille(940),
                    resets_at: Some(1_790_456_400),
                },
            ],
            read_at: Some(now),
            trip: None,
        }
    );
}

#[test]
fn codex_99_percent_seven_day_brakes_at_defaults() {
    let s = codex_limits(CODEX_155).expect("the rollout has rate limits");
    let cfg = on();
    let now = 1_790_000_000;
    let mut book = QuotaBook::default();
    book.record(Codex, &s, now, &cfg);
    assert_eq!(
        book.verdict(Codex, now + 60, &cfg),
        Verdict::Braked(Trip {
            window: SevenDay,
            used: Permille(990),
            threshold_percent: 95,
            until: 1_790_332_405,
            read_at: now,
        })
    );
    let info = book.info(now + 60, &cfg);
    assert_eq!(info.len(), 2, "Claude then Codex");
    assert_eq!(info[0].runtime, Claude);
    assert_eq!(info[0].state, QuotaState::Unmeasured);
    assert_eq!(info[1].state, QuotaState::Braked);
    assert_eq!(
        info[1].trip,
        Some(BrakeTripInfo {
            window: SevenDay,
            used: Permille(990),
            threshold_percent: 95,
            until: 1_790_332_405,
        })
    );
    assert_eq!(book.verdict(Codex, 1_790_332_405, &cfg), Verdict::Blind);
}

#[test]
fn info_states_follow_freshness_and_the_switch() {
    let cfg = on();
    let mut book = QuotaBook::default();
    book.record(Claude, &sample(&[(SevenDay, 100, None)]), 1000, &cfg);
    assert_eq!(book.info(1600, &cfg)[0].state, QuotaState::Fresh);
    assert_eq!(book.info(1601, &cfg)[0].state, QuotaState::Stale);
    book.record(Claude, &sample(&[(SevenDay, 990, Some(9000))]), 2000, &cfg);
    assert_eq!(book.info(2001, &cfg)[0].state, QuotaState::Braked);
    // Disabled: the trip is not shown, and nothing is braked.
    let off = BrakeConfig {
        quota_brake: false,
        ..cfg
    };
    let info = &book.info(2001, &off)[0];
    assert_eq!((info.state, info.trip.clone()), (QuotaState::Fresh, None));
    // Past `until`, `info` alone (no verdict call) no longer shows it braked.
    assert_eq!(book.info(9000, &cfg)[0].state, QuotaState::Stale);
}
