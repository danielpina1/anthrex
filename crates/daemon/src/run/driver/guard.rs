//! Final review B-I2: `step` under `catch_unwind`. A bug in the reducer must not take
//! the engine's state, its event loop or the daemon's start with it (AGENTS.md rule 3's
//! aim: one bad window, or here one bad run, never takes the daemon down).

use std::panic::{AssertUnwindSafe, catch_unwind};

use crate::run::engine::{EngineState, Event, EventKind, ReplyId, step};

/// What a request whose step panicked is told.
pub(super) const STEP_PANICKED: &str =
    "the run engine failed on this request (a bug; see the daemon log); the runs are as they were";

/// Steps `event` on `state`. On a panic, `state` is put back as it was before the step
/// and the panic's message is returned; the step's effects are dropped with it. The
/// state is cloned first, as `step` itself clones every run to tell what changed.
pub(super) fn guarded_step(
    state: &mut EngineState,
    event: Event,
) -> Result<Vec<crate::run::engine::Effect>, String> {
    let before = state.clone();
    let taken = std::mem::take(state);
    match catch_unwind(AssertUnwindSafe(move || step(taken, event))) {
        Ok((next, fx)) => {
            *state = next;
            Ok(fx)
        }
        Err(panic) => {
            *state = before;
            Err(panic_text(panic.as_ref()))
        }
    }
}

fn panic_text(panic: &(dyn std::any::Any + Send)) -> String {
    if let Some(text) = panic.downcast_ref::<&str>() {
        (*text).to_string()
    } else if let Some(text) = panic.downcast_ref::<String>() {
        text.clone()
    } else {
        "a panic with no message".to_string()
    }
}

/// The request an event carries, whose waiting caller is answered when its step
/// panics.
pub(super) fn reply_of(kind: &EventKind) -> Option<ReplyId> {
    match kind {
        EventKind::Start { reply, .. }
        | EventKind::Approve { reply, .. }
        | EventKind::Reject { reply, .. }
        | EventKind::Edit { reply, .. }
        | EventKind::Retry { reply, .. }
        | EventKind::Override { reply, .. }
        | EventKind::Cancel { reply, .. }
        | EventKind::Resume { reply, .. }
        | EventKind::Finish { reply, .. }
        | EventKind::Tool { reply, .. } => Some(*reply),
        EventKind::BaseAdvanced { .. }
        | EventKind::OpDone { .. }
        | EventKind::Signal { .. }
        | EventKind::Delivered { .. }
        | EventKind::Restore { .. }
        | EventKind::Stop
        | EventKind::Tick => None,
    }
}
