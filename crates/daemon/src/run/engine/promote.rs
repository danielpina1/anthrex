//! Milestone 9 decision 29: `run promote` gives a fast-path run an orchestrator. M8b
//! decision 25 only recorded the wish (`engine/requests.rs`); this performs it, and
//! performs a wish recorded before milestone 9 when the run is next resumed or, while
//! it runs, on the first tick. The fast-path task keeps running through its gates; what
//! the orchestrator adds waits under hold `promotion` (`gate_holds.rs`). Pure (design
//! decision 2).

use proto::{OrchestratorChoice, RunPath, RunState};

use super::requests::log;
use super::{Effect, EngineState, ReplyId, orch_window};
use crate::run::model::Run;
use crate::run::orch::OrchestratorRecord;
use crate::run::orch::contract::promoted_first_prompt;
use crate::run::orch::installed::resolve_promoted;
use crate::run::orch::roles::RoleSnapshot;

/// `EventKind::Promote`.
pub(super) fn request(
    state: &mut EngineState,
    reply: ReplyId,
    run_id: &str,
    choice: Option<OrchestratorChoice>,
    now: u64,
    fx: &mut Vec<Effect>,
) {
    let result = promote(state, run_id, choice.as_ref(), now, fx);
    fx.push(Effect::Reply { reply, result });
}

fn promote(
    state: &mut EngineState,
    run_id: &str,
    choice: Option<&OrchestratorChoice>,
    now: u64,
    fx: &mut Vec<Effect>,
) -> Result<String, String> {
    let Some(run) = state.runs.get_mut(run_id) else {
        return Err(format!("unknown run {run_id}"));
    };
    // M8c.1 review: the repeat reply carries no time (the user reads times locally).
    if run.promote_requested_at.is_some() && run.orch.orchestrator.is_some() {
        return Ok(format!("run {run_id} was already marked for promotion"));
    }
    if let Some(text) = super::actions::rules::promote(run) {
        return Err(text);
    }
    perform(run, choice, now, fx)?;
    Ok(format!(
        "promoted: run {run_id} now has an orchestrator; it starts in a moment (anthrex run status {run_id})"
    ))
}

/// A fast-path run whose promotion was recorded before milestone 9 and not performed.
fn recorded_before_m9(run: &Run) -> bool {
    run.path == Some(RunPath::Fast)
        && run.promote_requested_at.is_some()
        && run.orch.orchestrator.is_none()
        && !run.state.is_terminal()
        && run.state != RunState::Complete
}

/// A running run's recorded promotion, on the tick.
pub(super) fn on_tick(state: &mut EngineState, now: u64, fx: &mut Vec<Effect>) {
    for run in state.runs.values_mut() {
        if run.state == RunState::Running && recorded_before_m9(run) {
            // The run's own frozen `[orchestrator.agent]` and roster decide the route;
            // with no explicit choice, a run from before milestone 9 always resolves.
            let _ = perform(run, None, now, fx);
        }
    }
}

/// A resumed run's recorded promotion.
pub(super) fn on_resume(run: &mut Run, now: u64, fx: &mut Vec<Effect>) {
    if recorded_before_m9(run) {
        let _ = perform(run, None, now, fx);
    }
}

/// Decision 29: the route (decision 6, from the run's frozen `[orchestrator.agent]`
/// and roster), the planned path, the orchestrator record with `plan_submitted` false,
/// and its `CreateOrchestrator`.
fn perform(
    run: &mut Run,
    choice: Option<&OrchestratorChoice>,
    now: u64,
    fx: &mut Vec<Effect>,
) -> Result<(), String> {
    // Milestone 9.5 rulings RH-5, RL-3: the choice, then the list, then decision 6.
    let resolved = resolve_promoted(run, choice, &run.orch.installed)?;
    run.path = Some(RunPath::Plan);
    run.promote_requested_at.get_or_insert(now);
    let mut record = OrchestratorRecord::new(resolved.route, now);
    record.routing = RoleSnapshot {
        source: resolved.source,
        candidates: resolved.candidates,
    };
    run.orch.orchestrator = Some(record);
    super::chains::assign(run);
    let first = promoted_first_prompt(run);
    if let Some(o) = run.orch.orchestrator.as_mut() {
        o.first_prompt = first;
    }
    log(
        run,
        now,
        "promoted to a planned run; its orchestrator starts",
    );
    orch_window::launch(run, now, fx);
    Ok(())
}
