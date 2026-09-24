//! Decisions 28, 44 and 45, engine side (M8a.15): a run restored after a daemon
//! restart, `run resume` (and the `resume` edit) of a paused run, and the launches the
//! restart lost, re-issued once the run runs. Pure (design decision 2).
//!
//! **Restore.** A `running` run becomes `paused` (`paused_from = running`); every other
//! state is kept. Every session is ended: its process died with the old daemon. The
//! ops the journal replays keep their ids and their results are applied; every other
//! pending op is dropped (reconcile's `NotStarted`), so a late result of it is ignored
//! (ruling T12-N's correlation), and whatever waited for it is cleared or re-issued:
//! idempotent git work (the integration worktree, a task worktree, an abort, a
//! removal) at once, a session's launch on the first running pass, and everything else
//! by the scheduler from the task's state (a gate's op, the merge candidate, a lost
//! merge-queue hand-back, the fresh session's diff, the ref check). Messages a lost
//! delivery or resume carried go again (decision 29: at least once). A cancel deferred
//! behind a merge that did not survive applies.
//!
//! **Resume.** Second-stage deadlines that expired fire first (an interrupted stall's
//! grace is rung 2, without resuming the old session; a pending continue and a nudged
//! round's silence follow in the same step's watchdog); first-stage ones are re-armed
//! from `now`. After a restore each working task's worker, and each reviewer owing a
//! verdict, is resumed with `RESUME_WORKER` or `RESUME_REVIEWER` through the outbox
//! (decisions 28, 29); a worker whose task is in a gate gets its next message when the
//! task works again (ruling T13-I3). A resume that fails starts a fresh session
//! (`outbox::resumed`). Neither the downtime nor the pause is session time
//! (`clock`).

use std::collections::BTreeSet;

use proto::{AgentRole, RunState, TaskState};

use super::dispatch::history;
use super::ladder::{self, worker_round};
use super::requests::log;
use super::signals::end_round;
use super::{
    Effect, EngineState, OpId, OpKind, OpResult, ReplyId, clock, complete, emit_op, merge, next_op,
    op_done, outbox, review,
};
use crate::run::contract::{RESUME_REVIEWER, RESUME_WORKER, sha7};
use crate::run::model::{FallbackState, PendingOp, Run, StallState};
use crate::run::role_launch::session_uuid;

/// `Event::Restore` (decisions 44, 45).
pub(super) fn restore(
    state: &mut EngineState,
    runs: Vec<Run>,
    replay: Vec<(String, OpId, OpResult)>,
    now: u64,
    fx: &mut Vec<Effect>,
) {
    let mut restored = Vec::new();
    for mut run in runs {
        let original = run.clone();
        let kept: BTreeSet<OpId> = replay
            .iter()
            .filter(|(id, _, _)| *id == run.id)
            .map(|(_, op, _)| *op)
            .collect();
        prepare(&mut run, &kept, now, fx);
        restored.push((run.id.clone(), original));
        state.runs.insert(run.id.clone(), run);
    }
    for (run_id, op, result) in replay {
        op_done(state, &run_id, op, result, now, fx);
    }
    for (id, original) in restored {
        let Some(run) = state.runs.get_mut(&id) else {
            continue;
        };
        settle(run, now, fx);
        // Decision 47: a run the restore changed bumps its revision (review minor 5);
        // `step` leaves a run new to the state at the revision it arrived with.
        if *run != original {
            run.revision += 1;
        }
    }
}

/// Before the replay: the run's state, the waits that died with the old daemon, and the
/// ops the journal did not see finish.
fn prepare(run: &mut Run, kept: &BTreeSet<OpId>, now: u64, fx: &mut Vec<Effect>) {
    // M8a.14: an accept's or discard's reply belonged to the old daemon.
    run.finish_reply = None;
    if run.state.is_terminal() {
        return;
    }
    // Rulings T15-I2, T15-I3: the downtime is no session time.
    for task in run.tasks.iter_mut() {
        clock::stop_at_restore(task);
    }
    if run.state == RunState::Running {
        run.state = RunState::Paused;
        run.paused_from = Some(RunState::Running);
        log(run, now, "restored after a daemon restart; paused");
    }
    for message in run.outbox.iter_mut() {
        message.delivered_at = None;
    }
    for task in run.tasks.iter_mut() {
        // Ruling T12-I1: no claim outlives its session; an override's reply id was the
        // old daemon's.
        task.claim = None;
        task.override_count = None;
        for round in task.rounds.iter_mut() {
            if round.fallback == FallbackState::Counting {
                round.fallback = FallbackState::None;
            }
            round.count_op = None;
            round.count_retry_at = None;
            round.resume_op = None;
            round.carried.clear();
        }
    }
    let dropped: Vec<PendingOp> = run
        .pending_ops
        .values()
        .filter(|p| !kept.contains(&p.op))
        .cloned()
        .collect();
    run.pending_ops.retain(|op, _| kept.contains(op));
    for pending in dropped {
        lost(run, pending, now, fx);
    }
}

/// Decision 44: an op dropped as `NotStarted`, and what its task needs instead.
fn lost(run: &mut Run, pending: PendingOp, now: u64, fx: &mut Vec<Effect>) {
    let PendingOp { op, task_id, kind } = pending;
    let i = task_id
        .as_deref()
        .and_then(|id| run.tasks.iter().position(|t| t.id() == id));
    match (&kind, i) {
        // Idempotent git work that starts no session: sent again under a new id.
        (
            OpKind::CreateRunBranch { .. }
            | OpKind::PrepareWorktree { .. }
            | OpKind::AbortMerge { .. }
            | OpKind::RemoveWorktree { .. },
            _,
        ) => {
            let again = next_op(run);
            emit_op(run, again, task_id.as_deref(), kind, fx);
        }
        // A session starts only while the run runs (ruling T14-I2): `relaunch`.
        (OpKind::CreateWindow { .. }, Some(i)) => {
            if let Some(round) = run.tasks[i].rounds.iter_mut().find(|r| r.launch_op == op) {
                round.relaunch = Some(Box::new(kind));
            }
        }
        (OpKind::MergeCandidate { .. }, Some(i)) if run.tasks[i].merge_op == Some(op) => {
            run.tasks[i].merge_op = None;
        }
        // Carry T14-R2: a lost merge-queue hand-back is sent again by the running pass.
        (OpKind::HandBack { .. }, Some(i)) if run.tasks[i].merge_op == Some(op) => {
            let task = &mut run.tasks[i];
            task.merge_op = None;
            task.handback_due = task.state == TaskState::MergeQueue;
        }
        // Carry T13: `start_gates` and `dispatch_reviewers` re-issue a gate's op.
        (OpKind::Proof { .. } | OpKind::Check { .. } | OpKind::PrepareReview { .. }, Some(i))
            if run.tasks[i].gate_op == Some(op) =>
        {
            run.tasks[i].gate_op = None;
        }
        (OpKind::Accept { .. } | OpKind::Discard { .. }, _) => {
            let text = "an accept or discard did not finish before the restart; request it again";
            log(run, now, text);
        }
        // `VerifyDone`, `CountCommits` and `ResumeSession` had their waiters cleared;
        // `DiffSoFar`, `VerifyRefs`, the final check and an N5 hand-back are sent again
        // by the scheduler, which sees none in flight.
        _ => {}
    }
}

/// After the replay: every session ends (its process died with the old daemon), and a
/// cancel deferred behind a merge that did not survive applies (carry T14-R2).
fn settle(run: &mut Run, now: u64, fx: &mut Vec<Effect>) {
    if run.state.is_terminal() {
        return;
    }
    // Ruling T15-I1: whatever a session waited for died with the old daemon, so the
    // next resume re-engages every working task, whether or not a session was live.
    if run.tasks.iter().any(|t| !t.rounds.is_empty()) {
        run.restored = Some(now);
    }
    for round in run.tasks.iter_mut().flat_map(|t| t.rounds.iter_mut()) {
        if round.window_id.is_none() || round.ended {
            continue;
        }
        // Carry T12-RR4: `end_round` clears `interrupted`; an `Interrupted` stall is
        // settled by the resume (its grace fires, or the restart ended the turn).
        end_round(round, now);
        round.open_subagents.clear();
        round.fallback_waiting = false;
        round.in_retry_streak = false;
        round.turn_had_task_done = false;
    }
    for i in 0..run.tasks.len() {
        if run.tasks[i].cancel_deferred && !merge::candidate_in_flight(run, i) {
            let why = "its cancel, after its merge did not survive the restart";
            complete::cancel_now(run, i, why, now, fx);
        }
    }
}

/// `run resume` (decision 45; a halted run's, decision 21, is `merge::resume`).
pub(super) fn resume(
    state: &mut EngineState,
    reply: ReplyId,
    run_id: &str,
    rebaseline: Option<(String, String)>,
    now: u64,
    fx: &mut Vec<Effect>,
) {
    let paused = state
        .runs
        .get(run_id)
        .is_some_and(|r| r.state == RunState::Paused);
    if !paused {
        let halted = state
            .runs
            .get(run_id)
            .is_some_and(|r| r.state == RunState::Halted);
        merge::resume(state, reply, run_id, rebaseline, now, fx);
        if let Some(run) = state.runs.get_mut(run_id)
            && halted
            && run.state == RunState::Running
        {
            resumed(run, now, fx);
        }
        return;
    }
    let Some(run) = state.runs.get_mut(run_id) else {
        return;
    };
    let mut text = format!("run {run_id} resumed");
    // `--rebaseline` records the refs the driver read, as for a halted run.
    if let Some((base, head)) = rebaseline {
        text.push_str(&format!(
            " with --rebaseline: base {} at {}, run head {}",
            run.base_branch,
            sha7(&base),
            sha7(&head)
        ));
        run.base_sha = base;
        run.run_head = head;
        run.base_moved = None;
    }
    unpause(run, now, fx);
    fx.push(Effect::Reply {
        reply,
        result: Ok(text),
    });
}

/// A paused run returns to `paused_from` and resumes (the `resume` edit and `run
/// resume`).
pub(super) fn unpause(run: &mut Run, now: u64, fx: &mut Vec<Effect>) {
    run.state = run.paused_from.take().unwrap_or(RunState::Running);
    log(run, now, "resumed");
    resumed(run, now, fx);
}

/// Decision 45's resume of a run that runs again.
fn resumed(run: &mut Run, now: u64, fx: &mut Vec<Effect>) {
    // The time charged is `clock::sync`'s (rulings T15-I2, T15-I3).
    let restored = run.restored.take();
    for i in 0..run.tasks.len() {
        if run.tasks[i].state.is_finished() {
            continue;
        }
        resume_worker(run, i, restored.is_some(), now, fx);
        resume_reviewer(run, i, restored.is_some(), now);
    }
}

fn resume_worker(run: &mut Run, i: usize, restored: bool, now: u64, fx: &mut Vec<Effect>) {
    let Some(r) = worker_round(&run.tasks[i]) else {
        return;
    };
    let working = run.tasks[i].state == TaskState::Working;
    let round = &mut run.tasks[i].rounds[r];
    match round.stall {
        // Second stage: the grace ran out while nothing watched it.
        StallState::Interrupted { deadline } if working && now >= deadline => {
            let reason = "the interrupt did not end its turn".to_string();
            return ladder::stall(run, i, reason, now, fx);
        }
        // The restart ended the interrupted turn: its `stall_nudge` goes next.
        StallState::Interrupted { .. } if round.ended => round.stall = StallState::Nudged,
        // First stage: re-armed, so the downtime is not a stall.
        StallState::Watching => round.last_event = now,
        _ => {}
    }
    let resumable = round.ended && !round.retiring && round.session_id.is_some();
    let fresh = run.tasks[i].fresh_session.is_some();
    if restored && working && resumable && !fresh {
        let id = run.tasks[i].id().to_string();
        outbox::queue(run, &id, RESUME_WORKER.to_string(), now);
    }
}

fn resume_reviewer(run: &mut Run, i: usize, restored: bool, now: u64) {
    let Some(r) = review::reviewer_round(run, i) else {
        return;
    };
    if !review::owes_verdict(run, i, r) {
        return;
    }
    let round = &mut run.tasks[i].rounds[r];
    // A reviewer's silence has one stage (M8a.13): re-armed.
    round.last_event = now;
    if !restored || !round.ended || round.relaunch.is_some() {
        return;
    }
    if round.session_id.is_none() {
        // Nothing to resume: the round is given up with no verdict-less round counted,
        // and its reader slot takes a new round.
        round.retiring = true;
        return history(
            run,
            i,
            now,
            "its reviewer had no session to resume; a new round",
        );
    }
    let address = review::mailbox(run.tasks[i].id());
    outbox::queue_to(run, &address, r, RESUME_REVIEWER.to_string(), now);
}

/// Decision 44: a launch the restart lost, re-issued on the first running pass under a
/// new op id (and so, for Claude, a new session id: decision 24's uuid). A round its
/// task no longer wants ends instead.
pub(super) fn relaunch(run: &mut Run, now: u64, fx: &mut Vec<Effect>) {
    for i in 0..run.tasks.len() {
        for r in 0..run.tasks[i].rounds.len() {
            let Some(kind) = run.tasks[i].rounds[r].relaunch.take() else {
                continue;
            };
            let task = &run.tasks[i];
            let round = &task.rounds[r];
            let wanted = !round.retiring
                && match round.role {
                    AgentRole::Worker => {
                        matches!(task.state, TaskState::Preparing | TaskState::Working)
                    }
                    _ => task.state == TaskState::Review,
                };
            if !wanted {
                end_round(&mut run.tasks[i].rounds[r], now);
                continue;
            }
            let op = next_op(run);
            let mut kind = *kind;
            let round = &mut run.tasks[i].rounds[r];
            if let OpKind::CreateWindow {
                session_uuid: uuid, ..
            } = &mut kind
                && uuid.is_some()
            {
                let fresh = session_uuid(&run.id, op);
                *uuid = Some(fresh.clone());
                round.session_id = Some(fresh);
            }
            round.launch_op = op;
            round.started_at = now;
            // Ruling T15-R2 (N-1): the new session owes nothing to the lost launch.
            round.excused_secs = 0;
            round.last_event = now;
            let id = run.tasks[i].id().to_string();
            emit_op(run, op, Some(&id), kind, fx);
            history(run, i, now, "launching the session the restart lost");
        }
    }
}
