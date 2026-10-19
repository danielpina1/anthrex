//! The host's sleeps and the wall clock's backward steps, measured for the reducer
//! (`EventKind::Slept`, `EventKind::SteppedBack`). The reducer's `now` is wall time
//! (`unix_now`), which a suspend moves forward; `Instant` (`CLOCK_UPTIME_RAW` on macOS,
//! `CLOCK_MONOTONIC` on Linux) does not advance while the host sleeps. The wall clock's
//! lead over the awake clock since the first read is its offset; a rise of the offset
//! is a sleep (or a forward step of the clock, which looks the same), a fall a step
//! back. Every read measures the offset from the same anchor and rounds it the same
//! way, and a step reported is the offset's change since the last one reported, so the
//! steps telescope: a forward step and its correction cancel (review round 1, M1). In
//! memory only: a new daemon starts a new anchor, and the downtime before it is the
//! restore's (`engine/restore.rs`).

use std::sync::Mutex;
use std::time::{Duration, Instant};

/// A change of the offset of up to this many seconds is neither a sleep nor a step:
/// `now` is truncated to whole seconds, and a clock adjustment slews it a little. It is
/// not lost: it counts once the offset has moved past the slack.
pub(super) const SLEEP_SLACK_SECS: u64 = 2;

/// What the wall clock did since the last step reported, beyond the slack.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ClockStep {
    /// It kept pace with the awake clock (or this is the first read).
    Kept,
    /// It ran this many seconds ahead: the host slept, or the clock was set forward.
    Slept(u64),
    /// It fell this many seconds behind: the clock was set back.
    Back(u64),
}

/// The first read, `(wall second, awake instant)`, and the offset (see [`offset`])
/// already reported as steps.
#[derive(Debug, Clone, Copy)]
pub(super) struct Anchor {
    wall: u64,
    at: Instant,
    stepped: i128,
}

impl Anchor {
    pub(super) fn new(wall: u64, at: Instant) -> Self {
        Anchor {
            wall,
            at,
            stepped: 0,
        }
    }
}

#[derive(Default)]
pub(super) struct AwakeClock(Mutex<Option<Anchor>>);

impl AwakeClock {
    /// What the wall clock did since the last step reported; the first read is the
    /// anchor.
    pub(super) fn read(&self, wall: u64, awake: Instant) -> ClockStep {
        let mut anchor = crate::lock(&self.0);
        let Some(a) = anchor.as_mut() else {
            *anchor = Some(Anchor::new(wall, awake));
            return ClockStep::Kept;
        };
        let off = offset(a.wall, wall, awake.saturating_duration_since(a.at));
        let step = off - a.stepped;
        let slack = i128::from(SLEEP_SLACK_SECS);
        let secs = |s: i128| u64::try_from(s.unsigned_abs()).unwrap_or(u64::MAX);
        if step.abs() <= slack {
            return ClockStep::Kept;
        }
        a.stepped = off;
        match step > 0 {
            true => ClockStep::Slept(secs(step)),
            false => ClockStep::Back(secs(step)),
        }
    }
}

/// The wall clock's lead over the awake clock from the anchor's wall second `was` to
/// `wall`, `awake` the awake time between: the wall's seconds less the whole awake
/// seconds. Rounding the awake time down makes it read high by less than a second, and
/// both ends of a step read through the same rounding, so a step back is never
/// overstated by a whole second, and a forward step and its correction net 0 or 1
/// (`a_forward_step_and_its_correction_never_net_a_step_back`).
fn offset(was: u64, wall: u64, awake: Duration) -> i128 {
    i128::from(wall) - i128::from(was) - i128::from(awake.as_secs())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A year, the forward step's length below.
    const X: u64 = 365 * 24 * 3_600;

    /// The net shift (`Slept` less `Back`) that a forward step of `X` read `a1`
    /// milliseconds awake after the anchor, and its correction read `a2` later, step:
    /// the wall clock's true second is `w0 + phase` tenths at the anchor.
    fn net_shift(phase: u64, a1: u64, a2: u64) -> i64 {
        let clock = AwakeClock::default();
        let at = Instant::now();
        let wall = |ms: u64| 100 + (phase * 100 + ms) / 1_000;
        assert_eq!(clock.read(wall(0), at), ClockStep::Kept);
        let ahead = clock.read(wall(a1) + X, at + Duration::from_millis(a1));
        let back = clock.read(wall(a1 + a2), at + Duration::from_millis(a1 + a2));
        let (ClockStep::Slept(f), ClockStep::Back(b)) = (ahead, back) else {
            panic!("{ahead:?} then {back:?} (phase {phase}, {a1} ms, {a2} ms)");
        };
        i64::try_from(f).unwrap() - i64::try_from(b).unwrap()
    }

    /// Review round 1 (M1): each read rounded on its own, so a step and its correction
    /// read 1.5 s and then 0.3 s awake after the anchor came out as `Slept(X - 1)` and
    /// `Back(X)`: the waits moved a second earlier than before, and a stall fired a
    /// second short. Measured from one anchor, the two cancel.
    #[test]
    fn a_forward_step_and_its_correction_cancel_across_fractional_spans() {
        assert_eq!(net_shift(0, 1_500, 300), 0);
    }

    /// Whatever the wall clock's phase and the awake spans, the net shift of a forward
    /// step and its correction is never negative, so no wait ends early, and at most a
    /// second, the rounding of one wall second.
    #[test]
    fn a_forward_step_and_its_correction_never_net_a_step_back() {
        for phase in 0..10 {
            for a1 in [0, 100, 300, 500, 900, 1_000, 1_500, 1_999, 2_700] {
                for a2 in [0, 100, 300, 500, 900, 1_000, 1_500, 1_999, 2_700] {
                    let net = net_shift(phase, a1, a2);
                    assert!((0..=1).contains(&net), "{net} (phase {phase}, {a1}, {a2})");
                }
            }
        }
    }

    /// One step: the anchor at wall second `was`, then a read at `wall`, `ms` awake
    /// milliseconds later.
    fn once(was: u64, wall: u64, ms: u64) -> ClockStep {
        let clock = AwakeClock::default();
        let at = Instant::now();
        assert_eq!(clock.read(was, at), ClockStep::Kept);
        clock.read(wall, at + Duration::from_millis(ms))
    }

    #[test]
    fn a_wall_jump_with_no_awake_time_is_a_sleep() {
        assert_eq!(once(0, 1_001, 1_200), ClockStep::Slept(1_000));
        assert_eq!(once(0, 1_000, 0), ClockStep::Slept(1_000));
    }

    #[test]
    fn awake_time_and_rounding_are_not_a_sleep() {
        assert_eq!(once(0, 1, 300), ClockStep::Kept);
        assert_eq!(once(0, 3, 1_000), ClockStep::Kept);
        assert_eq!(once(0, 600, 600_000), ClockStep::Kept);
    }

    #[test]
    fn a_wall_falling_behind_the_awake_clock_is_a_step_back() {
        assert_eq!(once(100, 100, 5_000), ClockStep::Back(5));
        assert_eq!(once(1_000, 50, 1_500), ClockStep::Back(951));
        // Within the slack, the awake seconds rounded down: no step.
        assert_eq!(once(100, 100, 2_900), ClockStep::Kept);
        assert_eq!(once(100, 98, 0), ClockStep::Kept);
        assert_eq!(once(100, 97, 0), ClockStep::Back(3));
        // Keeping pace, or ahead, is no step back.
        assert_eq!(once(100, 700, 600_000), ClockStep::Kept);
        assert_eq!(once(100, 1_100, 1_000), ClockStep::Slept(999));
    }

    /// A drift within the slack is not lost: it counts once the offset has moved past
    /// the slack, and only what was not reported yet.
    #[test]
    fn a_drift_within_the_slack_counts_once_past_it() {
        let clock = AwakeClock::default();
        let at = Instant::now();
        let read = |wall, secs| clock.read(wall, at + Duration::from_secs(secs));
        assert_eq!(read(100, 0), ClockStep::Kept);
        assert_eq!(read(100, 2), ClockStep::Kept);
        assert_eq!(read(100, 3), ClockStep::Back(3));
        assert_eq!(read(101, 4), ClockStep::Kept);
        assert_eq!(read(107, 5), ClockStep::Slept(5));
    }

    #[test]
    fn the_clock_reads_from_its_last_anchor() {
        let clock = AwakeClock::default();
        let at = Instant::now();
        let read = |wall, secs| clock.read(wall, at + Duration::from_secs(secs));
        assert_eq!(
            read(100, 0),
            ClockStep::Kept,
            "the first read is the anchor"
        );
        assert_eq!(read(101, 1), ClockStep::Kept);
        assert_eq!(read(1_102, 2), ClockStep::Slept(1_000));
        assert_eq!(read(1_103, 3), ClockStep::Kept);
        // Set back to true time: the lag is the forward step's.
        assert_eq!(read(104, 4), ClockStep::Back(1_000));
        assert_eq!(read(105, 5), ClockStep::Kept);
    }
}

#[cfg(test)]
#[path = "awake_handle_tests.rs"]
mod handle_tests;
