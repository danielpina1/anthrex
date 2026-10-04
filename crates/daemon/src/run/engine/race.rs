//! Milestone 9.5 decisions 18–21 (rulings RR-1, RR-3, RR-9, T1-2, T1-3), the race in
//! the reducer, part I (task M9.5.17a): whether a racing task races at dispatch, its
//! two lanes, each lane's events and scheduler passes in the lane's view
//! (`race_view.rs`), a lane that leaves the race, and the crown. A lane that passes
//! its last pre-merge gate first is `Won` and only then is `CrownRacer` sent; its
//! `Crowned` makes the task an ordinary task in the lane's checkout. Stopping and
//! salvaging a loser, adoption, messages and actions are task M9.5.17b's. Pure
//! (design decision 1).

use proto::{AgentRole, BlockReason, LaneState, RaceLane, Route, TaskState};

use super::dispatch::{block, history};
use super::race_view::{Exit, address_lane, in_lane, live, view_lane};
use super::{Effect, OpKind, OpResult, ReplyId, concurrency, emit_op, next_op, requests, schedule};
use crate::run::contract::sha7;
use crate::run::model::{Lane, Race, Run, Task, lane_checkout, task_branch};
use crate::run::phases::set_state;
use crate::run::validate::strength_label;
use crate::run::validate_patterns::{RACE_DISPATCH, peer_route};

/// What dispatch does with a task the scheduler would start (decision 18).
pub(super) enum Start {
    /// One ordinary worker, as for any task.
    Single,
    /// Two racers, the second on this route.
    Race(Route),
    /// It waits for its second writer slot.
    Wait,
}

/// Decision 18: whether task `i`, which the scheduler would start now, races. It races
/// only on the critical path, with `max_writers >= 2`, a second racer's route, and two
/// writer slots within the runtimes' caps (decision 16); with one, it waits up to
/// `race_slot_wait_secs`. Each fallback to one worker is noted on the task and logged.
pub(super) fn start(run: &mut Run, i: usize, now: u64) -> Start {
    let task = &run.tasks[i];
    if !RACE_DISPATCH || !task.spec.race || task.race.is_some() {
        return Start::Single;
    }
    if !schedule::critical_path(run).contains(&i) {
        return single(
            run,
            i,
            "race skipped: not on the critical path at dispatch",
            now,
        );
    }
    if run.limits.max_writers < 2 {
        return single(run, i, "race skipped: max_writers is 1", now);
    }
    let task = &run.tasks[i];
    let Some(peer) = peer_route(&run.roster, &task.route, &run.orch.installed) else {
        // Task M9.5.14's review: a plan file is validated against nothing installed.
        let text = format!(
            "race skipped: no installed {} model at strength {} for the second racer",
            crate::run::roster::peer(task.route.runtime).label(),
            strength_label(task.route.strength)
        );
        return single(run, i, &text, now);
    };
    let free = usize::from(run.limits.max_writers).saturating_sub(schedule::writers_busy(run));
    let runtime = task.route.runtime;
    if free >= 2 && concurrency::has_room(run, runtime) && concurrency::has_room(run, peer.runtime)
    {
        return Start::Race(peer);
    }
    let since = *run.tasks[i].race_wait_since.get_or_insert(now);
    let wait = run.limits.race_slot_wait_secs;
    if now.saturating_sub(since) < wait {
        return Start::Wait;
    }
    run.tasks[i].race_wait_since = None;
    let text = format!("race skipped: no second writer slot within {wait} s");
    single(run, i, &text, now)
}

fn single(run: &mut Run, i: usize, note: &str, now: u64) -> Start {
    let task = &mut run.tasks[i];
    task.race_wait_since = None;
    if !task.notes.iter().any(|n| n == note) {
        task.notes.push(note.to_string());
    }
    let line = format!("task {}: {note}", task.id());
    history(run, i, now, note);
    requests::log(run, now, line);
    Start::Single
}

/// A new lane of task `task` on `route`, reviewed on `review_route`.
fn new_lane(task: &str, lane: RaceLane, route: Route, review_route: Option<Route>) -> Lane {
    Lane {
        lane,
        route,
        review_route,
        checkout: lane_checkout(task, lane),
        state: LaneState::Preparing,
        session: 0,
        start_commit: None,
        head: None,
        done: None,
        failures: 0,
        bounces: Default::default(),
        stalls: 0,
        budget_exceeded: 0,
        spent: Default::default(),
        reason: None,
        salvage_ref: None,
        cleared_locks: Vec::new(),
        kill_sent_at: None,
        exited: false,
        removed: false,
        kept: false,
        gates: Default::default(),
    }
}

/// Decision 19: task `i` races. Lane a on its route, lane b on `peer`, each with its
/// own reviewer route, and each prepares its own standalone checkout `<task>.<lane>`
/// from the stage head (`prepare`, in the lane's view). The task is `working` until its
/// crown (decision 23).
pub(super) fn dispatch_race(
    run: &mut Run,
    i: usize,
    peer: Route,
    now: u64,
    fx: &mut Vec<Effect>,
    prepare: fn(&mut Run, usize, String, &mut Vec<Effect>),
) {
    let head = super::propagate::start_of(run, &run.tasks[i]);
    let task = &run.tasks[i];
    let (lists, installed) = (&run.limits.route_lists, &run.orch.installed);
    let reviewer = |route: &Route| {
        (task.review_level).map(|level| {
            crate::run::route_pick::review_route(lists, &run.roster, route, level, installed)
        })
    };
    let lanes = vec![
        new_lane(
            task.id(),
            RaceLane::A,
            task.route.clone(),
            reviewer(&task.route),
        ),
        new_lane(task.id(), RaceLane::B, peer.clone(), reviewer(&peer)),
    ];
    let text = format!(
        "dispatched as a race: racer a on {}, racer b on {}",
        task.route.runtime.label(),
        peer.runtime.label()
    );
    let task = &mut run.tasks[i];
    task.race = Some(Race {
        lanes,
        winner: None,
        adopted: false,
        started_at: now,
        crowned: false,
    });
    task.race_wait_since = None;
    set_state(task, TaskState::Working, now);
    history(run, i, now, text);
    for lane in [RaceLane::A, RaceLane::B] {
        let head = head.clone();
        with_lane(run, i, lane, now, fx, |run, fx| prepare(run, i, head, fx));
    }
}

/// Runs `f` in lane `lane`'s view of task `i`, then applies how the view ended: a lane
/// past its last gate (`on_lane_passed`), or out of the race (`eliminate`).
pub(super) fn with_lane<R>(
    run: &mut Run,
    i: usize,
    lane: RaceLane,
    now: u64,
    fx: &mut Vec<Effect>,
    f: impl FnOnce(&mut Run, &mut Vec<Effect>) -> R,
) -> Option<R> {
    let (result, exit) = in_lane(run, i, lane, fx, f)?;
    match exit {
        Exit::Stay => {}
        Exit::Passed => on_lane_passed(run, i, lane, now, fx),
        Exit::Out(reason) => eliminate(run, i, lane, &reason, now, fx),
    }
    Some(result)
}

/// Runs `f`, a scheduler pass, once in the view of every lane still in a race.
pub(super) fn each_lane(
    run: &mut Run,
    now: u64,
    fx: &mut Vec<Effect>,
    mut f: impl FnMut(&mut Run, &mut Vec<Effect>),
) {
    for i in 0..run.tasks.len() {
        if run.tasks[i].state.is_finished() {
            continue;
        }
        let lanes: Vec<RaceLane> = (run.tasks[i].race.iter())
            .flat_map(|race| {
                (race.lanes.iter())
                    .filter(|l| live(l.state) && race.winner != Some(l.lane))
                    .map(|l| l.lane)
            })
            .collect();
        for lane in lanes {
            with_lane(run, i, lane, now, fx, &mut f);
        }
    }
}

/// The task and lane of window `window_id`'s round, when its events go through the
/// lane's view (`signals::on_signal`'s search, refined to a lane).
pub fn lane_of_window(run: &Run, window_id: u32) -> Option<(usize, RaceLane)> {
    run.tasks.iter().enumerate().find_map(|(i, task)| {
        let round = (task.rounds.iter()).rfind(|r| r.window_id == Some(window_id))?;
        view_lane(task, round.lane).map(|lane| (i, lane))
    })
}

/// The task and lane an outbox address belongs to (`<task>.<x>`, `<task>.<x>.review`).
pub(super) fn lane_of_address(run: &Run, address: &str) -> Option<(usize, RaceLane)> {
    let stem = address.strip_suffix(".review").unwrap_or(address);
    let (id, _) = stem.rsplit_once('.')?;
    let i = run.tasks.iter().position(|t| t.id() == id)?;
    let lane = address_lane(id, address)?;
    view_lane(&run.tasks[i], Some(lane)).map(|lane| (i, lane))
}

/// The lane whose failed check waits for decider `decider_id`'s summary (M8b
/// decision 20), when the decider was started outside the lane's view.
pub(super) fn lane_of_decider(run: &Run, decider_id: u64) -> Option<(usize, RaceLane)> {
    run.tasks.iter().enumerate().find_map(|(i, task)| {
        let lane = (task.race.iter().flat_map(|r| &r.lanes)).find(|l| {
            (l.gates.pending_failure.as_ref()).is_some_and(|p| p.decider_id == decider_id)
        })?;
        view_lane(task, Some(lane.lane)).map(|lane| (i, lane))
    })
}

/// The engine's refusal of a `task_done` or `task_blocked` from a lane that left the
/// race (Interfaces, "MCP": engine-side refusals).
pub(super) fn refusal(run: &Run, i: usize, lane: RaceLane, tool: &str) -> Option<String> {
    if !matches!(tool, "task_done" | "task_blocked") {
        return None;
    }
    let task = &run.tasks[i];
    let race = task.race.as_ref()?;
    let id = task.id();
    if race.winner == Some(lane) {
        let text = format!(
            "racer {} of task {id} won the race; wait for the engine's next message",
            lane.label()
        );
        return Some(text);
    }
    if let Some(winner) = race.winner.filter(|w| *w != lane) {
        let winner = winner.label();
        return Some(format!(
            "the race for task {id} is over: racer {winner} won. Stop now."
        ));
    }
    let out = race.lanes.iter().find(|l| l.lane == lane)?;
    (out.state == LaneState::Out).then(|| {
        let reason = out.reason.as_deref().unwrap_or_default();
        format!(
            "racer {} of task {id} is out: {reason}. Stop now.",
            lane.label()
        )
    })
}

/// The reader slots the lanes' reviews hold (decision 41, counted per lane): a lane in
/// `review`, but the one whose view the task is (counted as the task).
pub fn lane_readers(run: &Run) -> usize {
    (run.tasks.iter())
        .filter(|t| !t.state.is_finished())
        .flat_map(|task| {
            (task.race.iter()).flat_map(move |race| {
                (race.lanes.iter()).filter(move |l| {
                    l.state == LaneState::Review
                        && Some(l.lane) != task.lane_view
                        && race.winner != Some(l.lane)
                })
            })
        })
        .count()
}

/// The window-name stem of task `task`'s sessions: `<task>.` (`t1.w1`, `t1.r1`), or in
/// a lane's view `<task>.<lane>` (`t1.aw1`, `t1.ar1`; decision 19).
pub(super) fn stem(task: &Task) -> String {
    match task.lane_view {
        Some(lane) => format!("{}.{}", task.id(), lane.label()),
        None => format!("{}.", task.id()),
    }
}

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

/// Milestone 9.5 decision 20: in a lane's view, rung 2 or 3 takes the lane out of its
/// race (`race_view` reads the block as `Out`): its sessions stopped, its rung and the
/// block kept for the lane. Nothing of the task's own changes (no escalation, no size
/// raised); the action that took the lane out is the adoption's (task M9.5.17b).
pub(super) fn lane_out(
    run: &mut Run,
    i: usize,
    rung: u8,
    reason: BlockReason,
    text: String,
    now: u64,
    fx: &mut Vec<Effect>,
) {
    super::ladder::kill_worker(run, i, fx);
    super::ladder::drop_queued(run, i);
    let task = &mut run.tasks[i];
    task.rung = rung;
    task.fresh_session = None;
    block(run, i, reason, text, now);
}
