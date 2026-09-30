//! Milestone 9 decisions 35–38, engine side: research and review tasks, and the
//! per-epic integration review. Pure (design decision 2).
//!
//! A **research** task runs a read-only scout session (`run::orch::launch::
//! research_spec`) in a reader slot, with no worktree and no branch: its round's role is
//! `Scout`, its mailbox `<task>.research`. Its `submit_scout_report` ends it `reported`
//! with the report kept on the task; its session's rules are `research.rs`'s. A **review** task resolves its target (`OpKind::ResolveTarget`),
//! then runs M8a's review round (`review.rs`) on that range: its first verdict, whatever
//! it is, ends it `reported`, and nothing is merged or sent back. An **integration
//! review** is a review task the engine adds itself once an epic's work is merged
//! (decision 37). Research and review tasks never merge anything, integration reviews
//! never approve anything but their epic's integration state, and completion is
//! decision 38's (`may_complete`); both are `integration.rs`'s.

use proto::{
    BlockReason, IntegrationState, RunState, Severity, Size, TaskKind, TaskState, Verdict,
};

use super::dispatch::{block, history};
pub(super) use super::integration::{engine_owned, integration_pass, may_complete, record_merge};
use super::requests::log;
pub(super) use super::research::{
    mailbox_task, research_round, resumed, stop_research, submit_research, watch,
};
use super::schedule::{
    dispatch_order, hub_holds_slot, is_reader_task, op_in_flight, readers_busy, size_check_pending,
};
use super::{Effect, OpId, OpKind, OpResult, emit_op, gate_holds, next_op, wake};
use crate::run::contract::sha7;
use crate::run::model::{ReviewLevel, Run, Task};
use crate::run::orch::contract::{integration_review_prompt, review_task_prompt};
use crate::run::phases::set_state;

pub(super) fn is_integration(task: &Task) -> bool {
    task.orch.integration_of.is_some()
}

/// Decision 31's reader-slot order: integration reviews go with the reviewers
/// (`integration`), research and review tasks after the sub-planners and run scouts.
/// Runs only while the run runs, as every task dispatch does, and not beside a hub
/// task holding its writer slot.
pub(super) fn dispatch(run: &mut Run, now: u64, integration: bool, fx: &mut Vec<Effect>) {
    if run.state != RunState::Running || hub_holds_slot(run) {
        return;
    }
    for i in dispatch_order(run) {
        if readers_busy(run) >= usize::from(run.limits.max_readers) {
            break;
        }
        let task = &run.tasks[i];
        if !is_reader_task(task) || is_integration(task) != integration {
            continue;
        }
        // A review task whose resolved range was lost (a restart) resolves it again.
        let unresolved = task.state == TaskState::Review
            && task.orch.review_range.is_none()
            && task.gate_op.is_none()
            && !op_in_flight(run, task.id(), |_| true);
        let runnable = task.state == TaskState::Queued
            && !size_check_pending(task)
            && gate_holds::released(run, task);
        if !runnable && !unresolved {
            continue;
        }
        match task.spec.kind {
            TaskKind::Research => super::research::launch(run, i, now, fx),
            _ => resolve(run, i, now, fx),
        }
    }
}

/// Decision 36: a review task takes its reader slot and resolves its target first.
fn resolve(run: &mut Run, i: usize, now: u64, fx: &mut Vec<Effect>) {
    let op = next_op(run);
    let task = &run.tasks[i];
    let kind = OpKind::ResolveTarget {
        root: run.root.clone(),
        target: task.spec.review_target.clone().unwrap_or_default(),
        base_branch: run.base_branch.clone(),
    };
    let id = task.id().to_string();
    set_state(&mut run.tasks[i], TaskState::Review, now);
    run.tasks[i].gate_op = Some(op);
    history(run, i, now, "dispatched: resolving its review target");
    emit_op(run, op, Some(&id), kind, fx);
}

/// `ResolveTarget`'s result: the range is kept and its review prepared, or the task is
/// `blocked(environment)` with git's text. A launch the restart lost resolves again.
pub(super) fn target_done(
    run: &mut Run,
    i: usize,
    op: OpId,
    result: OpResult,
    now: u64,
    fx: &mut Vec<Effect>,
) {
    if run.tasks[i].gate_op != Some(op) {
        return;
    }
    run.tasks[i].gate_op = None;
    if run.tasks[i].state != TaskState::Review {
        return;
    }
    match result {
        OpResult::Target { base, head } => {
            let text = format!("review target resolved to {}..{}", sha7(&base), sha7(&head));
            run.tasks[i].orch.review_range = Some((base.clone(), head.clone()));
            history(run, i, now, text);
            prepare(run, i, (base, head), fx);
        }
        OpResult::Failed { message } => {
            let target = run.tasks[i].spec.review_target.clone().unwrap_or_default();
            let text = format!("review target {target} does not resolve: {message}");
            block(run, i, BlockReason::Environment, text, now);
        }
        _ => {
            set_state(&mut run.tasks[i], TaskState::Queued, now);
            history(run, i, now, "its review target is resolved again");
        }
    }
}

/// M8a's `PrepareReview` of a review task's resolved range; the review round follows
/// from its result (`review::review_ready`).
fn prepare(run: &mut Run, i: usize, (base, head): (String, String), fx: &mut Vec<Effect>) {
    let op = next_op(run);
    let id = run.tasks[i].id().to_string();
    let kind = OpKind::PrepareReview {
        root: run.root.clone(),
        head_ref: head,
        base_ref: base,
        path: run.review_path(&id),
        base_tree: None,
    };
    run.tasks[i].gate_op = Some(op);
    emit_op(run, op, Some(&id), kind, fx);
}

/// A review task's `PrepareReview` refs (`review::dispatch_reviewers`): its resolved
/// `(head, base)`, or `None` until it is resolved.
pub(super) fn review_refs(task: &Task) -> Option<(String, String)> {
    task.orch
        .review_range
        .clone()
        .map(|(base, head)| (head, base))
}

/// A review task's reviewer (decisions 36, 37): the author and level it is recorded
/// with and the route it runs on. The task's own route, which policy filled as for any
/// task and the engine set for an integration review; `small` for an S task, `medium`
/// for any other, `frontier` for an integration review.
pub(super) fn review_route(task: &Task) -> (proto::Route, ReviewLevel, proto::Route) {
    let level = if is_integration(task) {
        ReviewLevel::Frontier
    } else if task.size == Size::S {
        ReviewLevel::Small
    } else {
        ReviewLevel::Medium
    };
    (task.route.clone(), level, task.route.clone())
}

/// A review task's first turn: `integration_review_prompt` for an integration review,
/// else `review_task_prompt`.
pub(super) fn review_first_turn(
    run: &Run,
    i: usize,
    (base, head, patch): (&str, &str, &str),
) -> String {
    let task = &run.tasks[i];
    let Some(epic) = task.orch.integration_of.as_deref() else {
        return review_task_prompt(run, task, base, head, patch);
    };
    let Some(record) = run.orch.epics.iter().find(|e| e.epic == epic) else {
        return review_task_prompt(run, task, base, head, patch);
    };
    // Its round: the `n` of its id `<e>-int<n>`, as its title says (M9.9 review
    // fixes, M3).
    let round = super::integration::round_of(task).unwrap_or(1);
    integration_review_prompt(run, record, round, base, head)
}

/// Decision 36: a review task's first verdict was recorded (`review::submit`): it is
/// `reported`, whatever the verdict. An integration review sets its epic's state and
/// tells the orchestrator (decisions 37, 39).
pub(super) fn reviewed(run: &mut Run, i: usize, now: u64) {
    let task = &mut run.tasks[i];
    let Some(review) = task.reviews.iter().rfind(|r| r.verdict.is_some()).cloned() else {
        return;
    };
    set_state(task, TaskState::Reported, now);
    task.block = None;
    let verdict = if review.verdict == Some(Verdict::Approve) {
        "approve"
    } else {
        "changes"
    };
    history(
        run,
        i,
        now,
        format!("reported: its review's verdict is {verdict}"),
    );
    let Some(epic) = run.tasks[i].orch.integration_of.clone() else {
        return;
    };
    let count = |s: Severity| review.findings.iter().filter(|f| f.severity == s).count();
    let (critical, important) = (count(Severity::Critical), count(Severity::Important));
    let blocking = critical + important > 0;
    if let Some(record) = run.orch.epics.iter_mut().find(|e| e.epic == epic) {
        record.integration_state = if blocking {
            IntegrationState::Changes
        } else {
            IntegrationState::Approved
        };
    }
    let outcome = if blocking {
        format!("changes ({critical} critical, {important} important)")
    } else {
        "approve".to_string()
    };
    let text = format!("integration review of epic {epic}: {outcome}");
    log(run, now, text.clone());
    wake::note(run, text);
}
