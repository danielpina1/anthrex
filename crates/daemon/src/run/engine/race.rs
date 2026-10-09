//! Milestone 9.5 decisions 18–21 (rulings RR-1, RR-3, RR-9, T1-2, T1-3), the race in
//! the reducer, part I (task M9.5.17a): whether a racing task races at dispatch, its
//! two lanes, each lane's events and scheduler passes in the lane's view
//! (`race_view.rs`) and a lane taken out at rung 2 or 3. How a race ends (the crown, a
//! lane that leaves it) is `race_end.rs`. Pure (design decision 1).

use proto::{AgentRole, BlockReason, LaneState, RaceLane, Route, TaskState};

use super::dispatch::history;
use super::race_view::{Exit, address_lane, in_lane, live, view_lane};
use super::{Effect, OpKind, concurrency, requests, schedule};
use crate::run::model::{Lane, Race, RaceDecision, Run, Task, lane_checkout};
use crate::run::phases::set_state;

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
/// only on the critical path, with `max_writers >= 2` (its second racer's route, milestone
/// 9.8 decision 28, always exists: the row's fallback, else its own route), and two
/// writer slots within the runtimes' caps (decision 16); with one, it waits up to
/// `race_slot_wait_secs` from when it first waited. Ruling T17a-1: the first decision
/// is latched (`Task.race_decision`), so a fallback to one worker is noted and logged
/// once, and a task decided single stays single for this dispatch. A task with its own
/// checkout never races (the final fix wave's A-I2).
pub(super) fn start(run: &mut Run, i: usize, now: u64) -> Start {
    let task = &run.tasks[i];
    if !task.spec.race || task.race.is_some() || task.race_decision.is_some() {
        return Start::Single;
    }
    // The final fix wave's A-I2: a task that has its own checkout (a start commit, a
    // live worktree, a pre-warm, one still being made: ruling FW-2 (a), or one whose
    // making failed after its branch may have been made: ruling FW-4) works there; its
    // lanes would race from the stage head, and the crown could not create a task
    // branch that already exists.
    let making = schedule::op_in_flight(run, task.id(), |k| {
        matches!(k, OpKind::PrepareWorktree { .. })
    });
    let had = task.worktree_live || task.prewarmed || task.prepare_failed;
    if task.start_commit.is_some() || had || making {
        return single(run, i, "race skipped: the task already has a checkout", now);
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
    // Milestone 9.8 decision 28: the second racer takes the row's fallback, else the
    // task's own route, and keeps the overlap rule (review B's M9).
    let peer = crate::run::model_roles::racer_route(run, i);
    let free = usize::from(run.limits.max_writers).saturating_sub(schedule::writers_busy(run));
    let runtime = task.route.runtime;
    // Both lanes on one runtime (no row fallback) take two slots under its cap.
    let room = match peer.runtime == runtime {
        true => {
            let busy = concurrency::writers_busy_on(run, runtime);
            concurrency::cap(run, runtime).saturating_sub(busy) >= 2
        }
        false => concurrency::has_room(run, runtime) && concurrency::has_room(run, peer.runtime),
    };
    if free >= 2 && room {
        run.tasks[i].race_decision = Some(RaceDecision::Race);
        return Start::Race(peer);
    }
    // The final fix wave's m6: the wait is timed on the run's running clock, so a
    // pause and a daemon's downtime (paused time, ruling T12-1) are no waiting time.
    let running = now.saturating_sub(run.paused_total(now));
    let since = *run.tasks[i].race_wait_since.get_or_insert(running);
    let wait = run.limits.race_slot_wait_secs;
    if running.saturating_sub(since) < wait {
        return Start::Wait;
    }
    run.tasks[i].race_wait_since = None;
    let text = format!("race skipped: no second writer slot within {wait} s");
    single(run, i, &text, now)
}

fn single(run: &mut Run, i: usize, note: &str, now: u64) -> Start {
    let task = &mut run.tasks[i];
    task.race_wait_since = None;
    task.race_decision = Some(RaceDecision::Single {
        reason: note.to_string(),
    });
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
    // Milestone 9.8 decision 27: each lane's reviewer is the reviewer row against it.
    let models = run.limits.models();
    let reviewer = |route: &Route| (task.review_level).map(|_| models.reviewer_route(route).0);
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
        ended: false,
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
        Exit::Passed => super::race_end::on_lane_passed(run, i, lane, now, fx),
        Exit::Out(reason) => super::race_end::eliminate(run, i, lane, &reason, now, fx),
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

/// Ruling RR-4: the lanes a message to racing task `task` (not in a lane's view) goes
/// to: every live lane, or the winner waiting for its crown. None once crowned or when
/// it does not race.
pub(super) fn message_lanes(task: &Task) -> Vec<RaceLane> {
    let Some(race) = task.race.as_ref().filter(|_| task.lane_view.is_none()) else {
        return Vec::new();
    };
    if race.crowned || race.ended {
        return Vec::new();
    }
    match race.winner {
        Some(winner) => vec![winner],
        None => (race.lanes.iter())
            .filter(|l| live(l.state))
            .map(|l| l.lane)
            .collect(),
    }
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
    // Task M9.5.17b: a lane that went out says why, whoever won since.
    let out = race.lanes.iter().find(|l| l.lane == lane)?;
    if out.state == LaneState::Out {
        let reason = out.reason.as_deref().unwrap_or_default();
        let label = lane.label();
        return Some(format!(
            "racer {label} of task {id} is out: {reason}. Stop now."
        ));
    }
    if race.winner == Some(lane) {
        let text = format!(
            "racer {} of task {id} won the race; wait for the engine's next message",
            lane.label()
        );
        return Some(text);
    }
    let winner = race.winner.filter(|w| *w != lane)?.label();
    Some(format!(
        "the race for task {id} is over: racer {winner} won. Stop now."
    ))
}

/// The reader slots the lanes' reviews hold (decision 41, counted per lane). Ruling
/// T17a-2: by `schedule::holds_reader`'s rule, a lane in `review` holds one only once
/// its reviewer round or its `PrepareReview` exists. In a lane's view the viewed lane
/// is counted as the task, and the others as `enter` found them (`parked_readers`).
pub fn lane_readers(run: &Run) -> usize {
    (run.tasks.iter())
        .map(|task| match task.lane_view {
            Some(_) => task.parked_readers,
            None if task.state.is_finished() => 0,
            None => (task.race.iter().flat_map(|r| &r.lanes))
                .filter(|l| lane_holds_reader(run, task, l))
                .count(),
        })
        .sum()
}

/// Whether lane `lane` of task `task` (not in a view) holds a reader slot: in `review`
/// with a live reviewer, a resumable one that owes its verdict, or a `PrepareReview`
/// in flight (`schedule::holds_reader`, per lane). A lane that left the race holds one
/// while its given-up reviewer has not exited (ruling T13-I2). The crowned lane's
/// reviewers are the task's (task 17a's re-review, (a): each reviewer counts once).
pub(super) fn lane_holds_reader(run: &Run, task: &Task, lane: &Lane) -> bool {
    if (task.race.as_ref()).is_some_and(|r| r.crowned && r.winner == Some(lane.lane)) {
        return false;
    }
    let l = Some(lane.lane);
    let reviewer =
        |r: &&crate::run::model::AgentRound| r.role == AgentRole::Reviewer && r.lane == l;
    let live = task.rounds.iter().filter(reviewer).any(|r| !r.ended);
    let resumable = task.rounds.iter().rfind(reviewer).is_some_and(|r| {
        r.ended
            && !r.retiring
            && r.session_id.is_some()
            && !(task.reviews.iter())
                .any(|rv| rv.lane == l && rv.round == r.round && rv.verdict.is_some())
    });
    let preparing = run.pending_ops.values().any(|p| {
        p.task_id.as_deref() == Some(task.id())
            && p.lane == l
            && matches!(p.kind, OpKind::PrepareReview { .. })
    });
    match lane.state {
        LaneState::Review => live || resumable || preparing,
        LaneState::Out | LaneState::Lost => live,
        _ => false,
    }
}

/// The window-name stem of task `task`'s sessions: `<task>.` (`t1.w1`, `t1.r1`), or in
/// a lane's view `<task>.<lane>` (`t1.aw1`, `t1.ar1`; decision 19).
pub(super) fn stem(task: &Task) -> String {
    match task.lane_view {
        Some(lane) => format!("{}.{}", task.id(), lane.label()),
        None => format!("{}.", task.id()),
    }
}

/// Milestone 9.5 decision 20: in a lane's view, rung 2 or 3 takes the lane out of its
/// race (`race_view` reads the view's block as `Out`, with its text): its sessions
/// stopped and its rung kept for the lane. Nothing of the task's own changes (no
/// escalation, no size raised); the action that took the lane out is the adoption's
/// (task M9.5.17b). Minor m7: the lane keeps no block reason (the rung says why), so
/// the view's block only carries the text out.
pub(super) fn lane_out(
    run: &mut Run,
    i: usize,
    rung: u8,
    text: String,
    now: u64,
    fx: &mut Vec<Effect>,
) {
    super::ladder::kill_worker(run, i, fx);
    super::ladder::drop_queued(run, i);
    let task = &mut run.tasks[i];
    task.rung = rung;
    task.fresh_session = None;
    set_state(task, TaskState::Blocked, now);
    let reason = BlockReason::Environment;
    task.block = Some(proto::BlockInfo::new(reason, text));
}
