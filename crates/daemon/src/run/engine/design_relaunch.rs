//! Milestone 9.6 ruling T8-7, part of `design_agents.rs`: a Codex design agent's
//! session is never resumed, so a turn that ends without its submission is not nudged
//! (`scout::machine`'s `unsubmitted` failure). The brainstormer is relaunched fresh
//! once, as a new session with the same written pack and the line
//! `scout::design_spec::resubmit_line` adds to its first turn; a second such end fails
//! it, `ended twice without submitting`, and the brainstorm goes on as for any failure.
//! Pure (design decision 2).

use super::super::Effect;
use super::super::requests::log;
use super::fail;
use crate::run::design::state::DesignAgentState;
use crate::run::model::Run;

/// The reason a brainstormer fails when its one relaunch also ended without submitting.
pub const ENDED_TWICE: &str = "ended twice without submitting";

/// Brainstormer `k`'s turn ended without its draft, unnudged: queued again for its one
/// relaunch (`DesignAgent.unsubmitted`), or failed when that was it.
pub(super) fn once(run: &mut Run, k: usize, now: u64, fx: &mut Vec<Effect>) {
    let Some(agent) = run.orch.design.as_mut().map(|d| &mut d.brainstormers[k]) else {
        return;
    };
    if agent.unsubmitted {
        return fail(run, k, ENDED_TWICE.to_string(), now, fx);
    }
    agent.unsubmitted = true;
    agent.state = DesignAgentState::Queued;
    agent.window_id = None;
    let text = format!(
        "brainstormer {} ended its turn without submitting; it relaunches fresh once",
        agent.label
    );
    log(run, now, text);
}
