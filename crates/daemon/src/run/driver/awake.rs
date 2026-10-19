//! The host's sleeps and the wall clock's backward steps, measured for the reducer
//! (`EventKind::Slept`, `EventKind::SteppedBack`). The reducer's `now` is wall time
//! (`unix_now`), which a suspend moves forward; `Instant` (`CLOCK_UPTIME_RAW` on macOS,
//! `CLOCK_MONOTONIC` on Linux) does not advance while the host sleeps. Between two
//! reads, the wall clock's lead over the awake clock is the time the host slept (or a
//! forward step of the clock, which looks the same), and its lag behind the awake
//! clock is a step back. In memory only: a new daemon starts a new anchor, and the
//! downtime before it is the restore's (`engine/restore.rs`).

use std::sync::Mutex;
use std::time::{Duration, Instant};

/// A lead or a lag of up to this many seconds is neither a sleep nor a step: `now` is
/// truncated to whole seconds, and a clock adjustment slews it a little.
pub(super) const SLEEP_SLACK_SECS: u64 = 2;

/// What the wall clock did since the last read, beyond the slack.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ClockStep {
    /// It kept pace with the awake clock (or this is the first read).
    Kept,
    /// It ran this many seconds ahead: the host slept, or the clock was set forward.
    Slept(u64),
    /// It fell this many seconds behind: the clock was set back.
    Back(u64),
}

/// The last `(wall seconds, awake instant)` read.
#[derive(Default)]
pub(super) struct AwakeClock(Mutex<Option<(u64, Instant)>>);

impl AwakeClock {
    /// What the wall clock did since the last read, with `wall` and `awake` the new
    /// anchor.
    pub(super) fn read(&self, wall: u64, awake: Instant) -> ClockStep {
        let last = crate::lock(&self.0).replace((wall, awake));
        let Some((was, at)) = last else {
            return ClockStep::Kept;
        };
        let span = awake.saturating_duration_since(at);
        match (
            slept(wall.saturating_sub(was), span),
            stepped_back(was, wall, span),
        ) {
            (0, 0) => ClockStep::Kept,
            (0, back) => ClockStep::Back(back),
            (secs, _) => ClockStep::Slept(secs),
        }
    }
}

/// The sleep in a span the wall clock measured as `wall` seconds and the awake clock
/// as `awake`: the wall's lead, rounded down, once it is past [`SLEEP_SLACK_SECS`].
pub(super) fn slept(wall: u64, awake: Duration) -> u64 {
    let awake = awake.as_secs() + u64::from(awake.subsec_nanos() > 0);
    match wall.saturating_sub(awake) {
        lead if lead > SLEEP_SLACK_SECS => lead,
        _ => 0,
    }
}

/// The step back in a span from wall second `was` to `wall` that the awake clock
/// measured as `awake`: the wall's lag, the awake seconds rounded down (so the step is
/// never overstated by the rounding), once it is past [`SLEEP_SLACK_SECS`].
pub(super) fn stepped_back(was: u64, wall: u64, awake: Duration) -> u64 {
    match was.saturating_add(awake.as_secs()).saturating_sub(wall) {
        lag if lag > SLEEP_SLACK_SECS => lag,
        _ => 0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_wall_jump_with_no_awake_time_is_a_sleep() {
        assert_eq!(slept(1_001, Duration::from_millis(1_200)), 999);
        assert_eq!(slept(1_000, Duration::ZERO), 1_000);
    }

    #[test]
    fn awake_time_and_rounding_are_not_a_sleep() {
        assert_eq!(slept(1, Duration::from_millis(300)), 0);
        assert_eq!(slept(3, Duration::from_millis(1_000)), 0);
        assert_eq!(slept(600, Duration::from_secs(600)), 0);
        // A clock set back is no sleep.
        assert_eq!(slept(0, Duration::from_secs(5)), 0);
    }

    #[test]
    fn a_wall_falling_behind_the_awake_clock_is_a_step_back() {
        assert_eq!(stepped_back(100, 100, Duration::from_secs(5)), 5);
        assert_eq!(stepped_back(1_000, 50, Duration::from_millis(1_500)), 951);
        // Within the slack, the awake seconds rounded down: no step.
        assert_eq!(stepped_back(100, 100, Duration::from_millis(2_900)), 0);
        assert_eq!(stepped_back(100, 98, Duration::ZERO), 0);
        assert_eq!(stepped_back(100, 97, Duration::ZERO), 3);
        // Keeping pace, or ahead, is no step back.
        assert_eq!(stepped_back(100, 700, Duration::from_secs(600)), 0);
        assert_eq!(stepped_back(100, 1_100, Duration::from_secs(1)), 0);
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
