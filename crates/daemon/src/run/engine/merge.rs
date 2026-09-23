//! Decision 36's merge queue and decision 21's ref guard, engine side (M8a.14). The
//! queue is width 1 and FIFO in arrival order: one `MergeCandidate` at a time, for the
//! claimed commit that passed the gates (`Task.head`, ruling T13-R2's N3), never the
//! task branch's tip. A merge sets the run head, cleans up (decision 20) and retires
//! the worker; a red candidate is a gate failure of `merge`; the first conflict hands
//! the run head back to the task's worktree and the second blocks it. A moved run ref
//! or a rewritten base halts the run until `run resume --rebaseline`; an advanced base
//! is only recorded. Results are taken by op id (`Task.merge_op`, ruling T12-N's
//! correlation). Pure (design decision 2).

use proto::{AgentRole, BlockReason, GateKind, RunState, Runtime, TaskState};

use super::dispatch::{block, history, salvage_ref};
use super::requests::log;
use super::signals::end_round;
use super::{
    Effect, EngineState, OpId, OpKind, OpResult, ReplyId, complete, emit_op, gates, ladder, next_op,
};
use crate::run::contract::{UNCLAIMED_COMMITS, candidate_red_message, conflict_message, sha7};
use crate::run::env::profile_env;
use crate::run::model::{BaseMoved, CheckRecord, Run, Task};

/// A `MergeCandidate` is in flight (decision 36: width 1).
pub(super) fn merging(run: &Run) -> bool {
    run.pending_ops
        .values()
        .any(|p| matches!(p.kind, OpKind::MergeCandidate { .. }))
}

/// Every scheduler pass of a running run: the queue drops tasks no longer in the merge
/// queue, then its head gets a `MergeCandidate` unless one is in flight.
pub(super) fn start_merge(run: &mut Run, now: u64, fx: &mut Vec<Effect>) {
    let queued: Vec<String> = run
        .merge_queue
        .iter()
        .filter(|id| {
            run.task(id)
                .is_some_and(|t| t.state == TaskState::MergeQueue)
        })
        .cloned()
        .collect();
    run.merge_queue = queued;
    if merging(run) {
        return;
    }
    let Some(id) = run.merge_queue.first().cloned() else {
        return;
    };
    let Some(i) = run.tasks.iter().position(|t| t.id() == id) else {
        return;
    };
    let Some(task_head) = run.tasks[i].head.clone() else {
        // Unreachable: a task reaches the merge queue only with an accepted claim.
        run.merge_queue.retain(|q| *q != id);
        let text = "no claimed commit to merge".to_string();
        return block(run, i, BlockReason::Environment, text, now);
    };
    let integration = run.integration_path();
    let kind = OpKind::MergeCandidate {
        root: run.root.clone(),
        integration: integration.clone(),
        run_branch: run.run_branch(),
        expected_run_head: run.run_head.clone(),
        base_branch: run.base_branch.clone(),
        expected_base: run.base_sha.clone(),
        task_head,
        message: format!("anthrex: merge {id}: {}", run.tasks[i].spec.title),
        check: run.profile.check.clone(),
        timeout_secs: run.profile.check_timeout_secs,
        env: profile_env(&run.profile, &integration),
    };
    let op = next_op(run);
    run.tasks[i].merge_op = Some(op);
    emit_op(run, op, Some(&id), kind, fx);
    history(run, i, now, "merging into the run branch");
}

/// Whether task `i` awaits `op` as its merge-queue op (a candidate or a hand-back).
pub(super) fn awaits(run: &Run, i: usize, op: OpId) -> bool {
    run.tasks[i].merge_op == Some(op)
}

/// A `MergeCandidate`'s result. `Merged` moved the run ref and `RefMoved` found the refs
/// wrong, so both apply to the run whatever became of the task meanwhile (a task
/// cancelled while its merge ran is logged, not un-merged). Any other result counts
/// only for the task that awaits it.
pub(super) fn candidate_done(
    run: &mut Run,
    i: Option<usize>,
    op: OpId,
    result: OpResult,
    now: u64,
    fx: &mut Vec<Effect>,
) {
    if let OpResult::Merged { commit } = &result {
        run.run_head = commit.clone();
        run.last_green_candidate = Some(commit.clone());
    }
    // The refs are the run's, whoever's merge found them moved. A task that awaits
    // this result stays at the head of the queue and merges again after a rebaseline.
    if let OpResult::RefMoved { reason } = &result {
        if let Some(i) = i.filter(|&i| awaits(run, i, op)) {
            run.tasks[i].merge_op = None;
            // Ruling T14-I1: the merge did not land, so a deferred cancel applies.
            if std::mem::take(&mut run.tasks[i].cancel_deferred) {
                complete::cancel_now(run, i, "its cancel, after its merge did not land", now, fx);
            }
        }
        return halt(run, reason.clone(), now);
    }
    let Some(i) = i.filter(|&i| awaits(run, i, op)) else {
        if let OpResult::Merged { commit } = &result {
            log(run, now, format!("a merge landed at {}", sha7(commit)));
        }
        return;
    };
    run.tasks[i].merge_op = None;
    let id = run.tasks[i].id().to_string();
    if run.tasks[i].state != TaskState::MergeQueue {
        if let OpResult::Merged { commit } = &result {
            let text = format!("{id} merged at {} after it left the queue", sha7(commit));
            log(run, now, text);
        }
        return;
    }
    run.merge_queue.retain(|q| *q != id);
    // Ruling T14-I1: a cancel that arrived during the merge applies only if the merge
    // did not land; one that landed makes the task merged and the cancel too late.
    if std::mem::take(&mut run.tasks[i].cancel_deferred) {
        if let OpResult::Merged { commit } = result {
            log(
                run,
                now,
                format!("the cancel of {id} arrived too late: it merged"),
            );
            return merged(run, i, commit, now, fx);
        }
        return complete::cancel_now(run, i, "its cancel, after its merge did not land", now, fx);
    }
    match result {
        OpResult::Merged { commit } => merged(run, i, commit, now, fx),
        OpResult::Conflict { files } => conflict(run, i, files, now, fx),
        OpResult::CandidateRed {
            code,
            timed_out,
            tail,
            secs,
        } => {
            let record = CheckRecord {
                at: now,
                ok: false,
                code,
                timed_out,
                tail,
                secs,
                on_candidate: true,
            };
            let command = run.profile.check.clone().unwrap_or_default();
            let text = candidate_red_message(&command, &record);
            run.tasks[i].checks.push(record);
            history(run, i, now, "the check failed on the merge candidate");
            ladder::gate_failure(run, i, GateKind::Merge, text, false, now, fx);
        }
        OpResult::Failed { message } => {
            let text = format!("could not merge: {message}");
            block(run, i, BlockReason::Environment, text, now);
        }
        _ => {}
    }
}

/// Decision 36 step 4 and decision 20's clean-up: the task is `merged`; its task, review
/// and proof worktrees are salvaged and removed (the task worktree unwatched first);
/// its worker is retired, and whatever still waits for it in the outbox is dropped.
fn merged(run: &mut Run, i: usize, commit: String, now: u64, fx: &mut Vec<Effect>) {
    let id = run.tasks[i].id().to_string();
    let task = &mut run.tasks[i];
    task.state = TaskState::Merged;
    task.block = None;
    task.merge_commit = Some(commit.clone());
    for round in task
        .rounds
        .iter_mut()
        .filter(|r| r.role == AgentRole::Worker && !r.ended && !r.retiring)
    {
        round.retiring = true;
        if let Some(window_id) = round.window_id {
            fx.push(Effect::RetireWindow { window_id });
        }
        // A Codex worker between turns has no process to wait for (one per turn).
        if round.route.runtime == Runtime::Codex && !round.turn_open && round.pid.is_none() {
            end_round(round, now);
        }
    }
    run.outbox.retain(|m| m.task_id != id);
    history(run, i, now, format!("merged at {}", sha7(&commit)));
    log(run, now, format!("merged {id} at {}", sha7(&commit)));
    let task = &run.tasks[i];
    fx.push(Effect::UnwatchWorktree {
        root: task.worktree.clone(),
    });
    let paths = [
        task.worktree.clone(),
        run.review_path(&id),
        run.proof_path(&id),
    ];
    let seq = next_salvage_seq(task);
    for (k, path) in paths.into_iter().enumerate() {
        let kind = OpKind::RemoveWorktree {
            root: run.root.clone(),
            path,
            salvage_ref: salvage_ref(run, &id, seq + k),
        };
        let op = next_op(run);
        emit_op(run, op, Some(&id), kind, fx);
    }
}

/// The next unused `<seq>` of the task's salvage refs (decision 20). Several worktrees
/// removed at once take consecutive numbers, and only the dirty ones record theirs, so
/// the next number follows the highest recorded one, not their count.
pub(super) fn next_salvage_seq(task: &Task) -> usize {
    task.salvage_refs
        .iter()
        .filter_map(|r| r.rsplit('/').next()?.parse::<usize>().ok())
        .max()
        .unwrap_or(0)
        .max(task.salvage_refs.len())
        + 1
}

/// Decision 36 step 6: the first conflict hands the run head back to the task's
/// worktree (not a gate failure); the second blocks the task as `conflict`.
fn conflict(run: &mut Run, i: usize, files: Vec<String>, now: u64, fx: &mut Vec<Effect>) {
    let task = &mut run.tasks[i];
    task.conflicts = task.conflicts.saturating_add(1);
    if task.conflicts >= 2 {
        let text = format!(
            "its branch conflicts with the run branch again: {}",
            files.join(", ")
        );
        return block(run, i, BlockReason::Conflict, text, now);
    }
    let id = task.id().to_string();
    let kind = OpKind::HandBack {
        worktree: task.worktree.clone(),
        run_head: run.run_head.clone(),
        task_head: task.head.clone(),
    };
    let op = next_op(run);
    run.tasks[i].merge_op = Some(op);
    emit_op(run, op, Some(&id), kind, fx);
    let text = format!(
        "conflicts with the run branch in {}; handing the run head back",
        files.join(", ")
    );
    history(run, i, now, text);
}

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
        OpResult::HandedBack { files, head, onto } => (files, head, onto),
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
            gates::enter(run, i, next);
            let text = format!("the run head merged cleanly; next: {}", next.label());
            return history(run, i, now, text);
        }
        gates::enter(run, i, TaskState::MergeQueue);
        return history(
            run,
            i,
            now,
            "the run head merged cleanly; back in the merge queue",
        );
    }
    let task = &mut run.tasks[i];
    task.state = TaskState::Working;
    task.handed_back = claimed && !gates_after;
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
pub(super) fn hand_back_due(run: &mut Run, i: usize, now: u64, fx: &mut Vec<Effect>) {
    let task = &mut run.tasks[i];
    task.handback_due = false;
    task.handed_back = false;
    task.gates_after_handback = true;
    task.state = TaskState::MergeQueue;
    task.gate_op = None;
    let id = task.id().to_string();
    let kind = OpKind::HandBack {
        worktree: task.worktree.clone(),
        run_head: run.run_head.clone(),
        task_head: task.head.clone(),
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

/// Ruling T14-I1: whether task `i`'s `MergeCandidate` is in flight, so a cancel waits
/// for its result.
pub(crate) fn candidate_in_flight(run: &Run, i: usize) -> bool {
    run.tasks[i].merge_op.is_some_and(|op| {
        run.pending_ops
            .get(&op)
            .is_some_and(|p| matches!(p.kind, OpKind::MergeCandidate { .. }))
    })
}

/// Decision 21: the run is `halted` with `reason`; nothing dispatches or merges, and
/// the windows keep running.
pub(super) fn halt(run: &mut Run, reason: String, now: u64) {
    run.halt_retryable = false;
    log(run, now, format!("halted: {reason}"));
    run.state = RunState::Halted;
    run.halted_reason = Some(reason);
}

/// Decision 21: the base branch advanced. The run goes on from its recorded `base_sha`;
/// the move is recorded for the attention line and accept. The same `to` changes
/// nothing, not even the revision.
pub(super) fn base_advanced(
    state: &mut EngineState,
    run_id: &str,
    to: String,
    commits: u32,
    now: u64,
) {
    let Some(run) = state.runs.get_mut(run_id) else {
        return;
    };
    if run.state.is_terminal()
        || to == run.base_sha
        || run.base_moved.as_ref().is_some_and(|m| m.to == to)
    {
        return;
    }
    let text = format!(
        "base {} moved from {} to {} ({commits} new commits)",
        run.base_branch,
        sha7(&run.base_sha),
        sha7(&to)
    );
    run.base_moved = Some(BaseMoved {
        from: run.base_sha.clone(),
        to,
        commits,
        seen_at: now,
    });
    log(run, now, text);
}

/// `run resume` of a halted run (decision 21): refused with the reason unless it
/// rebaselines, which records the refs the driver read — `base_sha` becomes the base
/// head, `run_head` the run branch's — clears `base_moved`, and returns the run to
/// `running`. Resuming a paused run is M8a.15's.
pub(super) fn resume(
    state: &mut EngineState,
    reply: ReplyId,
    run_id: &str,
    rebaseline: Option<(String, String)>,
    now: u64,
    fx: &mut Vec<Effect>,
) {
    let mut answer = |result| fx.push(Effect::Reply { reply, result });
    let Some(run) = state.runs.get_mut(run_id) else {
        return answer(Err(format!("unknown run {run_id}")));
    };
    match run.state {
        RunState::Halted => {}
        RunState::Paused => return answer(Err("run resume is not available yet".into())),
        other => return answer(Err(format!("run {run_id} is {}", other.label()))),
    }
    // Review m1: a halt on refs that could not be read is retried as it is.
    if rebaseline.is_none() && run.halt_retryable {
        run.halt_retryable = false;
        run.halted_reason = None;
        run.state = RunState::Running;
        log(run, now, "resumed; reading the refs again");
        return answer(Ok(format!("run {run_id} resumed")));
    }
    let Some((base, head)) = rebaseline else {
        let reason = run.halted_reason.clone().unwrap_or_default();
        return answer(Err(format!(
            "run {run_id} is halted: {reason}; check the refs, then resume with --rebaseline"
        )));
    };
    let text = format!(
        "resumed with --rebaseline: base {} at {}, run head {}",
        run.base_branch,
        sha7(&base),
        sha7(&head)
    );
    run.base_sha = base;
    run.run_head = head;
    run.base_moved = None;
    run.halted_reason = None;
    run.halt_retryable = false;
    run.state = RunState::Running;
    log(run, now, text.clone());
    answer(Ok(format!("run {run_id} {text}")));
}
