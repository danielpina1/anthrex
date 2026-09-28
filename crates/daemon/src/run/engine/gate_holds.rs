//! Milestone 9 decision 28's approval holds: work the orchestrator adds after the plan
//! gate waits for the user's approval, without blocking a tool call or the running
//! work. Not `holds.rs`, which is M8a ruling N5's dependency hold (`awaiting_deps`):
//! nothing here reads or changes that. Pure (design decision 2).
//!
//! A hold is `promotion` (decision 29: everything a promoted run's orchestrator adds
//! before it submits) or `epic:<e>` (a new epic of a running run). A task whose hold is
//! not `Approved` is not runnable and is not pre-warmed (`dispatch.rs`).

use proto::{HoldKind, HoldState, RunState};

use super::complete::cancel_now;
use super::requests::log;
use super::{Effect, EngineState, ReplyId};
use crate::run::model::{Run, Task};
use crate::run::orch::json::label;
use crate::run::orch::{EditSource, GateHoldRecord};

/// Decision 29's hold id.
pub(super) const PROMOTION: &str = "promotion";

/// Decision 28: the task has no approval hold, or its hold is approved.
pub(super) fn released(run: &Run, task: &Task) -> bool {
    task.orch.gate_hold.as_ref().is_none_or(|id| {
        run.orch
            .gate_holds
            .iter()
            .any(|h| &h.id == id && h.state == HoldState::Approved)
    })
}

/// The run's plan was approved once (by the user, `--yes` or the fast path): work added
/// from now on is past the gate.
fn past_gate(run: &Run) -> bool {
    run.approved_at.is_some()
}

/// Gives each task of `added` (a batch's new tasks) the hold it waits under, if any,
/// and returns the first such hold. Only the orchestrator's and sub-planners' tasks are
/// held; a user's own edit is theirs. Before a promoted run's orchestrator submits,
/// every task waits under `promotion`; after it, a task of an epic whose hold is still
/// undecided waits under that hold.
pub(super) fn assign(
    run: &mut Run,
    added: &[String],
    source: &EditSource,
    now: u64,
) -> Option<String> {
    if *source == EditSource::User || !past_gate(run) {
        return None;
    }
    let submitted = run.orch.orchestrator.as_ref()?.plan_submitted;
    let mut first = None;
    for id in added {
        let hold = if !submitted {
            Some(ensure(run, PROMOTION, HoldKind::Promotion, now))
        } else {
            epic_hold(run, id)
        };
        let Some(hold) = hold else {
            continue;
        };
        if let Some(record) = run.orch.gate_holds.iter_mut().find(|h| h.id == hold) {
            record.tasks.push(id.clone());
        }
        if let Some(task) = run.tasks.iter_mut().find(|t| t.id() == id) {
            task.orch.gate_hold = Some(hold.clone());
        }
        first.get_or_insert(hold);
    }
    first
}

/// The undecided hold of the epic task `id` belongs to.
fn epic_hold(run: &Run, id: &str) -> Option<String> {
    let epic = run.task(id)?.spec.epic.as_deref()?;
    let hold = run
        .orch
        .epics
        .iter()
        .find(|e| e.epic == epic)?
        .gate_hold
        .clone()?;
    let undecided = |h: &&GateHoldRecord| {
        h.id == hold && matches!(h.state, HoldState::Drafting | HoldState::Awaiting)
    };
    run.orch
        .gate_holds
        .iter()
        .find(undecided)
        .map(|h| h.id.clone())
}

/// The hold `id`, created `Drafting` when the run has none by that id.
fn ensure(run: &mut Run, id: &str, kind: HoldKind, now: u64) -> String {
    if !run.orch.gate_holds.iter().any(|h| h.id == id) {
        run.orch.gate_holds.push(GateHoldRecord {
            id: id.to_string(),
            kind,
            state: HoldState::Drafting,
            tasks: Vec::new(),
            created_at: now,
            decided_at: None,
            decided_by: None,
        });
        log(run, now, format!("hold {id} created"));
    }
    id.to_string()
}

/// Decision 28: a new epic of a running run, whose plan was submitted, gets hold
/// `epic:<e>`, `Drafting` until its sub-planner's epic is accepted.
pub(super) fn create_epic_hold(run: &mut Run, epic: &str, now: u64) -> Option<String> {
    let submitted = run.orch.orchestrator.as_ref()?.plan_submitted;
    if !past_gate(run) || !submitted {
        return None;
    }
    let id = ensure(
        run,
        &format!("epic:{epic}"),
        HoldKind::Epic { epic: epic.into() },
        now,
    );
    if let Some(record) = run.orch.epics.iter_mut().find(|e| e.epic == epic) {
        record.gate_hold = Some(id.clone());
    }
    Some(id)
}

/// A drafted hold is submitted (the orchestrator's `submit`, a sub-planner's accepted
/// epic): it awaits the user, or is approved at once for a run started with `--yes`.
pub(super) fn submitted(run: &mut Run, id: &str, now: u64) {
    let yes = run.orch.yes;
    let Some(hold) = run
        .orch
        .gate_holds
        .iter_mut()
        .find(|h| h.id == id && h.state == HoldState::Drafting)
    else {
        return;
    };
    if yes {
        hold.state = HoldState::Approved;
        hold.decided_at = Some(now);
        hold.decided_by = Some("--yes".into());
        log(run, now, format!("hold {id} approved by --yes"));
    } else {
        hold.state = HoldState::Awaiting;
        log(run, now, format!("hold {id} awaits approval"));
    }
}

/// `run approve --hold` (`approve`) or `run reject --hold`: an awaiting hold is
/// approved, and its tasks become runnable, or rejected, and its tasks are cancelled
/// (none has started; dependents become `blocked(dep_cancelled)`, decision 13).
pub(super) fn verdict(
    state: &mut EngineState,
    reply: ReplyId,
    (run_id, id): (&str, &str),
    approve: bool,
    now: u64,
    fx: &mut Vec<Effect>,
) {
    let result = decide(state, (run_id, id), approve, now, fx);
    fx.push(Effect::Reply { reply, result });
}

fn decide(
    state: &mut EngineState,
    (run_id, id): (&str, &str),
    approve: bool,
    now: u64,
    fx: &mut Vec<Effect>,
) -> Result<String, String> {
    let Some(run) = state.runs.get_mut(run_id) else {
        return Err(format!("unknown run {run_id}"));
    };
    if run.state.is_terminal() || run.state == RunState::Complete {
        return Err(format!("run {run_id} is {}", run.state.label()));
    }
    let Some(hold) = run.orch.gate_holds.iter_mut().find(|h| h.id == id) else {
        return Err(format!("run {run_id} has no hold {id}"));
    };
    if hold.state != HoldState::Awaiting {
        return Err(format!("hold {id} is {}", label(&hold.state)));
    }
    hold.decided_at = Some(now);
    hold.decided_by = Some("user".into());
    let tasks = hold.tasks.clone();
    if approve {
        hold.state = HoldState::Approved;
        log(run, now, format!("hold {id} approved by the user"));
        let n = tasks.len();
        return Ok(format!(
            "hold {id} of run {run_id} approved: {n} task{} may start",
            plural(n)
        ));
    }
    hold.state = HoldState::Rejected;
    let mut cancelled = 0;
    for task in &tasks {
        let Some(i) = run.tasks.iter().position(|t| t.id() == task) else {
            continue;
        };
        if run.tasks[i].state.is_finished() {
            continue;
        }
        cancel_now(run, i, &format!("its hold {id} was rejected"), now, fx);
        cancelled += 1;
    }
    log(run, now, format!("hold {id} rejected by the user"));
    Ok(format!(
        "hold {id} of run {run_id} rejected: {cancelled} task{} cancelled",
        plural(cancelled)
    ))
}

fn plural(n: usize) -> &'static str {
    if n == 1 { "" } else { "s" }
}
