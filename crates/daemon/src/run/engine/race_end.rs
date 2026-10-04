//! Milestone 9.5 decisions 20–23 (rulings RR-1, RR-2, T1-2, T1-3), the end of a race
//! (task M9.5.17b): the first lane past its last pre-merge gate is `Won` and the other
//! live lane `Lost` at once; `CrownRacer` creates the task branch at the winner's head
//! and its `Crowned` makes the task the lane's. A lane that leaves the race while the
//! other races on is `Out`; the last lane left is adopted, crowned at its last head, and
//! the ladder then applies to the task the action that took it out. A cancel stops
//! every lane. A stopped lane's salvage is `race_salvage.rs`. Pure (design decision 1).

use proto::{AgentRole, BlockInfo, BlockReason, LaneState, RaceLane, TaskState};

use super::dispatch::{block, history};
use super::race_view::{address_lane, live};
use super::{Effect, OpKind, OpResult, ReplyId, emit_op, ladder, next_op, requests, schedule};
use crate::run::contract::sha7;
use crate::run::model::{Lane, Run, Task, task_branch};
use crate::run::phases::set_state;

/// Decision 22: how long after its `KillWindow` a stopped lane's racer has to exit
/// before its checkout is kept instead of removed (the kill grace plus 30 s).
pub const LANE_EXIT_WAIT_SECS: u64 = crate::process::KILL_GRACE.as_secs() + 30;

fn lane_mut(task: &mut Task, lane: RaceLane) -> Option<&mut Lane> {
    (task.race.as_mut()).and_then(|r| r.lanes.iter_mut().find(|l| l.lane == lane))
}

/// A lane passed its last pre-merge gate (decision 21). The first is `Won`, then
/// `CrownRacer` creates the task branch at its head, and every other live lane is
/// `Lost` at once and stopped (task 17a's review, m1: it is never reviewed again). A
/// pass after a winner is recorded and never crowns.
pub(super) fn on_lane_passed(
    run: &mut Run,
    i: usize,
    lane: RaceLane,
    now: u64,
    fx: &mut Vec<Effect>,
) {
    let id = run.tasks[i].id().to_string();
    let Some(race) = run.tasks[i].race.as_mut() else {
        return;
    };
    if let Some(winner) = race.winner {
        let text = format!(
            "racer {} passed after racer {} won",
            lane.label(),
            winner.label()
        );
        return history(run, i, now, text);
    }
    let Some(won) = race.lanes.iter_mut().find(|l| l.lane == lane) else {
        return;
    };
    // Decision 21 (task M9.5.15's review): `Won` first, then the crown.
    won.state = LaneState::Won;
    race.winner = Some(lane);
    let losers: Vec<RaceLane> = (race.lanes.iter())
        .filter(|l| l.lane != lane && live(l.state))
        .map(|l| l.lane)
        .collect();
    let line = format!("race {id}: racer {} won", lane.label());
    history(run, i, now, line.clone());
    requests::log(run, now, line);
    for loser in losers {
        let reason = format!("racer {} won", lane.label());
        let refusal = format!(
            "the race for task {id} is over: racer {} won. Stop now.",
            lane.label()
        );
        stop_lane(run, i, loser, (LaneState::Lost, &reason), &refusal, now, fx);
        let line = format!("race {id}: racer {} lost: {reason}", loser.label());
        history(run, i, now, line.clone());
        requests::log(run, now, line);
    }
    crown(run, i, now, fx);
}

/// Decision 22 (ruling RR-2): lane `lane` leaves the race as `state` with `reason`. Its
/// live racer and reviewer are stopped through the existing `KillWindow` path, a claim
/// it waits on is answered with `refusal`, and its queued messages are dropped. Its
/// checkout is salvaged once its ops have returned and its racer has exited
/// (`race_salvage::pass`), timed from now.
fn stop_lane(
    run: &mut Run,
    i: usize,
    lane: RaceLane,
    (state, reason): (LaneState, &str),
    refusal: &str,
    now: u64,
    fx: &mut Vec<Effect>,
) {
    let id = run.tasks[i].id().to_string();
    let task = &mut run.tasks[i];
    for round in task.rounds.iter_mut().filter(|r| r.lane == Some(lane)) {
        if round.ended || round.retiring {
            continue;
        }
        match round.role {
            AgentRole::Reviewer => super::review::give_up(round, now, fx),
            _ => {
                round.retiring = true;
                if let Some(window_id) = round.window_id {
                    fx.push(Effect::KillWindow { window_id });
                }
            }
        }
    }
    let mut claim = None;
    if let Some(out) = lane_mut(task, lane) {
        out.state = state;
        out.reason = Some(reason.to_string());
        out.kill_sent_at = Some(now);
        claim = out.gates.claim.take();
    }
    if let Some(reply) = claim.and_then(|c| c.reply) {
        reply_err(fx, reply, refusal.to_string());
    }
    run.outbox
        .retain(|m| address_lane(&id, &m.task_id) != Some(lane));
}

/// Decision 20: lane `lane` leaves the race for `reason`. While the other lane races on
/// it is `Out` and stopped; when it is the last lane left it is adopted.
pub(super) fn eliminate(
    run: &mut Run,
    i: usize,
    lane: RaceLane,
    reason: &str,
    now: u64,
    fx: &mut Vec<Effect>,
) {
    let race = run.tasks[i].race.as_ref();
    let last = race.is_some_and(|r| {
        r.winner.is_none() && !(r.lanes.iter()).any(|l| l.lane != lane && live(l.state))
    });
    if last {
        return adopt(run, i, lane, reason, now, fx);
    }
    out(run, i, lane, reason, now, fx);
}

/// Lane `lane` is `Out` for `reason`, stopped, and logged.
fn out(run: &mut Run, i: usize, lane: RaceLane, reason: &str, now: u64, fx: &mut Vec<Effect>) {
    let id = run.tasks[i].id().to_string();
    let label = lane.label();
    let refusal = format!("racer {label} of task {id} is out: {reason}. Stop now.");
    stop_lane(run, i, lane, (LaneState::Out, reason), &refusal, now, fx);
    let line = format!("race {id}: racer {label} out: {reason}");
    history(run, i, now, line.clone());
    requests::log(run, now, line);
}

/// Decision 20: the last lane left is adopted. It is crowned (`adopt: true`) at its last
/// imported head, else its start commit; its session is not stopped, since the ladder
/// applies to the task once the crown is done ([`after_adoption`]). A lane whose
/// checkout was never prepared is not crowned: it is out, and the task is
/// `blocked(environment)` with its reason.
fn adopt(run: &mut Run, i: usize, lane: RaceLane, reason: &str, now: u64, fx: &mut Vec<Effect>) {
    let id = run.tasks[i].id().to_string();
    let prepared = lane_mut(&mut run.tasks[i], lane).is_some_and(|l| crown_head(l, true).is_some());
    if !prepared {
        out(run, i, lane, reason, now, fx);
        return block(run, i, BlockReason::Environment, reason.to_string(), now);
    }
    let task = &mut run.tasks[i];
    if let Some(adopted) = lane_mut(task, lane) {
        adopted.state = LaneState::Adopted;
        adopted.reason = Some(reason.to_string());
    }
    if let Some(race) = task.race.as_mut() {
        race.winner = Some(lane);
        race.adopted = true;
    }
    let line = format!("race {id}: racer {} adopted: {reason}", lane.label());
    history(run, i, now, line.clone());
    requests::log(run, now, line);
    crown(run, i, now, fx);
}

/// The head lane `lane` is crowned at: its last imported head, or, adopted, the start
/// commit its checkout was prepared from (decision 20). Never empty (task 17a's
/// review, m8).
fn crown_head(lane: &Lane, adopted: bool) -> Option<String> {
    let start = lane.start_commit.clone().filter(|_| adopted);
    lane.head.clone().or(start).filter(|h| !h.is_empty())
}

/// `CrownRacer` for task `i`'s winner (decision 21): the task branch created at the
/// lane's head, the lane checkout re-pinned under it (`driver/lane_ops.rs`). A winner
/// with no head to crown at is not crowned: the task is `blocked(environment)`.
fn crown(run: &mut Run, i: usize, now: u64, fx: &mut Vec<Effect>) {
    let task = &run.tasks[i];
    let Some(race) = task.race.as_ref() else {
        return;
    };
    let Some(won) = race.lanes.iter().find(|l| Some(l.lane) == race.winner) else {
        return;
    };
    let (id, lane) = (task.id().to_string(), won.lane);
    let Some(lane_head) = crown_head(won, race.adopted) else {
        let text = format!(
            "could not crown the race's winner: racer {} has no commit to crown",
            lane.label()
        );
        return block(run, i, BlockReason::Environment, text, now);
    };
    let kind = OpKind::CrownRacer {
        root: run.root.clone(),
        task_branch: task_branch(&run.id, &id),
        lane_head,
        adopt: race.adopted,
        checkout: run.task_path(&won.checkout),
    };
    let op = next_op(run);
    emit_op(run, op, Some(&id), kind, fx);
    if let Some(pending) = run.pending_ops.get_mut(&op) {
        pending.lane = Some(lane);
    }
}

/// Decision 27: a winner whose crown is neither done nor in flight (its `RefMoved`
/// halted the run, and a resume runs it again) is crowned again: the crown is a
/// compare-and-swap, so a branch the user has put right is created, and one still
/// elsewhere halts the run again. A blocked task (its crown failed) waits for `run
/// retry` ([`retry_crown`]), so a failing crown is not sent in a loop.
pub(super) fn crown_pass(run: &mut Run, now: u64, fx: &mut Vec<Effect>) {
    for i in 0..run.tasks.len() {
        let task = &run.tasks[i];
        let due = (task.race.as_ref()).is_some_and(|r| r.winner.is_some() && !r.crowned);
        let crowning =
            schedule::op_in_flight(run, task.id(), |k| matches!(k, OpKind::CrownRacer { .. }));
        let waits = task.state.is_finished() || task.state == TaskState::Blocked;
        if due && !crowning && !waits {
            crown(run, i, now, fx);
        }
    }
}

/// `run retry` of a task whose winner's crown failed: it works again, and the next pass
/// sends the crown again. Whether the task was such a task.
pub(super) fn retry_crown(run: &mut Run, i: usize, now: u64) -> bool {
    let task = &mut run.tasks[i];
    let waiting = (task.race.as_ref()).is_some_and(|r| r.winner.is_some() && !r.crowned);
    if !waiting || task.state != TaskState::Blocked {
        return false;
    }
    set_state(task, TaskState::Working, now);
    task.block = None;
    true
}

fn reply_err(fx: &mut Vec<Effect>, reply: ReplyId, text: String) {
    fx.push(Effect::Reply {
        reply,
        result: Err(text),
    });
}

/// `CrownRacer`'s result (decision 21): `Crowned` makes the task the winning lane;
/// `RefMoved` halts the run, as for any engine ref (ruling T1-2); a failure blocks
/// the task on its environment. A task cancelled meanwhile takes nothing.
pub(super) fn crown_done(
    run: &mut Run,
    i: usize,
    result: OpResult,
    now: u64,
    fx: &mut Vec<Effect>,
) {
    if run.tasks[i].state.is_finished() {
        return;
    }
    match result {
        OpResult::Crowned { head } => on_crowned(run, i, &head, now, fx),
        OpResult::RefMoved { reason } => super::merge::halt(run, reason, now),
        OpResult::Failed { message } => {
            let text = format!("could not crown the race's winner: {message}");
            block(run, i, BlockReason::Environment, text, now);
        }
        _ => {}
    }
}

/// Decision 21 and ruling T1-3: the crowned lane becomes the task. Its checkout is the
/// task's (`Task.worktree`, set together with `Task.branch`, the task branch the crown
/// created), its gate state and claimed head are the task's, its messages are the
/// task's, and the task goes to the merge queue with the lane's head; an adopted lane's
/// task takes the ladder's step instead ([`after_adoption`]).
pub(super) fn on_crowned(run: &mut Run, i: usize, head: &str, now: u64, fx: &mut Vec<Effect>) {
    let id = run.tasks[i].id().to_string();
    let Some(race) = run.tasks[i].race.as_ref() else {
        return;
    };
    let adopted = race.adopted;
    let Some(lane) = (race.lanes.iter().find(|l| Some(l.lane) == race.winner)).cloned() else {
        return;
    };
    let (path, branch) = (run.task_path(&lane.checkout), task_branch(&run.id, &id));
    let task = &mut run.tasks[i];
    super::race_view::become_lane(task, &lane);
    if let Some(race) = task.race.as_mut() {
        race.crowned = true;
    }
    task.worktree = path;
    task.branch = branch;
    task.head = Some(head.to_string());
    for message in run.outbox.iter_mut() {
        if address_lane(&id, &message.task_id) == Some(lane.lane) {
            message.task_id = match message.task_id.ends_with(".review") {
                true => super::review::mailbox(&id),
                false => id.clone(),
            };
        }
    }
    let label = lane.lane.label();
    if adopted {
        let text = format!("racer {label} adopted and crowned at {}", sha7(head));
        history(run, i, now, text);
        return after_adoption(run, i, &lane, now, fx);
    }
    super::gates::enter(run, i, TaskState::MergeQueue, now);
    let text = format!(
        "racer {label} crowned at {}; next: {}",
        sha7(head),
        TaskState::MergeQueue.label()
    );
    history(run, i, now, text);
}

/// Decision 20: the ladder applies to the adopted task the action that took its lane
/// out, its counters the lane's: rung 3 (a spill, a mis-size, a second budget breach),
/// rung 2 (a second gate failure, a stall, a budget breach), or the lane's own block
/// (`task_blocked`: a question waits for its answer in the same session).
fn after_adoption(run: &mut Run, i: usize, lane: &Lane, now: u64, fx: &mut Vec<Effect>) {
    let reason = lane.reason.clone().unwrap_or_default();
    match (lane.gates.rung, lane.gates.block.clone()) {
        (3.., _) => ladder::rung3(run, i, reason, now, fx),
        (2, _) => ladder::rung2(run, i, reason, now, fx),
        (_, Some(BlockInfo { reason, text })) => block(run, i, reason, text, now),
        (_, None) => block(run, i, BlockReason::Environment, reason, now),
    }
}

/// Decision 23: a cancelled task's lanes are all stopped, the winner waiting for its
/// crown included; `race_salvage::pass` salvages each once its racer has exited. A
/// crowned lane is the task's own checkout, which the cancel removes as any task's.
pub(super) fn cancel(run: &mut Run, i: usize, now: u64, fx: &mut Vec<Effect>) {
    let Some(race) = run.tasks[i].race.as_ref() else {
        return;
    };
    let crowned = race.crowned.then_some(race.winner).flatten();
    let stop: Vec<RaceLane> = (race.lanes.iter())
        .filter(|l| Some(l.lane) != crowned)
        .filter(|l| live(l.state) || matches!(l.state, LaneState::Won | LaneState::Adopted))
        .map(|l| l.lane)
        .collect();
    let id = run.tasks[i].id().to_string();
    for lane in stop {
        let refusal = format!("task {id} was cancelled. Stop now.");
        stop_lane(
            run,
            i,
            lane,
            (LaneState::Out, "cancelled"),
            &refusal,
            now,
            fx,
        );
    }
}
