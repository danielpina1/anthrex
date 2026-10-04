//! Decisions 28, 44 and 45, engine side (M8a.15): a run restored after a daemon
//! restart, `run resume` (and the `resume` edit) of a paused run, and the launches the
//! restart lost, re-issued once the run runs. Pure (design decision 2).
//!
//! **Restore.** A `running` run becomes `paused` (`paused_from = running`); every other
//! state is kept. Every session is ended: its process died with the old daemon. The
//! ops the journal replays keep their ids and their results are applied; a held op
//! stays pending for the driver to answer later (ruling T22-N3); every other pending
//! op is dropped (reconcile's `NotStarted`), so a late result of it is ignored
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
use super::restore_lost::lost;
use super::signals::end_round;
use super::{
    Effect, EngineState, OpId, OpKind, OpResult, ReplyId, clock, complete, emit_op, merge, next_op,
    outbox, results::op_done, review,
};
use crate::run::contract::{RESUME_REVIEWER, RESUME_WORKER};
use crate::run::model::{FallbackState, PendingOp, Run, StallState};
use crate::run::role_launch::session_uuid_of;

/// The journal's answers: `(run id, op, result)`.
type Replay = Vec<(String, OpId, OpResult)>;
/// Ops kept pending for the driver to answer later: `(run id, op)`.
type Held = Vec<(String, OpId)>;

/// `Event::Restore` (decisions 44, 45).
pub(super) fn restore(
    state: &mut EngineState,
    runs: Vec<Run>,
    (replay, held): (Replay, Held),
    now: u64,
    fx: &mut Vec<Effect>,
) {
    let mut restored = Vec::new();
    for mut run in runs {
        // M8b.15 review (I5): the new daemon's OTLP totals add to what was stored.
        run.orchestrator_base = run.orchestrator_usage;
        let original = run.clone();
        let kept: BTreeSet<OpId> = replay
            .iter()
            .map(|(id, op, _)| (id, *op))
            .chain(held.iter().map(|(id, op)| (id, *op)))
            .filter(|(id, _)| **id == run.id)
            .map(|(_, op)| op)
            .collect();
        prepare(&mut run, &kept, now, fx);
        // Milestone 9 decision 43: every session the old daemon ran is over.
        super::history::interrupt_open(&mut run, fx);
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
        // Decision 15 (ruling T12-1): the downtime of a run left stopped is paused time.
        super::pause::restored(&original, run, now);
        // Milestone 9.6 ruling T7-2: nor is it a design phase's.
        super::design::restored(run, now);
        // Decision 47: a run the restore changed bumps its revision (review minor 5);
        // `step` leaves a run new to the state at the revision it arrived with. The
        // digest moves with it (M9.6 review fix I-1); an unchanged run is left as
        // loaded, so restoring it writes nothing.
        if *run != original {
            run.revision += 1;
            crate::run::orch::digest::note_change(run);
        }
    }
    // Milestone 9.3 decision 19: idle chains end with the old daemon (KG §3.6).
    state.chains = crate::run::chain::rebuild(&state.runs);
}

/// Before the replay: the run's state, the waits that died with the old daemon, and the
/// ops the journal did not see finish.
fn prepare(run: &mut Run, kept: &BTreeSet<OpId>, now: u64, fx: &mut Vec<Effect>) {
    // M8a.14: an accept's or discard's reply belonged to the old daemon.
    run.finish_reply = None;
    // Milestone 9.1 decision 47: a run from before stages has its one.
    super::stages::ensure_first(run);
    // Milestone 9.3 decision 18: a run from before rounds has its one.
    super::goal_rounds::ensure_first(run);
    if run.state.is_terminal() {
        // M8b decision 33: an ended run still owes the history lines it had in flight.
        let history: Vec<PendingOp> = run
            .pending_ops
            .values()
            .filter(|p| !kept.contains(&p.op) && is_history(&p.kind))
            .cloned()
            .collect();
        for pending in history {
            run.pending_ops.remove(&pending.op);
            lost(run, pending, now, fx);
        }
        return;
    }
    // Rulings T15-I2, T15-I3: the downtime is no session time. The final fix wave's
    // A-I1 (c): nor is it a racer's, so each lane's clock stops too, in its view.
    for task in run.tasks.iter_mut() {
        clock::stop_at_restore(task);
    }
    for (i, lane) in lanes_in_view(run) {
        super::race_view::in_lane(run, i, lane, fx, |run, _| {
            clock::stop_at_restore(&mut run.tasks[i]);
        });
    }
    // Milestone 9 decisions 20 and 32: run scouts and sub-planners are not resumed.
    super::planners::restore(run, now);
    // Milestone 9 decision 26: a planning run has live agents too.
    if matches!(
        run.state,
        RunState::Running | RunState::Planning | RunState::Brainstorming | RunState::Specifying
    ) {
        run.paused_from = Some(run.state);
        run.state = RunState::Paused;
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
        // The final fix wave's A-I1 (b): nor does a race lane's.
        for lane in task.race.iter_mut().flat_map(|r| r.lanes.iter_mut()) {
            lane.gates.claim = None;
        }
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

/// M8b decisions 32 and 33: a diff measurement or a history line.
fn is_history(kind: &OpKind) -> bool {
    matches!(
        kind,
        OpKind::MeasureDiff { .. } | OpKind::AppendHistory { .. }
    )
}

/// Every `(task, lane)` whose events go through the lane's view
/// (`race_view::view_lane`): each lane of a race but the crowned one.
fn lanes_in_view(run: &Run) -> Vec<(usize, proto::RaceLane)> {
    (run.tasks.iter().enumerate())
        .flat_map(|(i, task)| {
            (task.race.iter().flat_map(|r| &r.lanes))
                .filter_map(move |l| super::race_view::view_lane(task, Some(l.lane)))
                .map(move |l| (i, l))
        })
        .collect()
}

/// After the replay: every session ends (its process died with the old daemon), and a
/// cancel deferred behind a merge that did not survive applies (carry T14-R2).
fn settle(run: &mut Run, now: u64, fx: &mut Vec<Effect>) {
    if run.state.is_terminal() {
        return;
    }
    // Milestone 9 decision 11: the orchestrator's window is restored dormant.
    super::orch_window::restored(run);
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
        // Milestone 9.5 ruling T17b-2: no `ProcessExited` ended a racer's round here,
        // so its lane's checkout is never cleaned as if it had exited.
        round.orphaned = round.role == AgentRole::Racer;
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
    rebaseline: Option<super::Rebaseline>,
    now: u64,
    fx: &mut Vec<Effect>,
) {
    // Milestone 9.0.6 decision 8: every refusal first, changing nothing.
    let refusal = state.runs.get(run_id).and_then(|r| {
        let rebaseline = rebaseline.is_some();
        super::actions::rules::resume(r, rebaseline)
    });
    if let Some(text) = refusal {
        return fx.push(Effect::Reply {
            reply,
            result: Err(text),
        });
    }
    // Milestone 9.1 ruling C-18: a resume retries tier 3 after the executor's failures;
    // a running run whose stage was held needs nothing more. Ruling C-27 (5): a running
    // run with no held stage is refused below, and a refused resume changes nothing.
    // Milestone 9.2 ruling R-11: a stage held by a refused push pushes again.
    if let Some(run) = state.runs.get_mut(run_id)
        && run.state == RunState::Running
        && let Some(text) = retry_held(run, now)
    {
        return fx.push(Effect::Reply {
            reply,
            result: Ok(text),
        });
    }
    let paused = state
        .runs
        .get(run_id)
        .is_some_and(|r| r.state == RunState::Paused);
    if !paused {
        // Milestone 9.6 decision 8: a run a design phase's budget halted goes back to it.
        if let Some(run) = state.runs.get_mut(run_id)
            && rebaseline.is_none()
            && let Some(text) = super::design::resume_phase(run, now)
        {
            return fx.push(Effect::Reply {
                reply,
                result: Ok(text),
            });
        }
        // Milestone 9 decision 11: a run at the gate (or planning, or in a design
        // phase) keeps its state, and its dormant orchestrator restarts.
        if let Some(run) = state.runs.get_mut(run_id)
            && matches!(
                run.state,
                RunState::AwaitingApproval
                    | RunState::Planning
                    | RunState::Brainstorming
                    | RunState::Specifying
            )
            && super::orch_window::relaunch(run, now, fx)
        {
            let text = format!("run {run_id}: its orchestrator restarts");
            return fx.push(Effect::Reply {
                reply,
                result: Ok(text),
            });
        }
        let halted = state
            .runs
            .get(run_id)
            .is_some_and(|r| r.state == RunState::Halted);
        merge::resume(state, reply, run_id, rebaseline, now, fx);
        if let Some(run) = state.runs.get_mut(run_id)
            && halted
            && run.state == RunState::Running
        {
            super::full::retry(run, now);
            super::delivery::release(run, now);
            resumed(run, now, fx);
        }
        return;
    }
    let Some(run) = state.runs.get_mut(run_id) else {
        return;
    };
    let mut text = format!("run {run_id} resumed");
    // `--rebaseline` records the refs the driver read, as for a halted run.
    if let Some(read) = rebaseline {
        text.push_str(&super::stages::rebaseline(run, &read, now, fx));
    }
    super::full::retry(run, now);
    super::delivery::release(run, now);
    unpause(run, now, fx);
    fx.push(Effect::Reply {
        reply,
        result: Ok(text),
    });
}

/// What a running run's `run resume` retries: a stage's tier 3 held after executor
/// failures (9.1 ruling C-18) and a stage's push the remote refused (9.2 ruling R-11).
fn retry_held(run: &mut Run, now: u64) -> Option<String> {
    let tier3 = super::full::held(run) && super::full::retry(run, now);
    let pushes = super::delivery::release(run, now);
    let mut parts = Vec::new();
    if tier3 {
        parts.push("tier 3 retries".to_string());
    }
    if !pushes.is_empty() {
        let stages: Vec<String> = pushes.iter().map(u16::to_string).collect();
        parts.push(format!("held pushes retry (stage {})", stages.join(", ")));
    }
    (!parts.is_empty()).then(|| format!("run {}: {}", run.id, parts.join("; ")))
}

/// A paused run returns to `paused_from` and resumes (the `resume` edit and `run
/// resume`).
pub(super) fn unpause(run: &mut Run, now: u64, fx: &mut Vec<Effect>) {
    run.state = run.paused_from.take().unwrap_or(RunState::Running);
    log(run, now, "resumed");
    // Milestone 9.6 task M9.6.8 (m1): a brainstorm that ended while paused settles.
    super::design_agents::settle(run, now, fx);
    resumed(run, now, fx);
    // Milestone 9 decisions 11 and 29: the orchestrator restarts, and a promotion
    // recorded before milestone 9 is performed.
    super::orch_window::relaunch(run, now, fx);
    super::promote::on_resume(run, now, fx);
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
        // Milestone 9.5 decision 23: each live lane of a race, in its view.
        let lanes: Vec<proto::RaceLane> = (run.tasks[i].race.iter())
            .flat_map(|r| &r.lanes)
            .filter(|l| super::race_view::live(l.state))
            .map(|l| l.lane)
            .collect();
        for lane in lanes {
            super::race::with_lane(run, i, lane, now, fx, |run, fx| {
                resume_worker(run, i, restored.is_some(), now, fx);
                resume_reviewer(run, i, restored.is_some(), now);
            });
        }
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
            // Milestone 9.5 decision 20: a lane's session is relaunched in its view.
            let task = &run.tasks[i];
            let lane = super::race_view::view_lane(task, task.rounds[r].lane);
            if task.lane_view.is_none() && lane.is_some() {
                continue;
            }
            let Some(kind) = run.tasks[i].rounds[r].relaunch.take() else {
                continue;
            };
            let task = &run.tasks[i];
            let round = &task.rounds[r];
            let wanted = !round.retiring
                && match round.role {
                    // Milestone 9.5 decisions 19, 26: a test writer and a racer resume as a
                    // worker does.
                    AgentRole::Worker | AgentRole::TestWriter | AgentRole::Racer => {
                        matches!(task.state, TaskState::Preparing | TaskState::Working)
                    }
                    // Milestone 9 decision 35: a research task's session.
                    AgentRole::Scout => task.state == TaskState::Working,
                    _ => task.state == TaskState::Review,
                };
            if !wanted {
                end_round(&mut run.tasks[i].rounds[r], now);
                continue;
            }
            let op = next_op(run);
            let mut kind = *kind;
            let fresh = session_uuid_of(run, op);
            let round = &mut run.tasks[i].rounds[r];
            if let OpKind::CreateWindow {
                session_uuid: uuid, ..
            } = &mut kind
                && uuid.is_some()
            {
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
