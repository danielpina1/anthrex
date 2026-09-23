//! Decision 29's outbox: every engine message to an agent is queued on the run, and
//! delivered as one new turn only when the round's turn is closed, no interrupt is
//! pending and no rate-limit continue is being waited out. Pure (design decision 2).
//!
//! M8a.11 builds the queue, the gate for a live session and the retry delay after a
//! failed delivery. M8a.12 adds the rest of decision 29: the block after
//! `DELIVERY_MAX_FAILURES`, the wait for a failed turn's continue, and `ResumeSession`
//! for a round whose session has ended (and its result).

use proto::{AgentRole, BlockReason, TaskState};

use super::dispatch::{block, history};
use super::signals::end_round;
use super::{Effect, OpId, OpKind, OpResult, emit_op, next_op};
use crate::run::messages::{DELIVERY_MAX_FAILURES, DELIVERY_RETRY_SECS, join_turn};
use crate::run::model::{FailedTurn, FreshSession, Outgoing, Run, StallState};
use crate::run::role_launch::jitter_ms;

/// Queues `text` for task `task_id`'s current worker. The window is the worker round's
/// at queue time (0 when it has none yet); delivery always goes to the task's current
/// worker round.
pub(super) fn queue(run: &mut Run, task_id: &str, text: String, now: u64) {
    let window_id = run
        .task(task_id)
        .and_then(|t| t.rounds.iter().rev().find(|r| r.role == AgentRole::Worker))
        .and_then(|r| r.window_id)
        .unwrap_or(0);
    let id = run.next_message;
    run.next_message += 1;
    run.outbox.push(Outgoing {
        id,
        window_id,
        task_id: task_id.to_string(),
        text,
        queued_at: now,
        delivered_at: None,
    });
}

/// Emits one `Deliver` per task whose worker can take a turn now.
pub(super) fn deliver(run: &mut Run, now: u64, fx: &mut Vec<Effect>) {
    let mut tasks: Vec<String> = Vec::new();
    for message in run.outbox.iter().filter(|m| m.delivered_at.is_none()) {
        if !tasks.contains(&message.task_id) {
            tasks.push(message.task_id.clone());
        }
    }
    for task_id in tasks {
        let Some(i) = run.tasks.iter().position(|t| t.id() == task_id) else {
            continue;
        };
        // A blocked task holds its messages (M8a.6 ruling N5: an answer waits for new
        // dependencies); a finished one never takes a turn.
        if matches!(
            run.tasks[i].state,
            TaskState::Blocked | TaskState::Merged | TaskState::Cancelled
        ) {
            continue;
        }
        // Ruling T12-I3: a fresh session still to start takes the messages in its prompt.
        if run.tasks[i].fresh_session.is_some() {
            let texts: Vec<String> = run
                .outbox
                .iter()
                .filter(|m| m.task_id == task_id && m.delivered_at.is_none())
                .map(|m| m.text.clone())
                .collect();
            run.outbox
                .retain(|m| m.task_id != task_id || m.delivered_at.is_some());
            if let Some(fresh) = run.tasks[i].fresh_session.as_mut() {
                accumulate(fresh, texts);
            }
            continue;
        }
        let Some(r) = run.tasks[i]
            .rounds
            .iter()
            .rposition(|r| r.role == AgentRole::Worker)
        else {
            continue;
        };
        let round = &run.tasks[i].rounds[r];
        let rate_limited = round.rate_limited_until.is_some_and(|t| t > now);
        let interrupted = matches!(round.stall, StallState::Interrupted { .. });
        let retry_later = round.delivery_retry_at.is_some_and(|t| t > now);
        let continue_due = matches!(round.failed_turn, FailedTurn::WaitingContinue { .. });
        let Some(window_id) = round.window_id else {
            continue;
        };
        // An ended session is resumed with the messages (decision 28), unless the engine
        // ended it (a kill: a fresh session takes over) or it has no id to resume.
        let resumable = round.ended && !round.retiring && round.session_id.is_some();
        if round.turn_open
            || (round.ended && !resumable)
            || round.retiring
            || rate_limited
            || interrupted
            || retry_later
            || continue_due
        {
            continue;
        }
        let batch: Vec<&Outgoing> = run
            .outbox
            .iter()
            .filter(|m| m.task_id == task_id && m.delivered_at.is_none())
            .collect();
        let text = join_turn(&batch);
        let message_ids: Vec<u64> = batch.iter().map(|m| m.id).collect();
        for message in run.outbox.iter_mut() {
            if message_ids.contains(&message.id) {
                message.delivered_at = Some(now);
            }
        }
        let round = &mut run.tasks[i].rounds[r];
        round.turn_open = true;
        round.turns += 1;
        // The stall clock counts from the turn's start, not from the last turn's end.
        round.last_event = now;
        if resumable {
            round.ended = false;
            round.ended_at = None;
            round.carried = message_ids;
            let session_id = round.session_id.clone().unwrap_or_default();
            let session = run.tasks[i].session;
            let kind = OpKind::ResumeSession {
                window_id,
                session_id,
                message: text,
                jitter_ms: jitter_ms(&run.id, &task_id, session),
            };
            let op = next_op(run);
            run.tasks[i].rounds[r].resume_op = Some(op);
            emit_op(run, op, Some(&task_id), kind, fx);
            continue;
        }
        fx.push(Effect::Deliver {
            run_id: run.id.clone(),
            message_ids,
            window_id,
            text,
        });
    }
}

/// `Event::Delivered`: a delivered batch leaves the outbox; a failed one is queued again
/// with its turn closed and retried `DELIVERY_RETRY_SECS` later (decision 29; review
/// minor 6), and the `DELIVERY_MAX_FAILURES`th failure in a row blocks the task as
/// `blocked(environment)`. A superseded session's messages have left the outbox
/// (ruling T12-N), so its delivery's result finds nothing and changes nothing.
pub(super) fn delivered(
    run: &mut Run,
    message_ids: &[u64],
    ok: bool,
    error: Option<String>,
    now: u64,
) {
    let mut tasks = Vec::new();
    for message in run.outbox.iter_mut() {
        if message_ids.contains(&message.id) {
            message.delivered_at = None;
            tasks.push(message.task_id.clone());
        }
    }
    if ok {
        run.outbox.retain(|m| !message_ids.contains(&m.id));
    }
    for i in 0..run.tasks.len() {
        if !tasks.contains(&run.tasks[i].spec.id) {
            continue;
        }
        let Some(round) = run.tasks[i]
            .rounds
            .iter_mut()
            .rev()
            .find(|r| r.role == AgentRole::Worker)
        else {
            continue;
        };
        if ok {
            round.delivery_failures = 0;
            round.delivery_retry_at = None;
            continue;
        }
        round.turn_open = false;
        round.delivery_failures = round.delivery_failures.saturating_add(1);
        round.delivery_retry_at = Some(now + DELIVERY_RETRY_SECS);
        let blocks = round.delivery_failures >= DELIVERY_MAX_FAILURES;
        if blocks && !run.tasks[i].state.is_finished() {
            let error = error.clone().unwrap_or_else(|| "unknown error".into());
            let text = format!("could not deliver to the agent: {error}");
            block(run, i, BlockReason::Environment, text, now);
        }
    }
}

/// Ruling T12-I3: messages for a fresh session still to start join the end of its
/// prompt, in order, after any already there.
fn accumulate(fresh: &mut FreshSession, texts: Vec<String>) {
    if texts.is_empty() {
        return;
    }
    let mut all: Vec<String> = fresh.append.take().into_iter().collect();
    all.extend(texts);
    fresh.append = Some(all.join("\n\n"));
}

/// A `ResumeSession`'s result (decisions 28, 29, 32). Resumed: the messages it carried
/// are delivered. Failed: the task gets a fresh session at the same rung, with no
/// failure counted, whose prompt ends with those messages. Only the round that awaits
/// this resume takes its result (ruling T12-N).
pub(super) fn resumed(run: &mut Run, i: usize, op: OpId, result: OpResult, now: u64) {
    if run.tasks[i].state.is_finished() {
        return;
    }
    let Some(r) = run.tasks[i]
        .rounds
        .iter()
        .rposition(|r| r.role == AgentRole::Worker && r.resume_op == Some(op))
    else {
        return;
    };
    run.tasks[i].rounds[r].resume_op = None;
    let carried = std::mem::take(&mut run.tasks[i].rounds[r].carried);
    let texts: Vec<String> = run
        .outbox
        .iter()
        .filter(|m| carried.contains(&m.id))
        .map(|m| m.text.clone())
        .collect();
    run.outbox.retain(|m| !carried.contains(&m.id));
    let error = match result {
        OpResult::Resumed => {
            run.tasks[i].rounds[r].delivery_failures = 0;
            return;
        }
        OpResult::ResumeFailed { error } => error,
        OpResult::Failed { message } => message,
        _ => return,
    };
    // Ruling T12-I3: the round is over for good; nothing resumes it again.
    let round = &mut run.tasks[i].rounds[r];
    end_round(round, now);
    round.retiring = true;
    let fresh = run.tasks[i].fresh_session.get_or_insert(FreshSession {
        reason: format!("the session could not be resumed: {error}"),
        append: None,
    });
    accumulate(fresh, texts);
    history(run, i, now, format!("resume failed: {error}"));
}
