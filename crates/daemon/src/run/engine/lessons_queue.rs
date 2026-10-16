//! Milestone 9.15 decisions 9 and 10: when `history::record` emits a task line, or
//! `history::pass` the run line, of a qualifying outcome, the engine also emits one
//! `QueueDistill` op: a `queued` line for `<repo_dir>/lessons.jsonl`, its dossier built
//! and redacted here (`lessons::dossier::build`, ruling C4's frozen limits). Only a run
//! frozen with `limits.enabled && limits.distill` and a repository data directory
//! queues anything (ruling C2: a run from before lessons has none). The op is owned by
//! no task, so nothing waits for it; a failed write is logged and lost, as a history
//! line is. Pure (design decision 2): no lock, no I/O.

use proto::{DistillCause, LESSONS_VERSION, LessonLine, QueuedLine, TaskOutcome, TaskRecord};

use super::requests::log;
use super::{Effect, OpKind, OpResult, emit_op, next_op};
use crate::lessons::{LESSONS_FILE, dossier};
use crate::run::model::{LessonLimits, Run};

/// Decision 9's cause for a task record: the first that applies, or none.
pub(crate) fn task_cause(record: &TaskRecord) -> Option<DistillCause> {
    let merged = matches!(
        record.outcome,
        TaskOutcome::Merged | TaskOutcome::MergedWithoutApproval
    );
    let g = &record.gates;
    let failed = g.checks_failed + g.proofs_failed + g.reviews_rejected;
    let blocking = record.severities.critical + record.severities.important;
    match record.outcome {
        TaskOutcome::Blocked => Some(DistillCause::TaskBlocked),
        _ if merged && failed >= 1 => Some(DistillCause::TaskRecovered),
        _ if merged && blocking >= 1 => Some(DistillCause::ReviewBlocking),
        _ => None,
    }
}

/// Decision 9's cause for a run record's outcome.
pub(crate) fn run_cause(outcome: &str) -> Option<DistillCause> {
    match outcome {
        "discarded" => Some(DistillCause::RunDiscarded),
        "failed" => Some(DistillCause::RunFailed),
        _ => None,
    }
}

/// The run's frozen limits when it distills: lessons and distillation on, and a
/// repository data directory to write to.
fn distills(run: &Run) -> Option<LessonLimits> {
    let limits = &run.lessons.as_ref()?.limits;
    let on = limits.enabled && limits.distill && !run.repo_dir.as_os_str().is_empty();
    on.then(|| limits.clone())
}

/// Task `i`'s record was emitted, of `cause` ([`task_cause`]): its distill entry, when
/// it qualifies.
pub(super) fn task(
    run: &mut Run,
    i: usize,
    cause: Option<DistillCause>,
    now: u64,
    fx: &mut Vec<Effect>,
) {
    if let Some(cause) = cause {
        queue(run, Some(i), cause, now, fx);
    }
}

/// The run's record of outcome `outcome` was emitted: its distill entry, when it
/// qualifies.
pub(super) fn run_line(run: &mut Run, outcome: &str, now: u64, fx: &mut Vec<Effect>) {
    if let Some(cause) = run_cause(outcome) {
        queue(run, None, cause, now, fx);
    }
}

/// One `QueueDistill` for task `i` (`None`: the run), `<run>/<task>/distill` or
/// `<run>/run/distill`.
fn queue(run: &mut Run, i: Option<usize>, cause: DistillCause, now: u64, fx: &mut Vec<Effect>) {
    let Some(limits) = distills(run) else {
        return;
    };
    let task = i.map(|i| &run.tasks[i]);
    let (text, fps) = dossier::build(run, task, cause, &limits);
    let task_id = task.map(|t| t.id().to_string());
    let record_id = format!("{}/{}/distill", run.id, task_id.as_deref().unwrap_or("run"));
    let line = LessonLine::Queued(QueuedLine {
        v: LESSONS_VERSION,
        record_id: record_id.clone(),
        at: now,
        run_id: run.id.clone(),
        task_id,
        cause,
        dossier: text,
        fps,
    });
    let kind = OpKind::QueueDistill {
        path: run.repo_dir.join(LESSONS_FILE),
        record_id,
        line: Box::new(line),
    };
    let op = next_op(run);
    emit_op(run, op, None, kind, fx);
}

/// `QueueDistill`'s result: an entry that could not be written is lost, and said so in
/// the run's log (decision 10).
pub(super) fn queued(run: &mut Run, kind: &OpKind, result: OpResult, now: u64) {
    if let (OpResult::Failed { message }, OpKind::QueueDistill { record_id, .. }) = (result, kind) {
        let text = format!("lessons: the distill entry {record_id} was not written: {message}");
        log(run, now, text);
    }
}
