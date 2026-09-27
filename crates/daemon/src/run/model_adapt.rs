//! Milestone 8b's additions to the run model. Pure (M8b decision 1): no file,
//! process, thread, async-runtime or clock access. Declared by `model.rs`.

use serde::{Deserialize, Serialize};

use super::{Run, Task};

impl Run {
    /// M8b decision 7's attention line, while the stored profile is stale: a stale
    /// profile is still used, and the user is told which files changed.
    pub fn stale_profile_line(&self) -> Option<String> {
        (!self.stale_profile.is_empty()).then(|| {
            format!(
                "the repository profile may be stale: {} changed since it was confirmed; run anthrex profile detect",
                self.stale_profile.join(", ")
            )
        })
    }

    /// M8b decision 25's attention line, once the user asked to promote the run. It
    /// carries no time (M8c decision 4: the snapshot formats no time); a client formats
    /// the raw `promote_requested_at` in local time.
    pub fn promotion_line(&self) -> Option<String> {
        self.promote_requested_at.map(|_| {
            "promotion requested; it takes effect when the orchestrator exists (milestone 9)"
                .to_string()
        })
    }
}

/// `<hh:mm>` in UTC, for `run promote`'s reply to a second request. Never used in the
/// snapshot, which carries raw times only (M8c decision 4).
pub fn hh_mm(at: u64) -> String {
    format!("{:02}:{:02}", (at / 3600) % 24, (at / 60) % 60)
}

impl Task {
    /// Ends a size cross-check the task still waits for (M8b decision 19; a cancel).
    pub fn drop_pending_size_check(&mut self) {
        self.size_check
            .take_if(|s| matches!(s, SizeCheckState::Pending { .. }));
    }
}

/// A decider waiting for a reader slot (M8b decision 18). `decider_id` is the run's own
/// count, so the task that waits for the answer names it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct QueuedDecider {
    pub decider_id: u64,
    pub task_ids: Vec<String>,
    pub request: crate::decider::DeciderRequest,
    pub queued_at: u64,
}

/// A failed check's rung, counted but not yet taken (M8b decision 20): taken once the
/// check summary `decider_id` is decided. `check_index` is the record in `Task.checks`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PendingFailure {
    pub gate: proto::GateKind,
    pub rung: u8,
    pub check_index: usize,
    pub decider_id: u64,
}

/// A task's size cross-check (M8b decision 19): waiting for the size-check decider
/// `decider_id` (the task is not runnable meanwhile), or answered.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum SizeCheckState {
    Pending { decider_id: u64 },
    Done(proto::SizeCheckInfo),
}

/// `RunLimits.decider_mode` of a run recorded before milestone 8b.
pub(super) fn decider_mode_absent() -> proto::DeciderMode {
    proto::DeciderMode::Off
}

/// `RunLimits.decider_slot_wait_secs` of a run recorded before milestone 8b: the
/// config's default.
pub(super) fn slot_wait_absent() -> u64 {
    30
}
