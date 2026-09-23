//! Decision 29's outbox: every engine message to an agent is queued on the run, and
//! delivered as one new turn only when the round's turn is closed, no interrupt is
//! pending and no rate-limit continue is being waited out. Pure (design decision 2).
//!
//! M8a.11 builds the queue and the gate for a live session. M8a.12 adds the rest of
//! decision 29: a failed delivery's retry and block, and `ResumeSession` for a round
//! whose session has ended.

use proto::{AgentRole, TaskState};

use super::Effect;
use crate::run::messages::join_turn;
use crate::run::model::{Outgoing, Run, StallState};

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
        let Some(window_id) = round.window_id else {
            continue;
        };
        if round.turn_open || round.ended || round.retiring || rate_limited || interrupted {
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
        fx.push(Effect::Deliver {
            run_id: run.id.clone(),
            message_ids,
            window_id,
            text,
        });
    }
}

/// `Event::Delivered`: a delivered batch leaves the outbox; a failed one is queued
/// again with its turn closed (M8a.12 adds the retry delay and the block).
pub(super) fn delivered(run: &mut Run, message_ids: &[u64], ok: bool) {
    if ok {
        run.outbox.retain(|m| !message_ids.contains(&m.id));
        return;
    }
    let mut tasks = Vec::new();
    for message in run.outbox.iter_mut() {
        if message_ids.contains(&message.id) {
            message.delivered_at = None;
            tasks.push(message.task_id.clone());
        }
    }
    for task in run.tasks.iter_mut().filter(|t| tasks.contains(&t.spec.id)) {
        if let Some(round) = task
            .rounds
            .iter_mut()
            .rev()
            .find(|r| r.role == AgentRole::Worker)
        {
            round.turn_open = false;
        }
    }
}
