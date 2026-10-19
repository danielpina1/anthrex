//! The host's sleeps, measured for the reducer (`EventKind::Slept`). The reducer's
//! `now` is wall time (`unix_now`), which a suspend moves forward; `Instant`
//! (`CLOCK_UPTIME_RAW` on macOS, `CLOCK_MONOTONIC` on Linux) does not advance while the
//! host sleeps. Between two reads, the wall clock's lead over the awake clock is the
//! time the host slept. In memory only: a new daemon starts a new anchor, and the
//! downtime before it is the restore's (`engine/restore.rs`).

use std::sync::Mutex;
use std::time::{Duration, Instant};

/// A lead of up to this many seconds is not a sleep: `now` is truncated to whole
/// seconds, and a clock adjustment slews it a little.
pub(super) const SLEEP_SLACK_SECS: u64 = 2;

/// The last `(wall seconds, awake instant)` read.
#[derive(Default)]
pub(super) struct AwakeClock(Mutex<Option<(u64, Instant)>>);

impl AwakeClock {
    /// The seconds the host slept since the last read (0 if none), with `wall` and
    /// `awake` the new anchor.
    pub(super) fn read(&self, wall: u64, awake: Instant) -> u64 {
        let last = crate::lock(&self.0).replace((wall, awake));
        last.map_or(0, |(was, at)| {
            slept(
                wall.saturating_sub(was),
                awake.saturating_duration_since(at),
            )
        })
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
    fn the_clock_reads_from_its_last_anchor() {
        let clock = AwakeClock::default();
        let at = Instant::now();
        assert_eq!(clock.read(100, at), 0, "the first read is the anchor");
        assert_eq!(clock.read(101, at + Duration::from_secs(1)), 0);
        assert_eq!(clock.read(1_102, at + Duration::from_secs(2)), 1_000);
        assert_eq!(clock.read(1_103, at + Duration::from_secs(3)), 0);
    }
}
