//! Milestone 8b's additions to the run model. Pure (M8b decision 1): no file,
//! process, thread, async-runtime or clock access. Declared by `model.rs`.

use serde::{Deserialize, Serialize};

use super::Run;

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

/// `RunLimits.decider_mode` of a run recorded before milestone 8b.
pub(super) fn decider_mode_absent() -> proto::DeciderMode {
    proto::DeciderMode::Off
}

/// `RunLimits.decider_slot_wait_secs` of a run recorded before milestone 8b: the
/// config's default.
pub(super) fn slot_wait_absent() -> u64 {
    30
}
