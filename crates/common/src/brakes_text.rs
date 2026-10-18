//! Milestone 9.16 (ruling C13): the quota brake's text that the daemon's attention line,
//! the TUI's alert and the CLI's `brake_lines` all print, so the three never drift.
//! Pure.

use proto::Runtime;
use proto::brakes::BrakeTripInfo;

use crate::numbers::{Zone, clock};

/// Decision 13: `<runtime> dispatch held: <window> at <n>% (≥<t>%) until <clock>`, the
/// time through [`clock`] against `now` in `zone` (milestone 9.19 decision 3): `Utc`
/// from the daemon, the viewer's offset from a client.
pub fn quota_held_text(runtime: Runtime, trip: &BrakeTripInfo, now: u64, zone: Zone) -> String {
    format!(
        "{runtime} dispatch held: {} at {}% (≥{}%) until {}",
        trip.window.label(),
        trip.used.percent(),
        trip.threshold_percent,
        clock(trip.until, now, zone),
    )
}

/// Decision 42 (`FAILOVER_ATTENTION_TEXT`): `<runtime> braked: <who> started on <other>
/// (fallback)`.
pub fn failover_attention_text(runtime: Runtime, who: &str, other: Runtime) -> String {
    format!("{runtime} braked: {who} started on {other} (fallback)")
}

#[cfg(test)]
mod tests {
    use proto::brakes::{Permille, QuotaWindow};

    use super::*;
    use crate::numbers::Zone;

    /// 1970-01-02 14:30 UTC.
    const UNTIL: u64 = 86_400 + 14 * 3600 + 30 * 60;

    fn trip(until: u64) -> BrakeTripInfo {
        BrakeTripInfo {
            window: QuotaWindow::SevenDay,
            used: Permille(968),
            threshold_percent: 95,
            until,
        }
    }

    #[test]
    fn quota_held_text_names_the_window_usage_threshold_and_utc_time() {
        assert_eq!(
            quota_held_text(Runtime::Claude, &trip(UNTIL), UNTIL - 3600, Zone::Utc),
            "claude dispatch held: 7d at 96% (≥95%) until 14:30 UTC"
        );
    }

    /// Decision 3: a brake lifting on another day names it, in the daemon's UTC and in
    /// a client's local offset.
    #[test]
    fn a_seven_day_brake_names_its_day() {
        // 2026-10-17 14:30 UTC, a Saturday; five days on is Thursday.
        let now = 1_792_247_400;
        let until = now + 5 * 86_400;
        assert_eq!(
            quota_held_text(Runtime::Claude, &trip(until), now, Zone::Utc),
            "claude dispatch held: 7d at 96% (≥95%) until Thu 14:30 UTC"
        );
        assert_eq!(
            quota_held_text(Runtime::Codex, &trip(until), now, Zone::Local(7200)),
            "codex dispatch held: 7d at 96% (≥95%) until Thu 16:30 +02:00"
        );
    }

    #[test]
    fn failover_attention_text_names_both_runtimes_and_who() {
        assert_eq!(
            failover_attention_text(Runtime::Claude, "t3", Runtime::Codex),
            "claude braked: t3 started on codex (fallback)"
        );
    }
}
