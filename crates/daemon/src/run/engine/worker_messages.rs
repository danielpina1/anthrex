//! Milestone 9 decision 42, engine side (task M9.13a): a worker message's delivery
//! through M8a's turn-boundary outbox (decision 29) and the record of what reached the
//! worker (decision 42d), the refresh's hand-back at the turn boundary and its result
//! (decision 42e), and a worker's `task_note` (decision 42f). The `message` and
//! `refresh` edits themselves are `run/edits_orch.rs`. No message path interrupts a
//! turn: everything here waits for the outbox's gate. Pure (design decision 2).

use proto::{AgentRole, TaskNoteKind, TaskState, ToolCall};

use super::dispatch::history;
use super::ladder::{live, worker_round};
use super::{Effect, OpId, OpKind, OpResult, ReplyId, emit_op, next_op, outbox, wake};
use crate::run::edits_orch::release;
use crate::run::edits_state::is_paused;
use crate::run::model::{Run, Task};
use crate::run::orch::contract::{
    NOTE_LIMIT, NOTE_RECORDED, PAUSE_RELEASED, refresh_clean, refresh_conflict,
};
use crate::run::orch::json::label;
use crate::run::orch::tools::{OrchCall, parse_call};
use crate::run::orch::{RefreshState, WorkerNote, add_worker_note};

/// Queues a message's `text` for task `task_id`'s worker in the outbox, linked to the
/// task's last recorded message not yet linked (the one the edit just recorded), so
/// its delivery marks that message delivered.
pub(super) fn queue(run: &mut Run, task_id: &str, text: String, now: u64) {
    let id = run.next_message;
    outbox::queue(run, task_id, text, now);
    let message = run
        .tasks
        .iter_mut()
        .find(|t| t.id() == task_id)
        .and_then(|t| {
            t.orch
                .messages
                .iter_mut()
                .rev()
                .find(|m| !m.delivered && m.outbox.is_none())
        });
    if let Some(message) = message {
        message.outbox = Some(id);
    }
}

/// The outbox messages `ids` reached their worker: the recorded messages they carried
/// are delivered (decision 42b's rate limit counts the others).
pub(super) fn delivered(run: &mut Run, ids: &[u64]) {
    for task in &mut run.tasks {
        for message in &mut task.orch.messages {
            if message.outbox.is_some_and(|id| ids.contains(&id)) {
                message.delivered = true;
            }
        }
    }
}

/// Whether outbox message `id` carries a recorded message: a fresh session's first
/// turn has those in its notes section (decision 42d), not appended again.
pub(super) fn carries(run: &Run, id: u64) -> bool {
    run.tasks
        .iter()
        .flat_map(|t| t.orch.messages.iter())
        .any(|m| m.outbox == Some(id))
}

/// A new worker session of task `i`, whose first turn lists every recorded message
/// (decision 42d): each is delivered, and the outbox drops its copies not yet sent.
pub(super) fn launched(run: &mut Run, i: usize) {
    let task = &mut run.tasks[i];
    let ids: Vec<u64> = task.orch.messages.iter().filter_map(|m| m.outbox).collect();
    for message in &mut task.orch.messages {
        message.delivered = true;
    }
    run.outbox
        .retain(|m| !(ids.contains(&m.id) && m.delivered_at.is_none()));
}

/// Decision 42c: `run retry` and `run override` of a paused task.
pub(super) fn paused_refusal(task: &Task) -> Option<String> {
    is_paused(task).then(|| {
        format!(
            "task {} is paused(message); send it a message to resume it",
            task.id()
        )
    })
}

/// Decision 42c: the run-wide `resume` plan edit releases every paused task, whose
/// worker is told to go on as its next turn.
pub(super) fn release_all(run: &mut Run, now: u64) {
    for i in 0..run.tasks.len() {
        if is_paused(&run.tasks[i]) {
            release(&mut run.tasks[i], now);
            let id = run.tasks[i].id().to_string();
            outbox::queue(run, &id, PAUSE_RELEASED.to_string(), now);
        }
    }
}

/// Decision 42e: a task's commits since it started, less the merge commits its
/// refreshes made. A refresh merge alone is never work.
pub(super) fn own_commits(task: &Task, count: u32) -> u32 {
    let merges = u32::try_from(task.orch.refresh_merges.len()).unwrap_or(u32::MAX);
    count.saturating_sub(merges)
}

/// Decision 42e: the outbox holds a task's mail while its refresh is due or in flight,
/// so the refresh's result and a message sent in the next call go out as one turn.
pub(super) fn holds_mail(task: &Task) -> bool {
    task.orch.refresh.is_some()
}

/// Decision 42e: at its worker's turn boundary, each due refresh becomes M8a's
/// `HandBack` of the run head with `list_merged`. A finished task's refresh is dropped.
pub(super) fn refresh_pass(run: &mut Run, now: u64, fx: &mut Vec<Effect>) {
    for i in 0..run.tasks.len() {
        if run.tasks[i].orch.refresh != Some(RefreshState::Due) {
            continue;
        }
        if run.tasks[i].state.is_finished() {
            run.tasks[i].orch.refresh = None;
            history(run, i, now, "refresh dropped: the task finished");
            continue;
        }
        if !at_boundary(run, i) {
            continue;
        }
        let task = &run.tasks[i];
        let kind = OpKind::HandBack {
            worktree: task.worktree.clone(),
            run_head: run.run_head.clone(),
            task_head: None,
            list_merged: true,
        };
        let id = task.id().to_string();
        let op = next_op(run);
        run.tasks[i].orch.refresh = Some(RefreshState::InFlight(op));
        emit_op(run, op, Some(&id), kind, fx);
    }
}

/// The worker round's turn is closed, nothing is being delivered, resumed or counted,
/// and no claim, gate or other hand-back is in flight.
fn at_boundary(run: &Run, i: usize) -> bool {
    let task = &run.tasks[i];
    let steady = (task.state == TaskState::Working || is_paused(task))
        && task.claim.is_none()
        && task.gate_op.is_none()
        && task.merge_op.is_none()
        && !task.awaiting_deps
        && !task.resolving;
    let Some(r) = worker_round(task).filter(|_| steady) else {
        return false;
    };
    let round = &task.rounds[r];
    let sending = run
        .outbox
        .iter()
        .any(|m| m.task_id == task.id() && m.delivered_at.is_some());
    let hand_back = run.pending_ops.values().any(|p| {
        p.task_id.as_deref() == Some(task.id()) && matches!(p.kind, OpKind::HandBack { .. })
    });
    round.window_id.is_some()
        && !round.turn_open
        && !round.retiring
        && round.resume_op.is_none()
        && round.count_op.is_none()
        && !sending
        && !hand_back
}

/// Whether task `i`'s refresh awaits `op`'s result.
pub(super) fn awaits_refresh(run: &Run, i: usize, op: OpId) -> bool {
    run.tasks[i].orch.refresh == Some(RefreshState::InFlight(op))
}

/// Decision 42e's result. Clean: the worker is told what its branch now has (every
/// merged commit, so `refresh_clean`'s count is the list's length), and the merge
/// commit is recorded so it never counts as the task's work. Up to date: nothing is
/// sent. Conflict: the worker resolves it (`resolving`, as M8a ruling N5's hand-back);
/// `Task.conflicts` is not touched. Failed, a dirty tree included: no block, a history
/// line, a wake note, and the error on the refresh's edit-log entry.
pub(super) fn refreshed(run: &mut Run, i: usize, result: OpResult, now: u64) {
    run.tasks[i].orch.refresh = None;
    if run.tasks[i].state.is_finished() {
        return;
    }
    let id = run.tasks[i].id().to_string();
    match result {
        OpResult::HandedBack { files, .. } if !files.is_empty() => {
            run.tasks[i].resolving = true;
            history(
                run,
                i,
                now,
                format!("refresh conflicted in: {}", files.join(", ")),
            );
            outbox::queue(run, &id, refresh_conflict(&files), now);
        }
        OpResult::HandedBack {
            head: Some(head),
            onto,
            merged,
            ..
        } if onto.as_ref() != Some(&head) => {
            run.tasks[i].orch.refresh_merges.push(head);
            let list: Vec<(String, String)> = merged
                .iter()
                .map(|line| match line.split_once(' ') {
                    Some((sha, subject)) => (sha.to_string(), subject.to_string()),
                    None => (line.clone(), String::new()),
                })
                .collect();
            let n = list.len();
            history(run, i, now, format!("refreshed: merged {n} commits"));
            if n >= 1 {
                outbox::queue(run, &id, refresh_clean(n, &list), now);
            }
        }
        OpResult::HandedBack { .. } => history(run, i, now, "refresh: nothing new"),
        OpResult::Failed { message } => {
            history(run, i, now, format!("refresh skipped: {message}"));
            wake::note(run, format!("refresh of {id} skipped: {message}"));
            crate::run::edit_log::set_error(run, &format!("refresh {id}"), &message);
        }
        _ => {}
    }
}

/// Decision 42f: a worker's `task_note`, from its task's current live worker round in
/// any unfinished state, `paused(message)` included. It never changes the task's
/// state; `discovery` and `risk` wake the orchestrator. Stored through
/// `add_worker_note` (M9.6 review, M-2).
pub(super) fn task_note(
    run: &mut Run,
    reply: ReplyId,
    call: &ToolCall,
    now: u64,
    fx: &mut Vec<Effect>,
) {
    let answer = |fx: &mut Vec<Effect>, result| fx.push(Effect::Reply { reply, result });
    let task_id = call.task_id.clone().unwrap_or_default();
    let found = run.tasks.iter().position(|t| t.id() == task_id);
    let current = found.is_some_and(|i| {
        let task = &run.tasks[i];
        call.role == AgentRole::Worker
            && worker_round(task).is_some_and(|r| {
                let round = &task.rounds[r];
                live(round) && round.window_id == Some(call.window_id)
            })
    });
    let (Some(i), true) = (found, current) else {
        let text = format!("this window is not the current worker of task {task_id}");
        return answer(fx, Err(text));
    };
    let task = &run.tasks[i];
    if task.state.is_finished() {
        let text = format!(
            "task_note is accepted only while the task is unfinished (it is {})",
            task.state.label()
        );
        return answer(fx, Err(text));
    }
    let (kind, text) = match parse_call(AgentRole::Worker, &call.tool, &call.args) {
        Ok(OrchCall::TaskNote { kind, text }) => (kind, text),
        Ok(_) => return answer(fx, Err("invalid arguments".into())),
        Err(e) => return answer(fx, Err(format!("invalid arguments: {e}"))),
    };
    let limit = run.limits.orch.note_max_per_task as usize;
    if task.orch.worker_notes.len() >= limit {
        return answer(fx, Err(NOTE_LIMIT.into()));
    }
    let kind_label = label(&kind);
    let first: String = text.chars().take(120).collect();
    let note = WorkerNote {
        at: now,
        kind,
        text,
        seq: 0,
    };
    add_worker_note(run, &task_id, note);
    history(run, i, now, format!("note ({kind_label}): {first}"));
    if kind != TaskNoteKind::Progress {
        wake::note(run, format!("{task_id} noted a {kind_label}: {first}"));
    }
    answer(fx, Ok(NOTE_RECORDED.into()))
}
