//! Milestone 9.6 ruling T8-7, part of `design_agents.rs`: a Codex design agent's
//! session is never resumed, so a turn that ends without its submission is not nudged
//! (`scout::machine`'s `unsubmitted` failure, `ScoutEnd::Unsubmitted`). A brainstormer
//! is relaunched fresh once, as a new session with the same written pack, and the
//! document reviewer (task M9.6.10) with the same draft, each with the line
//! `scout::design_spec::resubmit_line` adds to its first turn; a second such end fails
//! it, `ended twice without submitting`, and the brainstorm or the review goes on as for
//! any failure. Pure (design decision 2).

use super::super::Effect;
use super::super::requests::log;
use super::fail;
use crate::run::design::state::DesignAgentState;
use crate::run::model::Run;

/// The reason a design agent fails when its one relaunch also ended without submitting.
pub const ENDED_TWICE: &str = "ended twice without submitting";

/// Which design agent ended: brainstormer `k`, or the document reviewer (task M9.6.10).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Agent {
    Brainstormer(usize),
    Reviewer,
}

/// `agent`'s turn ended without its submission, unnudged: queued again for its one
/// relaunch (`DesignAgent.unsubmitted`), or failed when that was it.
pub(super) fn once(run: &mut Run, agent: Agent, now: u64, fx: &mut Vec<Effect>) {
    let Some(design) = run.orch.design.as_mut() else {
        return;
    };
    let (noun, found) = match agent {
        Agent::Brainstormer(k) => ("brainstormer", design.brainstormers.get_mut(k)),
        Agent::Reviewer => ("document reviewer", design.reviewer.as_mut()),
    };
    let Some(found) = found else {
        return;
    };
    if found.unsubmitted {
        let reason = ENDED_TWICE.to_string();
        return match agent {
            Agent::Brainstormer(k) => fail(run, k, reason, now, fx),
            Agent::Reviewer => super::reviewer::fail(run, reason, now),
        };
    }
    found.unsubmitted = true;
    found.state = DesignAgentState::Queued;
    found.window_id = None;
    let text = format!(
        "{noun} {} ended its turn without submitting; it relaunches fresh once",
        found.label
    );
    log(run, now, text);
}
