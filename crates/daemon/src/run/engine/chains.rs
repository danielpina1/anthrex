//! Milestone 9.3 decision 19: the chain table's transitions inside `step`. Pure (design
//! decision 2).
//!
//! A run that gets an orchestrator carries its chain (`requests::start`,
//! `promote::perform`); [`pass`], after every event, puts a new chain in the table,
//! follows its current run's window, makes it idle once that run is finished (accepted,
//! discarded, or delivered, D17), makes it active again when that run iterates (D17),
//! and drops it once that run fails. [`window_gone`] ends an idle chain.
//! Task 6b: a continued run [`join`]s its idle chain, adopting its window (decision
//! 23) or, once the chain has ended, with a fresh session (decision 24).

use super::requests::log;
use super::{Effect, EngineState};
use crate::run::chain::{Chain, ChainState, finished, make_idle, newest, outcome};
use crate::run::model::Run;
use crate::run::orch::contract_rounds::{next_goal_wake, no_chain_to_continue, still_going};
use proto::RunState;

/// How a run started by `requests::start` joins a chain.
#[derive(Debug, PartialEq, Eq)]
pub(super) enum Join {
    /// Not a continued run: [`pass`] starts its own chain.
    New,
    /// It adopted its idle chain's window (decision 23): no launch.
    Adopted,
    /// Its chain's session ended: it launches a fresh one (decision 24).
    Fresh,
}

/// Decisions 23 and 24, in `requests::start`, for a run that arrived with its chain
/// set (`continued`, a next goal; `chain_goal.rs` checked the chain first): the chain
/// is checked again, so a second continue that was in flight is refused (KG §3.5), and
/// the run joins it. An idle chain's live window is adopted: the run takes its
/// predecessor's session (route, window, OTLP token, session and routing), marked
/// live; its next-goal wake waits on it, round 1's request (decision 11's identity);
/// and `Effect::AdoptOrchestrator` renames and rebinds the window. An ended chain's run
/// launches as any planned run, with the first prompt the driver set. D17: the chain's
/// previous run records the new run (`Run.continued_by`, so it iterates no more), and
/// its orchestrator record is no longer live (a delivered run has not ended, so nothing
/// else releases it).
pub(super) fn join(
    state: &mut EngineState,
    run: &mut Run,
    continued: bool,
    now: u64,
    fx: &mut Vec<Effect>,
) -> Result<Join, String> {
    let Some(id) = run.chain.clone().filter(|_| continued) else {
        return Ok(Join::New);
    };
    let h4 = |runs: &std::collections::BTreeMap<String, Run>, id: &str| {
        runs.get(id)
            .map_or(id.to_string(), |r| r.short().to_string())
    };
    let Some(chain) = state.chains.get(&id) else {
        let last = newest(&state.runs, &id, None).map_or(id.clone(), |r| r.short().into());
        return Err(no_chain_to_continue(&last));
    };
    if chain.state == ChainState::Active {
        return Err(still_going(&h4(&state.runs, chain.current())));
    }
    let window_id = chain.window_id;
    let prev = state
        .runs
        .get(chain.current())
        .filter(|_| !chain.ended && window_id != 0);
    let prev_id = chain.current().to_string();
    let join = match prev {
        Some(prev) => {
            adopt(run, prev, window_id, now, fx);
            Join::Adopted
        }
        None => Join::Fresh,
    };
    if let Some(prev) = state.runs.get_mut(&prev_id) {
        prev.continued_by = Some(run.id.clone());
        if let Some(o) = prev.orch.orchestrator.as_mut() {
            o.live = false;
        }
    }
    if let Some(chain) = state.chains.get_mut(&id) {
        chain.runs.push(run.id.clone());
        chain.state = ChainState::Active;
        chain.ended = false;
        if join == Join::Fresh {
            chain.window_id = 0;
        }
    }
    Ok(join)
}

/// Decision 23: `run` takes `prev`'s orchestrator session in `window_id`.
fn adopt(run: &mut Run, prev: &Run, window_id: u32, now: u64, fx: &mut Vec<Effect>) {
    if let (Some(o), Some(p)) = (
        run.orch.orchestrator.as_mut(),
        prev.orch.orchestrator.as_ref(),
    ) {
        o.route = p.route.clone();
        o.window_id = Some(window_id);
        o.otlp_token = p.otlp_token.clone();
        o.session = p.session;
        o.routing = p.routing.clone();
        o.launches = p.launches;
        o.first_prompt = p.first_prompt.clone();
        o.live = true;
        o.exited_at = None;
    }
    let ended = (prev.short(), outcome(prev));
    run.orch.request_wake = Some(next_goal_wake(run.short(), ended, &run.goal));
    log(
        run,
        now,
        format!(
            "the orchestrator of run {} continues in window {window_id}",
            prev.short()
        ),
    );
    fx.push(Effect::AdoptOrchestrator {
        run_id: run.id.clone(),
        window_id,
        name: format!("{}/orchestrator", run.short()),
    });
}

/// Task 6b fix round 1 (m1), `OrchEvent::AdoptLost`: run `run_id` never took window
/// `window_id` (gone before the driver rebound it, or still the previous run's after a
/// restart). Its orchestrator keeps no window and is not live, with the driver's
/// handoff as its first prompt, so `run resume` launches a fresh session
/// (`orch_window::relaunch`); the chain follows no window until then. A report for
/// another window, or for a run that has ended, changes nothing.
pub(super) fn adopt_lost(
    state: &mut EngineState,
    run_id: &str,
    window_id: u32,
    first_prompt: String,
    now: u64,
) {
    let Some(run) = state
        .runs
        .get_mut(run_id)
        .filter(|r| !r.state.is_terminal())
    else {
        return;
    };
    let Some(o) = run
        .orch
        .orchestrator
        .as_mut()
        .filter(|o| o.window_id == Some(window_id))
    else {
        return;
    };
    o.window_id = None;
    o.live = false;
    o.first_prompt = first_prompt;
    let text = format!(
        "window {window_id} was gone before run {} took it; run resume launches a fresh session",
        run.short()
    );
    log(run, now, text);
    if let Some(chain) = run.chain.as_deref().and_then(|id| state.chains.get_mut(id))
        && chain.current() == run_id
    {
        chain.window_id = 0;
    }
}

/// A run that gets an orchestrator starts its own chain, unless it continues one.
pub(super) fn assign(run: &mut Run) {
    if run.orch.orchestrator.is_some() && run.chain.is_none() {
        run.chain = Some(crate::run::chain::chain_id(run));
    }
}

/// After every event: each chained run that is not finished (not terminal, and not
/// delivered, D17) has its chain in the table, so a chain that left it (the project's
/// older idle chain, task 6b fix round 3) stays out until its run iterates, when it
/// returns active with that run alone; each chain follows its current run.
pub(super) fn pass(state: &mut EngineState) {
    for run in state.runs.values() {
        let Some(id) = run.chain.as_deref() else {
            continue;
        };
        if !run.state.is_terminal()
            && !finished(run)
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
            _ if finished(run) && chain.state == ChainState::Active => idle.push(chain.id.clone()),
            // D17: an idle chain's delivered run iterated (back in `planning`): the
            // chain is active again, its session the run's own.
            _ if !finished(run) && !run.state.is_terminal() && chain.state == ChainState::Idle => {
                chain.state = ChainState::Active;
                chain.ended = false;
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
