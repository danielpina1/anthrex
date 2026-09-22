//! Reconnecting to the daemon after the link drops (decisions 30-32, M6.11): the pure
//! retry schedule the event loop consults, and the I/O attempt it runs on a spawned
//! task so the UI keeps responding while it waits.

use crate::connection::Connection;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

/// How long the event loop waits before the first attempt after a drop, and between
/// every attempt after that (decision 31).
pub const RETRY_INTERVAL: Duration = Duration::from_secs(2);

/// How long after the drop (or after a manual `C-b r`) the event loop keeps retrying
/// before the link gives up (decision 31 and 32).
pub const RETRY_WINDOW: Duration = Duration::from_secs(30);

/// A pure schedule of when the next reconnect attempt is due, and when to give up.
/// Takes every `Instant` from the caller rather than reading the clock itself, so it
/// can be driven with synthetic time in tests — see `docs/timing-budgets.md` on why a
/// bound must be derived from the constants it is entitled to use, never read off the
/// wall clock inside the thing being tested.
#[derive(Debug, Clone, Copy)]
pub struct RetrySchedule {
    /// The moment a failure stops being retried automatically: `RETRY_WINDOW` after
    /// the link dropped, or after the `C-b r` that opened this window.
    deadline: Instant,
    next_due: Instant,
}

impl RetrySchedule {
    /// Decision 31: the link just dropped. The first attempt is due `RETRY_INTERVAL`
    /// later, and automatic attempts give up `RETRY_WINDOW` after `now`.
    pub fn after_drop(now: Instant) -> Self {
        Self {
            deadline: now + RETRY_WINDOW,
            next_due: now + RETRY_INTERVAL,
        }
    }

    /// Decision 32: `C-b r`. Due at once, and opens its own fresh `RETRY_WINDOW`.
    pub fn manual(now: Instant) -> Self {
        Self {
            deadline: now + RETRY_WINDOW,
            next_due: now,
        }
    }

    /// Whether an attempt is due at `now`.
    pub fn is_due(&self, now: Instant) -> bool {
        now >= self.next_due
    }

    /// When the next attempt is due.
    pub fn next_due(&self) -> Instant {
        self.next_due
    }

    /// Records that an attempt just failed at `now`. Returns whether to keep
    /// retrying: `true` schedules the next attempt `RETRY_INTERVAL` later; `false`
    /// means this failure landed at or after the window's deadline, so the caller
    /// should give up instead of scheduling another one.
    pub fn after_failure(&mut self, now: Instant) -> bool {
        if now >= self.deadline {
            false
        } else {
            self.next_due = now + RETRY_INTERVAL;
            true
        }
    }
}

/// One reconnect attempt: connect, and if that fails and `daemon_exe` is `Some`,
/// start the daemon (`crate::spawn::ensure_daemon`) and try once more. Automatic
/// attempts pass `None` — decision 31's "a daemon stopped on purpose stays stopped"
/// — only a manual `C-b r` (decision 32) may spawn one.
pub async fn attempt(socket: &Path, daemon_exe: Option<PathBuf>) -> anyhow::Result<Connection> {
    match Connection::connect(socket).await {
        Ok(conn) => Ok(conn),
        Err(err) => {
            let Some(exe) = daemon_exe else {
                return Err(err);
            };
            crate::spawn::ensure_daemon(&exe, socket).await?;
            Connection::connect(socket).await
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn retry_schedule_timing() {
        let t0 = Instant::now();

        let after_drop = RetrySchedule::after_drop(t0);
        assert!(!after_drop.is_due(t0 + Duration::from_millis(1900)));
        assert!(after_drop.is_due(t0 + Duration::from_secs(2)));

        let mut after_drop = RetrySchedule::after_drop(t0);
        let failed_at = t0 + Duration::from_millis(2100);
        assert!(after_drop.after_failure(failed_at));
        assert_eq!(after_drop.next_due(), t0 + Duration::from_millis(4100));

        let mut gave_up = RetrySchedule::after_drop(t0);
        assert!(!gave_up.after_failure(t0 + RETRY_WINDOW));

        let t1 = t0 + Duration::from_secs(500);
        let manual = RetrySchedule::manual(t1);
        assert!(manual.is_due(t1));

        let mut manual = RetrySchedule::manual(t1);
        assert!(manual.after_failure(t1 + Duration::from_millis(29_999)));

        let mut manual_gives_up = RetrySchedule::manual(t1);
        assert!(!manual_gives_up.after_failure(t1 + RETRY_WINDOW));
    }
}
