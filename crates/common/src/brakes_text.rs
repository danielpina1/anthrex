//! Milestone 9.16 (ruling C13): the quota brake's text that the daemon's attention line,
//! the TUI's alert and the CLI's `brake_lines` all print, so the three never drift.
//! Pure.

use proto::Runtime;
use proto::brakes::BrakeTripInfo;

use crate::text::hh_mm;

/// Decision 13: `<runtime> dispatch held: <window> at <n>% (≥<t>%) until <HH:MM> UTC`.
pub fn quota_held_text(runtime: Runtime, trip: &BrakeTripInfo) -> String {
    format!(
        "{runtime} dispatch held: {} at {}% (≥{}%) until {} UTC",
        trip.window.label(),
        trip.used.percent(),
        trip.threshold_percent,
        hh_mm(trip.until),
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

    #[test]
    fn quota_held_text_names_the_window_usage_threshold_and_utc_time() {
        let trip = BrakeTripInfo {
            window: QuotaWindow::SevenDay,
            used: Permille(968),
            threshold_percent: 95,
            // 1970-01-02 14:30 UTC.
            until: 86_400 + 14 * 3600 + 30 * 60,
        };
        assert_eq!(
            quota_held_text(Runtime::Claude, &trip),
            "claude dispatch held: 7d at 96% (≥95%) until 14:30 UTC"
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
