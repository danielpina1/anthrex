//! Milestone 9.6 ruling T8-6, part of `design_agents.rs`: a brainstorm round's input
//! pack is written once, by its first start (`driver/design_ops.rs`), and every later
//! start of the round sends that file, read back against what the first reported. Here
//! the engine records it ([`record`]), and a start whose pack could not be read back
//! halts the brainstorm, retryably ([`unreadable`]); `run resume` relaunches that
//! brainstormer, the clock still waiting for the drafts ([`awaiting_drafts`]). Pure
//! (design decision 2).

use proto::{AgentRole, RoleOutcome, RunState};

use super::super::{Effect, history};
use crate::run::design::pack::PackFile;
use crate::run::design::state::DesignAgentState;
use crate::run::model::Run;

/// The round's pack file, as its first start to report one wrote it; a later report is
/// the same file (the driver never replaces it), so it changes nothing.
pub(super) fn record(run: &mut Run, file: PackFile) {
    let frozen = (run.orch.design.as_mut()).and_then(|d| d.pack.as_mut());
    if let Some(frozen) = frozen.filter(|p| p.file.is_none()) {
        frozen.file = Some(file);
    }
}

/// `session`'s start found its round's pack missing or changed: its record fails,
/// brainstormer `k` (when it is still that start's) waits to start again, and the run
/// halts with the exact text, retryably, back to brainstorming on `run resume`.
pub(super) fn unreadable(
    run: &mut Run,
    (session, k): (&str, Option<usize>),
    reason: String,
    now: u64,
    fx: &mut Vec<Effect>,
) {
    let text = format!("design flow: the brainstorm's input pack could not be read back: {reason}");
    let failed = (RoleOutcome::Failed, Some(text.clone()));
    history::close_session(run, (AgentRole::Brainstormer, session), failed, fx);
    let Some(design) = run.orch.design.as_mut() else {
        return;
    };
    if let Some(agent) = k.map(|k| &mut design.brainstormers[k]) {
        agent.state = DesignAgentState::Queued;
        agent.window_id = None;
    }
    if run.state != RunState::Brainstorming {
        return;
    }
    design.halted_from = Some(RunState::Brainstorming);
    design.phase_started = None;
    super::super::merge::halt(run, text, now);
    run.halt_retryable = true;
}

/// A brainstorm round still waiting for its drafts, with a brainstormer queued to start:
/// `run resume` of its halt relaunches it, and the clock keeps waiting.
pub(in crate::run::engine) fn awaiting_drafts(run: &Run) -> bool {
    run.orch.design.as_ref().is_some_and(|d| {
        !d.drafts_settled && (d.brainstormers.iter()).any(|a| a.state == DesignAgentState::Queued)
    })
}
