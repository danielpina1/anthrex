//! Decision 36's hand-back, engine side (M8a.14): the `HandBack` result and the due
//! hand-backs of ruling T14-I3. Pure (design decision 2).

use crate::run::phases::set_state;
use proto::{BlockReason, TaskState};

use super::ResolutionAt;
use super::dispatch::{block, history};
use super::merge::awaits;
use super::{Effect, OpId, OpKind, OpResult, emit_op, gates, ladder, next_op};
use crate::run::contract::{UNCLAIMED_COMMITS, conflict_message};
use crate::run::model::Run;

/// The merge queue's `HandBack` result (decision 36, ruling T14-C1). Only a merge made
/// onto the claimed commit (`onto == task.head`) skips the gates: clean, the merged
/// head re-queues at once; conflicted, the worker resolves it and its next accepted
/// `task_done` goes straight back to the queue. A merge made onto a later tip (the
/// worker committed after its claim) sends the task back to work, and its next claim
/// passes every gate. The due hand-back of ruling T14-I3 (`gates_after_handback`)
/// sends a clean head through the gates too. The task cannot be held meanwhile (it is
/// not `blocked`, so it gains no dependency: M8a.6 ruling N5 holds).
pub(super) fn handed_back(
    run: &mut Run,
    i: usize,
    op: OpId,
    run_head: &str,
    result: OpResult,
    now: u64,
    _fx: &mut Vec<Effect>,
) {
    if !awaits(run, i, op) {
        return;
    }
    run.tasks[i].merge_op = None;
    let gates_after = std::mem::take(&mut run.tasks[i].gates_after_handback);
    if run.tasks[i].state != TaskState::MergeQueue {
        return;
    }
    let (files, head, onto) = match result {
        OpResult::HandedBack {
            files, head, onto, ..
        } => (files, head, onto),
        OpResult::Failed { message } => {
            let text = format!("could not merge the run head into its worktree: {message}");
            return block(run, i, BlockReason::Environment, text, now);
        }
        _ => return,
    };
    let claimed = onto.is_some() && onto == run.tasks[i].head;
    let id = run.tasks[i].id().to_string();
    if files.is_empty() && claimed {
        if let Some(head) = head {
            run.tasks[i].head = Some(head);
        }
        if gates_after {
            let next = gates::next_gate(run, i, None);
            gates::enter(run, i, next, now);
            let text = format!("the run head merged cleanly; next: {}", next.label());
            return history(run, i, now, text);
        }
        gates::enter(run, i, TaskState::MergeQueue, now);
        return history(
            run,
            i,
            now,
            "the run head merged cleanly; back in the merge queue",
        );
    }
    let task = &mut run.tasks[i];
    set_state(task, TaskState::Working, now);
    ladder::reopen_stopped(task);
    task.handed_back = claimed && !gates_after;
    // Ruling T14-R2: the claim that resolves this conflict is checked against it.
    task.resolution = task.handed_back.then(|| ResolutionAt {
        onto: onto.clone().unwrap_or_default(),
        run_head: run_head.to_string(),
        files: files.clone(),
    });
    // As at rung 1: the time the merge took is not the worker's silence.
    if let Some(r) = ladder::worker_round(task) {
        task.rounds[r].last_event = now;
    }
    if files.is_empty() {
        super::outbox::queue(run, &id, UNCLAIMED_COMMITS.to_string(), now);
        let text = "the run head merged onto commits after its claim; back to work";
        return history(run, i, now, text);
    }
    run.tasks[i].resolving = true;
    super::outbox::queue(run, &id, conflict_message(&files), now);
    history(run, i, now, "handed back with conflicts to resolve");
}

/// Ruling T14-I3: the task's dependencies finished while its worker resolved a told
/// conflict, so the run head is handed back now that its claim was accepted, before
/// any gate. The task waits in `merge_queue` (out of the queue) for the result.
pub(super) fn hand_back_due(run: &mut Run, i: usize, now: u64) {
    let task = &mut run.tasks[i];
    task.handed_back = false;
    task.resolution = None;
    task.gates_after_handback = true;
    set_state(task, TaskState::MergeQueue, now);
    task.gate_op = None;
    history(
        run,
        i,
        now,
        "the run head its dependencies left is handed back first",
    );
}

/// Ruling T14-R2 (N3): each running pass sends the due hand-back of a task whose claim
/// was accepted (`hand_back_due`), never while the run is halted.
pub(super) fn start_due_hand_backs(run: &mut Run, now: u64, fx: &mut Vec<Effect>) {
    for i in 0..run.tasks.len() {
        let task = &run.tasks[i];
        if task.state == TaskState::MergeQueue && task.handback_due {
            send_due(run, i, now, fx);
        }
    }
}

fn send_due(run: &mut Run, i: usize, now: u64, fx: &mut Vec<Effect>) {
    let task = &mut run.tasks[i];
    task.handback_due = false;
    let id = task.id().to_string();
    let task = &run.tasks[i];
    let kind = OpKind::HandBack {
        worktree: task.worktree.clone(),
        run_head: run.head_for(task).to_string(),
        task_head: task.head.clone(),
        list_merged: false,
    };
    let op = next_op(run);
    run.tasks[i].merge_op = Some(op);
    emit_op(run, op, Some(&id), kind, fx);
    history(
        run,
        i,
        now,
        "handing back the run head its dependencies left",
    );
}
