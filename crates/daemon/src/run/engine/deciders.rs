//! Deciders inside a run (M8b decisions 18, 20 and 21). The reducer never calls a
//! decider: it queues a request, and [`dispatch`] starts queued ones as `Decide` ops in
//! free reader slots, before any reviewer; the driver answers each with `Decided`. A
//! decider still queued `decider_slot_wait_secs` after it was queued is answered by its
//! fallback, and with the deciders off every request is answered by its fallback at
//! once, with no op. A decider only summarises output or classifies a block: it never
//! approves, merges or changes what a task must meet.
//!
//! Three deciders so far: the check summary (decision 20), for which a failed check's
//! rung waits ([`summarise`]), the classification of a `task_blocked` with no kind
//! (decision 21, [`classify`]), and the size cross-check (decision 19, [`cross_check`],
//! in `deciders_size.rs`). Pure (design decision 2): no `std::fs`,
//! `std::process`, `std::thread`, `tokio` or `std::time::SystemTime`.

use proto::{BlockReason, DeciderMode, DeciderSource, GateKind, RunState, TaskState};

use super::dispatch::{block, history};
use super::schedule::readers_busy;
use super::{Effect, OpKind, OpResult, done, emit_op, ladder, next_op};
use crate::decider::fallback::{OFF_REASON, fallback_decision};
use crate::decider::{
    BlockKind, BlockedReasonInput, CheckSummaryInput, DeciderAnswer, DeciderRequest, Decision,
};
use crate::run::contract::{candidate_red_message, check_failed_message};
use crate::run::model::{PendingFailure, QueuedDecider, Run};

pub(super) use super::deciders_size::cross_check;

/// Queues `request` for `task_ids` (the task the answer is for comes first); its id.
pub(crate) fn queue(
    run: &mut Run,
    task_ids: Vec<String>,
    request: DeciderRequest,
    now: u64,
) -> u64 {
    let decider_id = run.next_decider;
    run.next_decider += 1;
    run.decider_queue.push(QueuedDecider {
        decider_id,
        task_ids,
        request,
        queued_at: now,
    });
    decider_id
}

/// Decision 18: with the deciders off, every queued request is answered by its fallback
/// at once (no call, so nothing is counted). Otherwise, while the run runs, queued
/// deciders start oldest first into free reader slots, and those that waited
/// `decider_slot_wait_secs` for one are answered by their fallback.
pub(crate) fn dispatch(run: &mut Run, now: u64) -> Vec<Effect> {
    let mut fx = Vec::new();
    if run.decider_queue.is_empty() || run.state.is_terminal() {
        return fx;
    }
    if run.limits.decider_mode == DeciderMode::Off {
        for q in std::mem::take(&mut run.decider_queue) {
            let decision = fallback_decision(&q.request, OFF_REASON.to_string());
            apply(
                run,
                q.decider_id,
                &q.task_ids,
                &q.request,
                decision,
                now,
                &mut fx,
            );
        }
        return fx;
    }
    if run.state != RunState::Running {
        return fx;
    }
    while !run.decider_queue.is_empty() && readers_busy(run) < usize::from(run.limits.max_readers) {
        let q = run.decider_queue.remove(0);
        let op = next_op(run);
        let task = q.task_ids.first().cloned();
        let kind = OpKind::Decide {
            decider_id: q.decider_id,
            task_ids: q.task_ids,
            request: q.request,
        };
        emit_op(run, op, task.as_deref(), kind, &mut fx);
    }
    let wait = run.limits.decider_slot_wait_secs;
    let (late, waiting): (Vec<_>, Vec<_>) = std::mem::take(&mut run.decider_queue)
        .into_iter()
        .partition(|q| now >= q.queued_at.saturating_add(wait));
    run.decider_queue = waiting;
    for q in late {
        let reason = format!("no reader slot was free within {wait} s");
        let decision = fallback_decision(&q.request, reason);
        fx.extend(on_decided(
            run,
            q.decider_id,
            &q.task_ids,
            &q.request,
            decision,
            now,
        ));
    }
    fx
}

/// A decider's answer (decision 18): counted on the run (a fallback too), its usage
/// added to the run and, when it was asked about one task, to that task; then applied.
pub(crate) fn on_decided(
    run: &mut Run,
    decider_id: u64,
    task_ids: &[String],
    request: &DeciderRequest,
    decision: Decision,
    now: u64,
) -> Vec<Effect> {
    // Review m4: an answer of another kind than its request is the request's fallback,
    // so whatever waits for it is never stuck. Its turn's usage is kept.
    let decision = if answers(&decision.answer, request) {
        decision
    } else {
        Decision {
            usage: decision.usage,
            secs: decision.secs,
            ..fallback_decision(request, MISMATCH.to_string())
        }
    };
    run.decider_calls = run.decider_calls.saturating_add(1);
    if decision.source == DeciderSource::Fallback {
        run.decider_fallbacks = run.decider_fallbacks.saturating_add(1);
    }
    if let Some(usage) = decision.usage {
        run.decider_usage += usage;
        if let [id] = task_ids
            && let Some(task) = run.tasks.iter_mut().find(|t| t.id() == id)
        {
            task.decider_usage += usage;
        }
    }
    let mut fx = Vec::new();
    apply(run, decider_id, task_ids, request, decision, now, &mut fx);
    fx
}

/// The fallback reason for an answer of another kind than its request (review m4).
const MISMATCH: &str = "the decider's answer does not match its request";

/// Whether `answer` is of `request`'s kind.
fn answers(answer: &DeciderAnswer, request: &DeciderRequest) -> bool {
    matches!(
        (answer, request),
        (DeciderAnswer::Triage(_), DeciderRequest::Triage(_))
            | (DeciderAnswer::SizeCheck(_), DeciderRequest::SizeCheck(_))
            | (
                DeciderAnswer::CheckSummary { .. },
                DeciderRequest::CheckSummary(_)
            )
            | (
                DeciderAnswer::BlockedReason { .. },
                DeciderRequest::BlockedReason(_)
            )
    )
}

/// Drops the queued deciders asked about task `id` (review m3: a cancelled task's). A
/// queued size check asked about several tasks only loses `id` (M8b.13), and goes
/// when no task is left. One in flight runs to its end; its answer finds nothing
/// waiting.
pub(crate) fn drop_queued(queue: &mut Vec<QueuedDecider>, id: &str) {
    for q in queue.iter_mut() {
        if let DeciderRequest::SizeCheck(input) = &mut q.request {
            input.tasks.retain(|t| t.id != id);
            q.task_ids.retain(|t| t != id);
        }
    }
    queue.retain(|q| q.task_ids.first().is_some_and(|first| first != id));
}

/// `Decide`'s result. A `Failed` (the driver always answers `Decided`) is the
/// fallback, so a decider never blocks a run.
pub(super) fn op_done(
    run: &mut Run,
    kind: &OpKind,
    result: OpResult,
    now: u64,
    fx: &mut Vec<Effect>,
) {
    let OpKind::Decide {
        decider_id,
        task_ids,
        request,
    } = kind
    else {
        return;
    };
    let decision = match result {
        OpResult::Decided(decision) => *decision,
        OpResult::Failed { message } => {
            let reason = format!("the decider could not start: {message}");
            fallback_decision(request, reason)
        }
        _ => return,
    };
    fx.extend(on_decided(
        run,
        *decider_id,
        task_ids,
        request,
        decision,
        now,
    ));
}

/// Decision 20: task `i`'s check (the last record) failed at `gate` (`Check`, or
/// `Merge` for a red candidate). The failure is counted on the ladder now; its rung
/// waits for the summary, and the task keeps its state meanwhile.
pub(super) fn summarise(
    run: &mut Run,
    i: usize,
    gate: GateKind,
    command: &str,
    now: u64,
    fx: &mut Vec<Effect>,
) {
    let rung = ladder::count_failure(run, i, gate);
    let task = &run.tasks[i];
    let Some(check_index) = task.checks.len().checked_sub(1) else {
        return;
    };
    let record = &task.checks[check_index];
    let task_id = task.id().to_string();
    let request = DeciderRequest::CheckSummary(CheckSummaryInput {
        task_id: task_id.clone(),
        command: command.to_string(),
        code: record.code,
        timed_out: record.timed_out,
        tail: record.tail.clone(),
    });
    let decider_id = queue(run, vec![task_id], request, now);
    run.tasks[i].pending_failure = Some(PendingFailure {
        gate,
        rung,
        check_index,
        decider_id,
    });
    fx.extend(dispatch(run, now));
}

/// Decision 21: task `i`, just blocked as a question on `reason` with no kind, is
/// classified. Whether a decider was asked (with the deciders off, the fallback's
/// `question` stands at once).
pub(super) fn classify(
    run: &mut Run,
    i: usize,
    reason: String,
    now: u64,
    fx: &mut Vec<Effect>,
) -> bool {
    let task = &run.tasks[i];
    let task_id = task.id().to_string();
    let request = DeciderRequest::BlockedReason(BlockedReasonInput {
        task_id: task_id.clone(),
        title: task.spec.title.clone(),
        reason,
    });
    let decider_id = queue(run, vec![task_id], request, now);
    run.tasks[i].pending_classification = Some(decider_id);
    fx.extend(dispatch(run, now));
    run.tasks[i].pending_classification.is_some()
}

/// Applies `decision` to the task it is for. With the deciders off nothing is noted, so
/// a run without deciders keeps M8a's history.
fn apply(
    run: &mut Run,
    decider_id: u64,
    task_ids: &[String],
    request: &DeciderRequest,
    decision: Decision,
    now: u64,
    fx: &mut Vec<Effect>,
) {
    // Decision 19: a size check is for every task it asked about.
    if let (DeciderAnswer::SizeCheck(verdicts), DeciderRequest::SizeCheck(input)) =
        (&decision.answer, request)
    {
        return super::deciders_size::sized(run, decider_id, input, verdicts, &decision, now);
    }
    // Milestone 9.1 decision 37: a bisect's summary is for its stage, not a task.
    if let DeciderAnswer::CheckSummary { lines } = &decision.answer
        && super::bisect::summarised(run, decider_id, lines)
    {
        return;
    }
    let Some(i) = task_ids
        .first()
        .and_then(|id| run.tasks.iter().position(|t| t.id() == id))
    else {
        return;
    };
    let note = match decision.fallback_reason.as_deref() {
        Some(OFF_REASON) => None,
        Some(reason) => Some(format!("fallback ({reason})")),
        None => Some("decider".to_string()),
    };
    match (&decision.answer, request) {
        (DeciderAnswer::CheckSummary { lines }, DeciderRequest::CheckSummary(input)) => {
            let summary = (decision.source == DeciderSource::Decider).then(|| lines.join("\n"));
            summarised(
                run,
                i,
                decider_id,
                input,
                (summary, decision.source),
                note,
                now,
                fx,
            );
        }
        (DeciderAnswer::BlockedReason { kind, .. }, DeciderRequest::BlockedReason(input)) => {
            classified(
                run,
                i,
                decider_id,
                input,
                *kind,
                decision.source,
                note,
                now,
                fx,
            );
        }
        // `on_decided` made the kinds match; the size check is applied above, and
        // triage by its own caller (M8b.14).
        _ => {}
    }
}

/// Decision 20: the summary goes on the check record, then the deferred rung is taken
/// if the task still waits in its gate (a cancel, say, has ended the wait).
#[allow(clippy::too_many_arguments)]
fn summarised(
    run: &mut Run,
    i: usize,
    decider_id: u64,
    input: &CheckSummaryInput,
    (summary, source): (Option<String>, DeciderSource),
    note: Option<String>,
    now: u64,
    fx: &mut Vec<Effect>,
) {
    let task = &mut run.tasks[i];
    let Some(pending) = task.pending_failure.take_if(|p| p.decider_id == decider_id) else {
        return;
    };
    let Some(record) = task.checks.get_mut(pending.check_index) else {
        return;
    };
    record.summary = summary;
    record.summary_source = Some(source);
    let record = record.clone();
    if let Some(note) = note {
        history(run, i, now, format!("check summary: {note}"));
    }
    let task = &run.tasks[i];
    let waiting = match pending.gate {
        GateKind::Merge => {
            task.state == TaskState::MergeQueue
                && task.merge_op.is_none()
                && !run.merge_queue.iter().any(|q| q == task.id())
        }
        _ => task.state == TaskState::Check && task.gate_op.is_none(),
    };
    if !waiting {
        return;
    }
    let text = match pending.gate {
        GateKind::Merge => candidate_red_message(&input.command, &record),
        _ => check_failed_message(&input.command, &record),
    };
    ladder::take_rung(run, i, pending.gate, pending.rung, text, false, now, fx);
}

/// Decision 21: a block still waiting for this decider's classification (review I1: a
/// retry, an override or a typed block ends the wait), as the worker gave it,
/// becomes what the decider judged: a question stays one, an environment problem
/// changes the reason, and mis-sized is rung 3 as a typed `mis_sized` is.
#[allow(clippy::too_many_arguments)]
fn classified(
    run: &mut Run,
    i: usize,
    decider_id: u64,
    input: &BlockedReasonInput,
    kind: BlockKind,
    source: DeciderSource,
    note: Option<String>,
    now: u64,
    fx: &mut Vec<Effect>,
) {
    let task = &mut run.tasks[i];
    let waiting = task.pending_classification == Some(decider_id)
        && task.state == TaskState::Blocked
        && task
            .block
            .as_ref()
            .is_some_and(|b| b.reason == BlockReason::Question && b.text == input.reason);
    if !waiting {
        return;
    }
    task.pending_classification = None;
    task.block_source = Some(source);
    let label = match kind {
        BlockKind::Question => "question",
        BlockKind::MisSized => "mis_sized",
        BlockKind::Environment => "environment",
    };
    if let Some(note) = note {
        history(run, i, now, format!("block classified as {label}: {note}"));
    }
    match kind {
        BlockKind::Question => {}
        BlockKind::Environment => {
            block(run, i, BlockReason::Environment, input.reason.clone(), now)
        }
        BlockKind::MisSized => done::mis_sized(run, i, &input.reason, now, fx),
    }
}
