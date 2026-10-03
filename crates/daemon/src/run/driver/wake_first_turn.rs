//! Milestone 9.5 decision 38 (task M9.5.5a, review ruling C1), the driver's half of the
//! orchestrator's first turn. The engine emits it, once its anthrex server has sent
//! `McpReady`, as an `Effect::WakeOrchestrator` with `first_turn`; the driver keeps it
//! as round [`FIRST_TURN`]'s request wake, so 9.3's request handling carries it
//! unchanged: pasted whole, kept while the engine holds it, never pasted twice. On top
//! of `ready`, it goes only to a window that has sent a hook or title signal: a quiet
//! window with none is `Idle` and is exactly a start or folder-trust prompt, whose
//! answer the paste's `\r` would be. Why it waits is logged once per reason
//! (`wake_report.rs`, decision 41).

use proto::WindowInfo;

use super::{WaitReason, Wakes};
use crate::run::model::Run;

/// The round a first turn is kept under: rounds count from 1 (`Run::round`), so no
/// round's request wake is ever round 0.
pub(super) const FIRST_TURN: u32 = 0;

/// The round a wake-up is kept under: [`FIRST_TURN`] for a first turn.
pub(super) fn kept_as(request: Option<u32>, first_turn: bool) -> Option<u32> {
    if first_turn {
        Some(FIRST_TURN)
    } else {
        request
    }
}

/// The request wake the engine holds for `run` (`Seen::request`): [`FIRST_TURN`] while
/// its first turn is pending and its anthrex server ready (what the engine emits it
/// on), else its current round's request, if any.
pub(super) fn request_held(run: &Run) -> Option<u32> {
    let pending = run
        .orch
        .orchestrator
        .as_ref()
        .is_some_and(|o| o.first_turn_pending);
    let first = (pending && run.orch.mcp_ready).then_some(FIRST_TURN);
    first.or(run.orch.request_wake.as_ref().map(|_| run.round()))
}

impl Wakes {
    /// Why `run_id`'s waiting wake-up, otherwise ready for `window`, may not go yet: a
    /// first turn goes only once the window has sent a signal.
    pub(super) fn unsignalled(&self, run_id: &str, window: &WindowInfo) -> Option<WaitReason> {
        let first = crate::lock(&self.pending)
            .get(run_id)
            .is_some_and(|p| p.request == Some(FIRST_TURN));
        (first && !window.signals_seen).then_some(WaitReason::NoSignal)
    }
}

#[cfg(test)]
#[path = "wake_first_turn_tests.rs"]
pub(super) mod tests;
