//! M8b decisions 32 and 33, engine side: a merged task's diff is measured
//! (`MeasureDiff`), and every record of `history.jsonl` is appended as a journaled op
//! (`AppendHistory`). A merged or cancelled task gets its record at once, after its
//! diff; when the run ends, every task without one gets one; then the run's own
//! record, once every task line has come back. A run with no repository data
//! directory (restored from milestone 8a) writes none. Milestone 9.1 decision 57 adds
//! a `tier` line per tier job, a `flaky` line per flake, and a `bisect` line per ended
//! bisect. Pure (design decision 2).

use proto::{
    BisectLine, FlakyRecord, HISTORY_VERSION, HistoryLine, TaskOutcome, TaskState, TierRunRecord,
};

use super::{Effect, OpId, OpKind, OpResult, emit_op, next_op};
use crate::run::history::{due, enabled, outcome, run_record, run_record_due, task_record};
use crate::run::model::{BisectRecord, Run};
use crate::run::tiers::{Affected, TierOutcome};

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
pub(super) fn append(run: &mut Run, task: Option<&str>, line: HistoryLine, fx: &mut Vec<Effect>) {
    let record_id = match &line {
        HistoryLine::Task(r) => r.record_id.clone(),
        HistoryLine::Run(r) => r.record_id.clone(),
        HistoryLine::Revert(r) => r.record_id.clone(),
        HistoryLine::RoleRoute(r) => r.record_id.clone(),
        HistoryLine::Tier(r) => r.record_id.clone(),
        HistoryLine::Flaky(r) => r.record_id.clone(),
        HistoryLine::Bisect(r) => r.record_id.clone(),
        HistoryLine::Stage(r) => r.record_id.clone(),
        HistoryLine::Round(r) => r.record_id.clone(),
        HistoryLine::Phase(r) => r.record_id.clone(),
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

/// Decision 57: tier job `op`'s `tier` line and one `flaky` line per test that passed
/// on its retry, for task `task` (`None`: a run-level tier 3) on stage `stage`. Each is
/// its own journaled `AppendHistory`, owned by no task, so a task's own record never
/// waits for them; `record_id` `<run>/tier/<op>` and `<run>/flaky/<op>/<test>`.
pub(super) fn tier_job(
    run: &mut Run,
    (op, task, stage): (OpId, Option<&str>, u16),
    outcome: &TierOutcome,
    now: u64,
    fx: &mut Vec<Effect>,
) {
    if !enabled(run) {
        return;
    }
    let mut flaky: Vec<String> = Vec::new();
    for name in outcome.steps.iter().flat_map(|s| s.flaky.iter()) {
        if !flaky.contains(name) {
            flaky.push(name.clone());
        }
    }
    let (affected, full_reason) = match &outcome.affected {
        Affected::Modules(names) => (u32::try_from(names.len()).unwrap_or(u32::MAX), None),
        Affected::Full(reason) => (0, Some(reason.clone())),
    };
    let count = |n: usize| u8::try_from(n).unwrap_or(u8::MAX);
    let cached_steps = count(outcome.steps.iter().filter(|s| s.cached).count());
    let run_id = run.id.clone();
    let line = HistoryLine::Tier(TierRunRecord {
        v: HISTORY_VERSION,
        record_id: format!("{run_id}/tier/{op}"),
        at: now,
        run_id: run_id.clone(),
        task_id: task.map(str::to_string),
        stage,
        tier: outcome.tier,
        secs: outcome.secs,
        affected,
        full_reason,
        cache_hit: cached_steps > 0,
        cached_steps,
        steps: count(outcome.steps.len()),
        ok: outcome.ok,
        flaky: flaky.clone(),
    });
    append(run, None, line, fx);
    for test in flaky {
        let line = HistoryLine::Flaky(FlakyRecord {
            v: HISTORY_VERSION,
            record_id: format!("{run_id}/flaky/{op}/{test}"),
            at: now,
            run_id: run_id.clone(),
            task_id: task.map(str::to_string),
            tier: outcome.tier,
            test,
        });
        append(run, None, line, fx);
    }
}

/// Decision 57: the `bisect` line of stage `stage`'s ended bisect `b`, the stage's
/// `seq`-th (`record_id` `<run>/bisect/<stage>/<seq>`): its range, probes and result,
/// a culprit with its fix task, or the reason there was none.
pub(super) fn bisect_ended(
    run: &mut Run,
    (stage, seq): (u16, u32),
    b: &BisectRecord,
    result: BisectResult<'_>,
    now: u64,
    fx: &mut Vec<Effect>,
) {
    if !enabled(run) {
        return;
    }
    let (culprit, fix_task, reason) = result.parts();
    let run_id = run.id.clone();
    let line = HistoryLine::Bisect(BisectLine {
        v: HISTORY_VERSION,
        record_id: format!("{run_id}/bisect/{stage}/{seq}"),
        at: now,
        run_id,
        stage,
        head: b.head.clone(),
        tests: b.tests.clone(),
        range: u32::try_from(b.candidates.len()).unwrap_or(u32::MAX),
        probes: b.probes,
        culprit: culprit.map(str::to_string),
        reason: reason.map(str::to_string),
        fix_task: fix_task.map(str::to_string),
    });
    append(run, None, line, fx);
}

/// How a bisect ended (decisions 36–38).
#[derive(Clone, Copy)]
pub(super) enum BisectResult<'a> {
    /// The culprit task, and the fix task added for it.
    Culprit { task: &'a str, fix: &'a str },
    /// A culprit whose fix task M8a's rules refused (decision 38's no single culprit).
    Refused { task: &'a str, reason: &'a str },
    /// No single culprit, for this reason.
    None(&'a str),
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

impl<'a> BisectResult<'a> {
    /// The culprit, its fix task, and the reason there was no single culprit.
    pub(super) fn parts(self) -> (Option<&'a str>, Option<&'a str>, Option<&'a str>) {
        match self {
            BisectResult::Culprit { task, fix } => (Some(task), Some(fix), None),
            BisectResult::Refused { task, reason } => (Some(task), None, Some(reason)),
            BisectResult::None(reason) => (None, None, Some(reason)),
        }
    }
}
