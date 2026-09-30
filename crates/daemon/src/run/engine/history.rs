//! M8b decisions 32 and 33, engine side: a merged task's diff is measured
//! (`MeasureDiff`), and every record of `history.jsonl` is appended as a journaled op
//! (`AppendHistory`). A merged or cancelled task gets its record at once, after its
//! diff; when the run ends, every task without one gets one; then the run's own
//! record, once every task line has come back. A run with no repository data
//! directory (restored from milestone 8a) writes none. Pure (design decision 2).

use proto::{HistoryLine, TaskOutcome, TaskState};

use super::{Effect, OpKind, OpResult, emit_op, next_op};
use crate::run::history::{due, enabled, outcome, run_record, run_record_due, task_record};
use crate::run::model::Run;

// Milestone 9 decision 43: the role-routing records of the orchestrator, sub-planners,
// run scouts and run-bound deciders.
#[path = "role_routes.rs"]
mod role_routes;
pub(super) use role_routes::{
    close, close_session, interrupt_open, keep, open, orchestrator_dispatched, orchestrator_ended,
    planner_accepted, planner_ended, session_stopped,
};

/// `<repo_dir>/history.jsonl` (M8b decision 4).
pub const HISTORY_FILE: &str = "history.jsonl";

/// Whether a `MeasureDiff` or `AppendHistory` of task `id` (`None`: of the run) is in
/// flight.
fn in_flight(run: &Run, id: Option<&str>, append_only: bool) -> bool {
    run.pending_ops.values().any(|p| {
        p.task_id.as_deref() == id
            && match p.kind {
                OpKind::AppendHistory { .. } => true,
                OpKind::MeasureDiff { .. } => !append_only,
                _ => false,
            }
    })
}

/// One line appended for `task` (`None`: the run's own).
fn append(run: &mut Run, task: Option<&str>, line: HistoryLine, fx: &mut Vec<Effect>) {
    let record_id = match &line {
        HistoryLine::Task(r) => r.record_id.clone(),
        HistoryLine::Run(r) => r.record_id.clone(),
        HistoryLine::Revert(r) => r.record_id.clone(),
        HistoryLine::RoleRoute(r) => r.record_id.clone(),
        HistoryLine::Tier(r) => r.record_id.clone(),
        HistoryLine::Flaky(r) => r.record_id.clone(),
        HistoryLine::Bisect(r) => r.record_id.clone(),
    };
    let kind = OpKind::AppendHistory {
        path: run.repo_dir.join(HISTORY_FILE),
        record_id,
        line: Box::new(line),
    };
    let op = next_op(run);
    emit_op(run, op, task, kind, fx);
}

/// What task `i` changed, `from` → `to` (decision 32).
fn measure(
    run: &mut Run,
    i: usize,
    (from, to): (String, String),
    three_dot: bool,
    fx: &mut Vec<Effect>,
) {
    let kind = OpKind::MeasureDiff {
        root: run.root.clone(),
        from,
        to,
        three_dot,
    };
    let id = run.tasks[i].id().to_string();
    let op = next_op(run);
    emit_op(run, op, Some(&id), kind, fx);
}

/// Task `i` merged at `commit` onto the run head `from` (decision 32): exactly what it
/// contributed is measured, and its record follows the answer.
pub(super) fn merged(run: &mut Run, i: usize, from: String, commit: String, fx: &mut Vec<Effect>) {
    if enabled(run) && !run.tasks[i].history_written {
        measure(run, i, (from, commit), false, fx);
    }
}

/// Task `i`'s record, emitted now.
fn record(run: &mut Run, i: usize, outcome: TaskOutcome, now: u64, fx: &mut Vec<Effect>) {
    let line = HistoryLine::Task(task_record(run, &run.tasks[i], outcome, now));
    run.tasks[i].history_written = true;
    let id = run.tasks[i].id().to_string();
    append(run, Some(&id), line, fx);
}

/// `MeasureDiff`'s result for task `i`: the diff is kept, and the record appended with
/// it, or without one when the measure failed.
pub(super) fn measured(run: &mut Run, i: usize, result: OpResult, now: u64, fx: &mut Vec<Effect>) {
    if let OpResult::DiffMeasured(stats) = result {
        run.tasks[i].diff = Some(stats);
    }
    if !run.tasks[i].history_written {
        let outcome = outcome(&run.tasks[i]);
        record(run, i, outcome, now, fx);
    }
}

/// Every step, for every run: the records that are due. A merged task waits for its
/// diff; a cancelled or unfinished one with a recorded head has it measured first
/// (from the run head, three dots); the run record waits until no task line is in
/// flight. A run that ended ends its orchestrator's record first (decision 43).
pub(super) fn pass(run: &mut Run, now: u64, fx: &mut Vec<Effect>) {
    role_routes::pass(run, fx);
    for (i, outcome) in due(run) {
        let task = &run.tasks[i];
        if in_flight(run, Some(task.id()), false) {
            continue;
        }
        let head = task
            .head
            .clone()
            .filter(|_| task.state != TaskState::Merged);
        match head {
            Some(head) if task.diff.is_none() => {
                let from = run.head_for(task).to_string();
                measure(run, i, (from, head), true, fx);
            }
            _ => record(run, i, outcome, now, fx),
        }
    }
    let Some(outcome) = run_record_due(run) else {
        return;
    };
    let tasks_in_flight = run.tasks.iter().any(|t| in_flight(run, Some(t.id()), true));
    if tasks_in_flight {
        return;
    }
    let line = HistoryLine::Run(run_record(run, outcome, now));
    run.run_record_written = true;
    append(run, None, line, fx);
}

/// `AppendHistory`'s result: a line that could not be written is lost, and said so in
/// the run's log (history is informational; nothing waits for it).
pub(super) fn appended(run: &mut Run, kind: &OpKind, result: OpResult, now: u64) {
    if let (OpResult::Failed { message }, OpKind::AppendHistory { record_id, .. }) = (result, kind)
    {
        let text = format!("history record {record_id} was not written: {message}");
        super::requests::log(run, now, text);
    }
}
