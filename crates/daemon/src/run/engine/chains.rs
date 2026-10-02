//! Milestone 9.3 decision 19: the chain table's transitions inside `step`. Pure (design
//! decision 2).
//!
//! A run that gets an orchestrator carries its chain (`requests::start`,
//! `promote::perform`); [`pass`], after every event, puts a new chain in the table,
//! follows its current run's window, makes it idle once that run is accepted or
//! discarded, and drops it once that run fails. [`window_gone`] ends an idle chain.

use super::EngineState;
use crate::run::chain::{Chain, ChainState, make_idle};
use crate::run::model::Run;
use proto::RunState;

/// A run that gets an orchestrator starts its own chain, unless it continues one.
pub(super) fn assign(run: &mut Run) {
    if run.orch.orchestrator.is_some() && run.chain.is_none() {
        run.chain = Some(crate::run::chain::chain_id(run));
    }
}

/// After every event: each chained, unfinished run's chain is in the table; each
/// chain follows its current run.
pub(super) fn pass(state: &mut EngineState) {
    for run in state.runs.values() {
        let Some(id) = run.chain.as_deref() else {
            continue;
        };
        if !run.state.is_terminal()
            && !state.chains.contains_key(id)
            && let Some(chain) = Chain::new(id.to_string(), run)
        {
            state.chains.insert(id.to_string(), chain);
        }
    }
    let mut idle = Vec::new();
    let mut failed = Vec::new();
    for chain in state.chains.values_mut() {
        let Some(run) = state.runs.get(chain.current()) else {
            continue;
        };
        if let Some(window) = run.orch.orchestrator.as_ref().and_then(|o| o.window_id) {
            chain.window_id = window;
        }
        match run.state {
            RunState::Failed => failed.push(chain.id.clone()),
            RunState::Accepted | RunState::Discarded if chain.state == ChainState::Active => {
                idle.push(chain.id.clone())
            }
            _ => {}
        }
    }
    for id in failed {
        state.chains.remove(&id);
    }
    for id in idle {
        make_idle(&mut state.chains, &id);
    }
}

/// `OrchEvent::ChainWindowGone`: the driver saw an idle chain's window close or exit
/// (KG §3.2). An active chain's window is decision 13's, so it is ignored here.
pub(super) fn window_gone(state: &mut EngineState, chain: &str, window_id: u32) {
    if let Some(c) = state.chains.get_mut(chain)
        && c.state == ChainState::Idle
        && c.window_id == window_id
    {
        c.ended = true;
    }
}
