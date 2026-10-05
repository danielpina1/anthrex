//! Ruling WB-A-W1, part of `design_agents.rs`: while a run is halted or paused in a
//! design phase, a live design agent's one write is held, as ruling T8-1 holds drafts,
//! never refused. A brainstormer's draft joins the held drafts; a document reviewer's
//! findings wait in `DesignState::held_findings`. The agent has submitted, so its end
//! is no failure and its one relaunch is not spent. A resume applies them in order (the
//! findings, then the brainstorm settles); a discard drops them, and a restore, which
//! loses them with the old daemon's memory, relaunches their agents. Pure (design
//! decision 2).

use proto::{AgentRole, RunState};

use super::super::Effect;
use crate::run::model::Run;

/// The design phase a halted or paused run left, if it left one.
pub(in crate::run::engine) fn phase(run: &Run) -> Option<RunState> {
    let left = match run.state {
        RunState::Halted => run.orch.design.as_ref()?.halted_from,
        RunState::Paused => run.paused_from,
        _ => None,
    };
    left.filter(|s| {
        matches!(
            s,
            RunState::Brainstorming
                | RunState::Specifying
                | RunState::Planning
                | RunState::AwaitingApproval
        )
    })
}

/// `orch::tool`'s state gate: a design agent's call to a run halted or paused in a
/// design phase goes on to its own checks (its live window first).
pub(in crate::run::engine) fn takes(run: &Run, role: AgentRole) -> bool {
    matches!(role, AgentRole::Brainstormer | AgentRole::DocReviewer)
        && run.orch.design.is_some()
        && phase(run).is_some()
}

/// On resume: the held findings, then the held drafts (the brainstorm settles).
pub(in crate::run::engine) fn apply(run: &mut Run, now: u64, fx: &mut Vec<Effect>) {
    super::reviewer::apply_held(run, now);
    super::settle(run, now, fx);
}
