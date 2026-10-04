//! Milestone 9.5 decisions 20–22 (rulings RR-1, RR-2, T1-2, T1-3), the end of a race
//! (moved from `race.rs`, task M9.5.17b): the first lane past its last pre-merge gate
//! is `Won`, then `CrownRacer` creates the task branch at its head and its `Crowned`
//! makes the task the lane's; a lane that leaves the race while the other races on.
//! Pure (design decision 1).

use proto::{AgentRole, BlockReason, LaneState, RaceLane, TaskState};

use super::dispatch::{block, history};
use super::race_view::{address_lane, live};
use super::{Effect, OpKind, OpResult, ReplyId, emit_op, next_op, requests, schedule};
use crate::run::contract::sha7;
use crate::run::model::{Run, task_branch};

/// A lane passed its last pre-merge gate (decision 21). The first is `Won`, then
/// `CrownRacer` creates the task branch at its head; one that passes after a winner
/// is recorded and never crowns.
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
    let line = format!("race {id}: racer {} won", lane.label());
    history(run, i, now, line.clone());
    requests::log(run, now, line);
    crown(run, i, fx);
}

/// `CrownRacer` for task `i`'s winner (decision 21): the task branch created at the
/// lane's head, the lane checkout re-pinned under it (`driver/lane_ops.rs`).
fn crown(run: &mut Run, i: usize, fx: &mut Vec<Effect>) {
    let task = &run.tasks[i];
    let Some(race) = task.race.as_ref() else {
        return;
    };
    let Some(won) = race.lanes.iter().find(|l| Some(l.lane) == race.winner) else {
        return;
    };
    let (id, lane) = (task.id().to_string(), won.lane);
    let kind = OpKind::CrownRacer {
        root: run.root.clone(),
        task_branch: task_branch(&run.id, &id),
        lane_head: won.head.clone().unwrap_or_default(),
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
/// elsewhere halts the run again.
pub(super) fn crown_pass(run: &mut Run, fx: &mut Vec<Effect>) {
    for i in 0..run.tasks.len() {
        let task = &run.tasks[i];
        let due = (task.race.as_ref()).is_some_and(|r| r.winner.is_some() && !r.crowned);
        let crowning =
            schedule::op_in_flight(run, task.id(), |k| matches!(k, OpKind::CrownRacer { .. }));
        if due && !crowning && !task.state.is_finished() {
            crown(run, i, fx);
        }
    }
}

/// Decision 20: lane `lane` leaves the race while the other lane races on: `Out`, with
/// `reason`, its live racer and reviewer stopped through the existing `KillWindow`
/// path (ruling RR-2). Waiting for the stop and salvaging the checkout, and adopting
/// the last lane left, are task M9.5.17b's; until then a task whose two lanes are out
/// is blocked on its environment with both reasons.
pub(super) fn eliminate(
    run: &mut Run,
    i: usize,
    lane: RaceLane,
    reason: &str,
    now: u64,
    fx: &mut Vec<Effect>,
) {
    let id = run.tasks[i].id().to_string();
    let label = lane.label();
    let task = &mut run.tasks[i];
    let mut claim = None;
    let mut running = false;
    for round in task.rounds.iter_mut().filter(|r| r.lane == Some(lane)) {
        running |= round.role == AgentRole::Racer && !round.ended;
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
    if let Some(out) =
        (task.race.as_mut()).and_then(|r| r.lanes.iter_mut().find(|l| l.lane == lane))
    {
        out.state = LaneState::Out;
        out.reason = Some(reason.to_string());
        out.kill_sent_at = running.then_some(now);
        claim = out.gates.claim.take();
    }
    let text = format!("racer {label} of task {id} is out: {reason}. Stop now.");
    if let Some(reply) = claim.and_then(|c| c.reply) {
        reply_err(fx, reply, text);
    }
    run.outbox
        .retain(|m| address_lane(&id, &m.task_id) != Some(lane));
    let line = format!("race {id}: racer {label} out: {reason}");
    history(run, i, now, line.clone());
    requests::log(run, now, line);
    let race = run.tasks[i].race.as_ref();
    let both_out =
        race.is_some_and(|r| r.winner.is_none() && r.lanes.iter().all(|l| !live(l.state)));
    if both_out {
        let reasons: Vec<String> = (race.into_iter().flat_map(|r| &r.lanes))
            .map(|l| {
                let reason = l.reason.as_deref().unwrap_or_default();
                format!("racer {}: {reason}", l.lane.label())
            })
            .collect();
        let text = format!("both racers are out ({})", reasons.join("; "));
        block(run, i, BlockReason::Environment, text, now);
    }
}

fn reply_err(fx: &mut Vec<Effect>, reply: ReplyId, text: String) {
    fx.push(Effect::Reply {
        reply,
        result: Err(text),
    });
}

/// `CrownRacer`'s result (decision 21): `Crowned` makes the task the winning lane;
/// `RefMoved` halts the run, as for any engine ref (ruling T1-2); a failure blocks
/// the task on its environment.
pub(super) fn crown_done(run: &mut Run, i: usize, result: OpResult, now: u64) {
    match result {
        OpResult::Crowned { head } => on_crowned(run, i, &head, now),
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
/// task's, and the task goes to the merge queue with the lane's head.
pub(super) fn on_crowned(run: &mut Run, i: usize, head: &str, now: u64) {
    let id = run.tasks[i].id().to_string();
    let Some(lane) = (run.tasks[i].race.as_ref())
        .and_then(|r| r.lanes.iter().find(|l| Some(l.lane) == r.winner))
        .cloned()
    else {
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
    super::gates::enter(run, i, TaskState::MergeQueue, now);
    let text = format!(
        "racer {} crowned at {}; next: {}",
        lane.lane.label(),
        sha7(head),
        TaskState::MergeQueue.label()
    );
    history(run, i, now, text);
}
