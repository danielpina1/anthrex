//! The host's sleeps and the wall clock's backward steps, measured for the reducer
//! (`EventKind::Slept`, `EventKind::SteppedBack`). The reducer's `now` is wall time
//! (`unix_now`), which a suspend moves forward; `Instant` (`CLOCK_UPTIME_RAW` on macOS,
//! `CLOCK_MONOTONIC` on Linux) does not advance while the host sleeps. The wall clock's
//! lead over the awake clock since the first read is its offset, read to the
//! nanosecond; a rise of the offset is a sleep (or a forward step of the clock, which
//! looks the same), a fall a step back. In memory only: a new daemon starts a new
//! anchor, and the downtime before it is the restore's (`engine/restore.rs`).
//!
//! **Every error runs late** (review round 2 of the clock-step fix). The reducer moves
//! each live wait by every step reported while it runs, in whole seconds. A wait never
//! ends before its true awake time if, between any read and a later one, the steps
//! reported add up to at least the offset's true change between them. Write
//! `lead = reported − offset`, the seconds reported beyond the offset. Then that holds
//! exactly when `lead` never falls from one read to a later one, and the clock keeps
//! it so: `lead` is never let below the highest it has been (`high`, less
//! [`READ_TOLERANCE`]), and a step is reported only in whole seconds that keep it
//! there. A rise of the offset is reported at once, rounded up, as a `Slept`. A fall
//! is reported as a `Back` only in whole seconds past `high`, rounded down, so a slow
//! drift back is never reported (the waits it spans run long by it), and a step back
//! is understated by under a second. The cost is lateness: each `Slept` overstates by
//! under a second, and a forward drift of the awake clock reports `Slept(1)` each time
//! it gains [`READ_TOLERANCE`] on `high`.

use std::sync::Mutex;
use std::time::{Duration, Instant};

/// A step reported of up to this many seconds is logged at debug, not info: a forward
/// drift of the awake clock, or the rounding up of a short sleep.
pub(super) const SLEEP_SLACK_SECS: u64 = 2;

/// How far the seconds reported may lag a rise of the offset before a `Slept` makes
/// them up: the two clocks are not read at the same instant, and the wall clock may
/// slew between reads. The one error that may run early, by at most this much; a larger
/// one would let more run early, a smaller one report `Slept(1)` more often under a
/// slow awake clock (each a second late).
pub(super) const READ_TOLERANCE: Duration = Duration::from_millis(100);

/// A second, in nanoseconds.
const SEC: i128 = 1_000_000_000;

/// What the wall clock did since the last read.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ClockStep {
    /// It kept pace with the awake clock (or this is the first read).
    Kept,
    /// It ran this many seconds ahead: the host slept, or the clock was set forward.
    Slept(u64),
    /// It fell this many seconds behind: the clock was set back.
    Back(u64),
}

/// The first read, `(wall time, awake instant)`; the seconds reported since; and the
/// highest `lead` (reported less the offset, in nanoseconds) after any read.
#[derive(Debug, Clone, Copy)]
pub(super) struct Anchor {
    wall: Duration,
    at: Instant,
    reported: i128,
    high: i128,
}

impl Anchor {
    pub(super) fn new(wall: Duration, at: Instant) -> Self {
        Anchor {
            wall,
            at,
            reported: 0,
            high: 0,
        }
    }
}

#[derive(Default)]
pub(super) struct AwakeClock(Mutex<Option<Anchor>>);

impl AwakeClock {
    /// What the wall clock did since the last read, `wall` the time since the epoch;
    /// the first read is the anchor.
    pub(super) fn read(&self, wall: Duration, awake: Instant) -> ClockStep {
        let mut anchor = crate::lock(&self.0);
        let Some(a) = anchor.as_mut() else {
            *anchor = Some(Anchor::new(wall, awake));
            return ClockStep::Kept;
        };
        let nanos = |d: Duration| i128::try_from(d.as_nanos()).unwrap_or(i128::MAX);
        let off = nanos(wall) - nanos(a.wall) - nanos(awake.saturating_duration_since(a.at));
        let lead = a.reported * SEC - off;
        let tolerance = nanos(READ_TOLERANCE);
        let step = if lead < a.high - tolerance {
            // Rounded up: `lead` back to `high` at least.
            let secs = (a.high - lead + SEC - 1) / SEC;
            a.reported += secs;
            ClockStep::Slept(whole(secs))
        } else if lead >= a.high + SEC {
            // Rounded down: `lead` stays at `high` at least.
            let secs = (lead - a.high) / SEC;
            a.reported -= secs;
            ClockStep::Back(whole(secs))
        } else {
            ClockStep::Kept
        };
        a.high = a.high.max(a.reported * SEC - off);
        step
    }
}

fn whole(secs: i128) -> u64 {
    u64::try_from(secs).unwrap_or(u64::MAX)
}

#[cfg(test)]
#[path = "awake_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "awake_handle_tests.rs"]
mod handle_tests;
