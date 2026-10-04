//! Milestone 9.5 decisions 19–20 (rulings RR-3, RR-9, T1-3): the view one lane of a
//! racing task runs its gates in. While the reducer handles an event or a scheduler
//! pass of lane `x`, the task *is* that lane to every existing function: its state is
//! the lane's, its checkout and branch are the lane's (`<task>.<x>`,
//! `anthrex/<run>/<task>.<x>`), its gate state is the lane's (`Lane` and its
//! `LaneGates`), and it shows the lane's rounds, records, ops and messages only. So the
//! done gate, the proof, tier 1 or the check, the weakening signals, the review, the
//! ladder's rung 1, the stall clock and the budgets run unchanged, once per lane, and
//! every op, round and record they make carries the lane (`PendingOp.lane`,
//! `AgentRound.lane`, …). On leaving, the task gets its own fields back and the lane
//! keeps what changed; the lane's spend is added to the task's `spent_total`.
//!
//! A lane's outbox messages are addressed `<task>.<x>` (its racer) and
//! `<task>.<x>.review` (its reviewer): no task id contains `.` (M8a decision 16), so
//! outside the view nothing matches them. A lane that is out or lost is shown as a
//! `cancelled` task: nothing it sends moves on, and a window it is given is killed.
//! Pure (design decision 1).

use std::mem::{replace, swap, take};

use proto::{AgentRole, LaneState, RaceLane, Spend, TaskState};

use super::Effect;
use crate::run::model::{Lane, PendingOp, Run, Task, task_branch};

/// How a lane's view ended: as it was, past its last pre-merge gate (the view
/// reached the merge queue), or out of the race (it was blocked, with the reason).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum Exit {
    Stay,
    Passed,
    Out(String),
}

/// `lane` of `task`, if its events go through its view: the task races and the lane
/// is not the one that became the task (crowned). A winner not crowned yet is shown
/// its own view, where nothing it sends moves on (decision 21: no import runs between
/// `Won` and the crown, even after a `RefMoved` and a resume).
pub(super) fn view_lane(task: &Task, lane: Option<RaceLane>) -> Option<RaceLane> {
    let lane = lane?;
    let race = task.race.as_ref()?;
    let known = race.lanes.iter().any(|l| l.lane == lane);
    let became = race.crowned && race.winner == Some(lane);
    (known && !became).then_some(lane)
}

/// A lane still in the race: preparing, working or in a pre-merge gate.
pub(super) fn live(state: LaneState) -> bool {
    matches!(
        state,
        LaneState::Preparing
            | LaneState::Working
            | LaneState::Proof
            | LaneState::Check
            | LaneState::Review
    )
}

/// The task state a lane in `state` shows (an ended lane: `cancelled`, inert).
fn task_state(state: LaneState) -> TaskState {
    match state {
        LaneState::Preparing => TaskState::Preparing,
        LaneState::Working => TaskState::Working,
        LaneState::Proof => TaskState::Proof,
        LaneState::Check => TaskState::Check,
        LaneState::Review => TaskState::Review,
        _ => TaskState::Cancelled,
    }
}

/// The outbox address of lane `lane` of task `id` that `address` (the task's own
/// address in the view: `<task>` or `<task>.review`) stands for.
fn lane_address(id: &str, lane: RaceLane, address: &str) -> Option<String> {
    let label = lane.label();
    match address.strip_prefix(id)? {
        "" => Some(format!("{id}.{label}")),
        ".review" => Some(format!("{id}.{label}.review")),
        _ => None,
    }
}

/// Whether `address` is task `id`'s, or one of its lanes' (`<id>`, `<id>.review`,
/// `<id>.<x>`, `<id>.<x>.review`).
fn task_address(id: &str, address: &str) -> bool {
    matches!(
        address.strip_prefix(id),
        Some("" | ".review" | ".a" | ".b" | ".a.review" | ".b.review")
    )
}

/// The lane an outbox address of task `id` belongs to, if a lane's.
pub(super) fn address_lane(id: &str, address: &str) -> Option<RaceLane> {
    match address.strip_prefix(id)? {
        ".a" | ".a.review" => Some(RaceLane::A),
        ".b" | ".b.review" => Some(RaceLane::B),
        _ => None,
    }
}

/// Items a view does not show, each with its index in the whole list.
struct Parked<T> {
    others: Vec<(usize, T)>,
    total: usize,
}

/// Keeps in `items` what `mine` accepts, in order, and parks the rest.
fn park<T>(items: &mut Vec<T>, mine: impl Fn(&T) -> bool) -> Parked<T> {
    let all = take(items);
    let total = all.len();
    let mut others = Vec::new();
    for (k, item) in all.into_iter().enumerate() {
        match mine(&item) {
            true => items.push(item),
            false => others.push((k, item)),
        }
    }
    Parked { others, total }
}

/// Puts the parked items back where they were; what the view added comes last,
/// marked by `tag`.
fn unpark<T>(items: &mut Vec<T>, parked: Parked<T>, mut tag: impl FnMut(&mut T)) {
    let mut shown = take(items).into_iter();
    let mut others = parked.others.into_iter().peekable();
    for k in 0..parked.total {
        match others.peek() {
            Some((at, _)) if *at == k => items.extend(others.next().map(|(_, t)| t)),
            _ => items.extend(shown.next()),
        }
    }
    for mut item in shown {
        tag(&mut item);
        items.push(item);
    }
}

/// The fields a lane keeps for itself, exchanged with the task's: calling it twice
/// gives each its own back.
fn exchange(task: &mut Task, lane: &mut Lane) {
    swap(&mut task.start_commit, &mut lane.start_commit);
    swap(&mut task.head, &mut lane.head);
    swap(&mut task.done, &mut lane.done);
    swap(&mut task.failures, &mut lane.failures);
    swap(&mut task.bounces, &mut lane.bounces);
    swap(&mut task.stalls, &mut lane.stalls);
    swap(&mut task.budget_exceeded, &mut lane.budget_exceeded);
    swap(&mut task.spent_total, &mut lane.spent);
    swap(&mut task.route, &mut lane.route);
    swap(&mut task.review_route, &mut lane.review_route);
    let gates = &mut lane.gates;
    swap(&mut task.rung, &mut gates.rung);
    swap(&mut task.gate_op, &mut gates.gate_op);
    swap(&mut task.claim, &mut gates.claim);
    swap(&mut task.review_misses, &mut gates.review_misses);
    swap(&mut task.pending_failure, &mut gates.pending_failure);
    swap(&mut task.signals, &mut gates.signals);
    swap(&mut task.signals_more, &mut gates.signals_more);
    swap(&mut task.signal_refusals, &mut gates.signal_refusals);
    swap(&mut task.fresh_session, &mut gates.fresh_session);
    swap(&mut task.ready_from, &mut gates.ready_from);
    swap(&mut task.worktree_live, &mut gates.worktree_live);
    swap(&mut task.clock, &mut gates.clock);
}

/// What a view holds while it is open: the task's own values of the fields the lane
/// shows, and everything of the task the lane's view does not show.
struct Held {
    lane: Lane,
    entry: LaneState,
    state: TaskState,
    block: Option<proto::BlockInfo>,
    worktree: std::path::PathBuf,
    branch: String,
    phases: proto::PhaseSecs,
    phase_since: u64,
    paused: crate::run::model::TaskPaused,
    epoch: Option<super::BudgetEpoch>,
    spent: Spend,
    history: usize,
    rounds: Parked<crate::run::model::AgentRound>,
    proofs: Parked<crate::run::model::ProofRecord>,
    checks: Parked<crate::run::model::CheckRecord>,
    reviews: Parked<crate::run::model::ReviewRecord>,
    ops: Vec<PendingOp>,
    mail: Vec<crate::run::model::Outgoing>,
}

/// Runs `f` with task `i` shown as its lane `lane`; `None` when the task has no such
/// lane. Returns `f`'s result and how the view ended.
pub(super) fn in_lane<R>(
    run: &mut Run,
    i: usize,
    lane: RaceLane,
    fx: &mut Vec<Effect>,
    f: impl FnOnce(&mut Run, &mut Vec<Effect>) -> R,
) -> Option<(R, Exit)> {
    let k = (run.tasks[i].race.as_ref())?
        .lanes
        .iter()
        .position(|l| l.lane == lane)?;
    let held = enter(run, i, k);
    let result = f(run, fx);
    Some((result, leave(run, i, k, held)))
}

fn enter(run: &mut Run, i: usize, k: usize) -> Held {
    let id = run.tasks[i].id().to_string();
    let mut lane = run.tasks[i].race.as_ref().expect("a race").lanes[k].clone();
    let label = lane.lane;
    let path = run.task_path(&lane.checkout);
    let branch = task_branch(&run.id, &lane.checkout);
    // The ops and messages of the task's other lanes, and its own, wait outside.
    let parked: Vec<u64> = (run.pending_ops.values())
        .filter(|p| p.task_id.as_deref() == Some(id.as_str()) && p.lane != Some(label))
        .map(|p| p.op)
        .collect();
    let ops = (parked.iter())
        .filter_map(|op| run.pending_ops.remove(op))
        .collect();
    let mut mail = Vec::new();
    for message in take(&mut run.outbox) {
        if !task_address(&id, &message.task_id) {
            run.outbox.push(message);
        } else if address_lane(&id, &message.task_id) == Some(label) {
            let own = match message.task_id.ends_with(".review") {
                true => format!("{id}.review"),
                false => id.clone(),
            };
            run.outbox.push(crate::run::model::Outgoing {
                task_id: own,
                ..message
            });
        } else {
            mail.push(message);
        }
    }
    let entry = lane.state;
    let task = &mut run.tasks[i];
    exchange(task, &mut lane);
    let held = Held {
        entry,
        state: replace(&mut task.state, task_state(entry)),
        block: task.block.take(),
        worktree: replace(&mut task.worktree, path),
        branch: replace(&mut task.branch, branch),
        phases: task.phases,
        phase_since: task.phase_since,
        paused: task.paused,
        epoch: task.epoch.take(),
        spent: task.spent_total,
        history: task.history.len(),
        rounds: park(&mut task.rounds, |r| r.lane == Some(label)),
        proofs: park(&mut task.proofs, |r| r.lane == Some(label)),
        checks: park(&mut task.checks, |r| r.lane == Some(label)),
        reviews: park(&mut task.reviews, |r| r.lane == Some(label)),
        lane,
        ops,
        mail,
    };
    task.lane_view = Some(label);
    held
}

fn leave(run: &mut Run, i: usize, k: usize, held: Held) -> Exit {
    let id = run.tasks[i].id().to_string();
    let task = &mut run.tasks[i];
    let label = task.lane_view.take().expect("a lane's view");
    let (shown, block) = (task.state, task.block.take());
    unpark(&mut task.rounds, held.rounds, |r| r.lane = Some(label));
    unpark(&mut task.proofs, held.proofs, |r| r.lane = Some(label));
    unpark(&mut task.checks, held.checks, |r| r.lane = Some(label));
    unpark(&mut task.reviews, held.reviews, |r| r.lane = Some(label));
    for event in task.history.iter_mut().skip(held.history) {
        event.text = format!("racer {}: {}", label.label(), event.text);
    }
    let session = (task.rounds.iter())
        .rfind(|r| r.lane == Some(label) && r.role == AgentRole::Racer)
        .map(|r| r.session);
    task.state = held.state;
    task.block = held.block;
    task.worktree = held.worktree;
    task.branch = held.branch;
    task.phases = held.phases;
    task.phase_since = held.phase_since;
    task.paused = held.paused;
    task.epoch = held.epoch;
    let mut lane = held.lane;
    exchange(task, &mut lane);
    let grown = |now: u64, then: u64| now.saturating_sub(then);
    task.spent_total.tool_calls = (task.spent_total.tool_calls)
        .saturating_add(lane.spent.tool_calls.saturating_sub(held.spent.tool_calls));
    task.spent_total.secs =
        (task.spent_total.secs).saturating_add(grown(lane.spent.secs, held.spent.secs));
    task.spent_total.tokens =
        (task.spent_total.tokens).saturating_add(grown(lane.spent.tokens, held.spent.tokens));
    if let Some(session) = session {
        lane.session = session;
    }
    let exit = match live(held.entry) {
        false => Exit::Stay,
        true => match shown {
            TaskState::Preparing => keep(&mut lane, LaneState::Preparing),
            TaskState::Working => keep(&mut lane, LaneState::Working),
            TaskState::Proof => keep(&mut lane, LaneState::Proof),
            TaskState::Check => keep(&mut lane, LaneState::Check),
            TaskState::Review => keep(&mut lane, LaneState::Review),
            TaskState::MergeQueue => Exit::Passed,
            TaskState::Blocked => {
                let reason = block.as_ref().map(|b| b.text.clone()).unwrap_or_default();
                lane.gates.block = block;
                Exit::Out(reason)
            }
            other => Exit::Out(format!("its view became {}", other.label())),
        },
    };
    if let Some(race) = run.tasks[i].race.as_mut() {
        race.lanes[k] = lane;
    }
    // What the view sent is the lane's; what waited outside comes back.
    for pending in run.pending_ops.values_mut() {
        if pending.task_id.as_deref() == Some(id.as_str()) && pending.lane.is_none() {
            pending.lane = Some(label);
        }
    }
    for pending in held.ops {
        run.pending_ops.insert(pending.op, pending);
    }
    for message in run.outbox.iter_mut() {
        if let Some(address) = lane_address(&id, label, &message.task_id) {
            message.task_id = address;
        }
    }
    run.outbox.extend(held.mail);
    exit
}

fn keep(lane: &mut Lane, state: LaneState) -> Exit {
    lane.state = state;
    Exit::Stay
}

/// Decisions 20–21: the crowned (or adopted) lane becomes the task. The lane's own
/// fields are the task's from now on; the task keeps its whole spend, both lanes'.
pub(super) fn become_lane(task: &mut Task, lane: &Lane) {
    let spent = task.spent_total;
    let mut own = lane.clone();
    exchange(task, &mut own);
    task.spent_total = spent;
}
