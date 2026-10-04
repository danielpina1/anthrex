//! Decision 41's scheduler, as pure functions of a run: runnability, the critical-path
//! order, writer and reader slots, the hub rule, and the snapshot's critical path and
//! waves. Pure — no `std::fs`, `std::process`, `std::thread`, `tokio` or
//! `std::time::SystemTime` (design decision 2).

use std::cmp::Reverse;

use proto::{AgentRole, LaneState, Runtime, Size, TaskKind, TaskState};

use super::{OpKind, goal_rounds};
use crate::run::model::{Run, RunLimits, Task};

/// Decision 41: a writer slot is held from `preparing` through `check` (a handed-back
/// task is `working` again, so it holds one too). `review` and `merge_queue` hold none.
pub fn holds_writer(state: TaskState) -> bool {
    matches!(
        state,
        TaskState::Preparing | TaskState::Working | TaskState::Proof | TaskState::Check
    )
}

/// Milestone 9 decisions 35 and 36: a research or review task reads and reports; it
/// runs in a reader slot and never holds a writer slot.
pub fn is_reader_task(task: &Task) -> bool {
    matches!(task.spec.kind, TaskKind::Research | TaskKind::Review)
}

/// Milestone 9.5 decision 16 and ruling RR-9: the writer slots busy, counting lanes.
pub fn writers_busy(run: &Run) -> usize {
    run.tasks.iter().map(|t| writer_slots(t).len()).sum()
}

/// The writer slots `task` holds, each with its runtime (decision 16): a racing task one
/// per live lane (ruling RR-9), a paired task's test writer one on the writer's
/// runtime (the final fix wave's A-I3), any other writer task one on its route's
/// runtime. A finished task holds none, whatever its lanes say; a crowned or adopted
/// race is its task's (task M9.5.13's review).
pub fn writer_slots(task: &Task) -> Vec<Runtime> {
    if is_reader_task(task) || task.state.is_finished() {
        return Vec::new();
    }
    // Task 17a's re-review (b): a race `run retry` ended counts as an ordinary task.
    if let Some(race) = task
        .race
        .as_ref()
        .filter(|r| r.winner.is_none() && !r.ended)
    {
        return (race.lanes.iter())
            .filter(|l| lane_holds_writer(l.state))
            .map(|l| l.route.runtime)
            .collect();
    }
    // Minor m5, ruling T17a-5: between `Won` and the crown, the race's one slot is the
    // winning lane's, on that lane's runtime; the crown (adoption included) swaps the
    // lane's route in, and from then on the task's route is the runtime.
    let won = (task.race.iter().filter(|r| !r.crowned))
        .find_map(|r| r.lanes.iter().find(|l| Some(l.lane) == r.winner));
    // The final fix wave's A-I3: a paired task's test writer holds its slot on its own
    // route's runtime while it writes.
    let writer = (task.pair.as_ref()).filter(|_| super::pair::writing(task));
    let runtime = match (won, writer) {
        (Some(lane), _) => lane.route.runtime,
        (None, Some(pair)) => pair.writer_route.runtime,
        (None, None) => task.route.runtime,
    };
    match holds_writer(task.state) {
        true => vec![runtime],
        false => Vec::new(),
    }
}

/// A lane holds a writer slot in the phases a task does ([`holds_writer`]).
fn lane_holds_writer(state: LaneState) -> bool {
    matches!(
        state,
        LaneState::Preparing | LaneState::Working | LaneState::Proof | LaneState::Check
    )
}

/// A hub task holds a writer slot: nothing else starts, reviews included (decision
/// 41, "hub alone").
pub fn hub_holds_slot(run: &Run) -> bool {
    run.tasks.iter().any(|t| t.hub && holds_writer(t.state))
}

/// A hub task holds the hub from `preparing` until it is merged or cancelled: no other
/// writer starts meanwhile. Final review A-I2: a hub task in `review`, `merge_queue`
/// or `blocked` holds no writer slot, yet comes back to `working` (a rejection, a
/// hand-back, an answer), so once started it keeps the hub. Its own reviewer, and
/// other tasks' reviewers, still start ([`hub_holds_slot`] governs those).
///
/// F3 review N1: a hub task held on a dependency it gained (M8a.6 ruling N5,
/// `awaiting_deps`) does not hold it: that dependency must run, or the run freezes.
/// While it is held only its unfinished dependencies start ([`held_hub_waits_for`]).
pub fn hub_started(run: &Run) -> bool {
    run.tasks.iter().any(|t| {
        t.hub
            && !t.state.is_finished()
            && (holds_writer(t.state) || t.start_commit.is_some())
            && !held_on_deps(run, t)
    })
}

/// A started task carrying M8a.6's hold with a dependency still unfinished.
fn held_on_deps(run: &Run, task: &Task) -> bool {
    task.awaiting_deps && !deps_done(run, task)
}

/// F3 review N1: while a started hub task is held on unfinished dependencies, the only
/// tasks that may start are those it waits for, transitively; `None` when no hub task
/// is held.
pub fn held_hub_waits_for(run: &Run) -> Option<Vec<String>> {
    let mut waits: Vec<String> = run
        .tasks
        .iter()
        .filter(|t| t.hub && !t.state.is_finished() && held_on_deps(run, t))
        .flat_map(|t| unfinished_deps(run, t))
        .collect();
    if waits.is_empty() {
        return None;
    }
    let mut next = 0;
    while next < waits.len() {
        if let Some(task) = run.tasks.iter().find(|t| t.id() == waits[next]) {
            for dep in unfinished_deps(run, task) {
                if !waits.contains(&dep) {
                    waits.push(dep);
                }
            }
        }
        next += 1;
    }
    Some(waits)
}

/// Final review A-I2: a task in `review` or `merge_queue` holds no writer slot but can
/// come back to `working` on its own (a rejection, a hand-back), so a hub task does
/// not start beside it. A `blocked` task comes back only through the user (an answer,
/// a retry, an override) and does not hold a hub task back (recorded residual).
pub fn may_return_to_working(state: TaskState) -> bool {
    matches!(state, TaskState::Review | TaskState::MergeQueue)
}

/// Whether an op of `task` matching `pred` is in flight.
pub fn op_in_flight(run: &Run, task: &str, pred: impl Fn(&OpKind) -> bool) -> bool {
    run.pending_ops
        .values()
        .any(|p| p.task_id.as_deref() == Some(task) && pred(&p.kind))
}

/// A task in `review` holds a reader slot from its `PrepareReview` until its reviewer
/// round ends (decision 41: "held by a live reviewer"). A reviewer given up or retired
/// holds it until its process has exited (ruling T13-I2), so the next round never
/// starts beside it. A reviewer the restart ended still holds it while it owes its
/// verdict and can be resumed (M8a.15, decision 45): it is resumed, not replaced.
pub fn holds_reader(run: &Run, task: &Task) -> bool {
    // Milestone 9 decision 35: a research task holds its slot while it works.
    if task.state == TaskState::Working && task.spec.kind == TaskKind::Research {
        return true;
    }
    // Milestone 9.5 (task 17a's re-review, (a)): a crowned race's other lanes hold
    // their own slots (`race::lane_readers`); only the task's own count here.
    let own = |lane: Option<proto::RaceLane>| own_lane(task, lane);
    task.state == TaskState::Review
        && (task
            .rounds
            .iter()
            .any(|r| r.role == AgentRole::Reviewer && !r.ended && own(r.lane))
            || resumable_reviewer(task)
            || run.pending_ops.values().any(|p| {
                p.task_id.as_deref() == Some(task.id())
                    && own(p.lane)
                    // Milestone 9 decision 36: a review task's target first.
                    && matches!(
                        p.kind,
                        OpKind::PrepareReview { .. } | OpKind::ResolveTarget { .. }
                    )
            }))
}

/// Whether a round, record or op of `lane` is task `task`'s own: in a lane's view every
/// one it shows is; otherwise one of no lane, or of the crowned lane.
fn own_lane(task: &Task, lane: Option<proto::RaceLane>) -> bool {
    task.lane_view.is_some() || lane.is_none() || lane == task.crowned_lane()
}

/// The task's last reviewer round, ended but resumable, with no verdict yet.
fn resumable_reviewer(task: &Task) -> bool {
    task.rounds
        .iter()
        .rfind(|r| r.role == AgentRole::Reviewer && own_lane(task, r.lane))
        .is_some_and(|r| {
            r.ended
                && !r.retiring
                && r.session_id.is_some()
                && !task
                    .reviews
                    .iter()
                    .any(|rv| rv.lane == r.lane && rv.round == r.round && rv.verdict.is_some())
        })
}

/// Live reviewers and, since M8b decision 18, deciders in flight; since milestone 9
/// (decision 31), live sub-planners and run scouts.
pub fn readers_busy(run: &Run) -> usize {
    let deciders = run
        .pending_ops
        .values()
        .filter(|p| matches!(p.kind, OpKind::Decide { .. }))
        .count();
    run.tasks.iter().filter(|t| holds_reader(run, t)).count()
        + deciders
        + super::planners::readers(run)
        // Milestone 9.6 decision 9: the design agents.
        + super::design_agents::readers(run)
        // Milestone 9.5 decision 20: each lane's review.
        + super::race::lane_readers(run)
}

/// A task in `review` with no reviewer yet.
pub fn needs_reviewer(run: &Run, task: &Task) -> bool {
    task.state == TaskState::Review && !holds_reader(run, task)
}

fn state_of(run: &Run, id: &str) -> Option<TaskState> {
    run.task(id).map(|t| t.state)
}

/// The dependencies `task` still waits for: declared ones neither `merged` nor, since
/// milestone 9 (decision 35), `reported`; implicit ones neither `merged` nor
/// `cancelled` (decision 41). Research and review tasks own nothing, so they are never
/// an implicit dependency.
pub fn unfinished_deps(run: &Run, task: &Task) -> Vec<String> {
    // Milestone 9.3 decision 13: a task of an earlier round is a met dependency.
    let earlier = |d: &&String| {
        run.task(d)
            .is_some_and(|d| goal_rounds::dep_met(run, task, d))
    };
    let declared = task.spec.deps.iter().filter(|d| {
        !earlier(d)
            && !matches!(
                state_of(run, d),
                Some(TaskState::Merged | TaskState::Reported)
            )
    });
    let implicit = task
        .implicit_deps
        .iter()
        .filter(|d| !earlier(d) && state_of(run, d).is_some_and(|s| !s.is_finished()));
    let mut out: Vec<String> = Vec::new();
    for dep in declared.chain(implicit) {
        if !out.contains(dep) {
            out.push(dep.clone());
        }
    }
    out
}

pub fn deps_done(run: &Run, task: &Task) -> bool {
    unfinished_deps(run, task).is_empty()
}

/// M8b decision 19: the task waits for its size cross-check, so it is not runnable.
pub fn size_check_pending(task: &Task) -> bool {
    matches!(
        task.size_check,
        Some(crate::run::model::SizeCheckState::Pending { .. })
    )
}

/// A task's weight on the critical path. Milestone 9.5 decision 7: its class's seconds
/// when the run froze history's weights (a hub task the hub's), else decision 41's
/// S = 1, M = 3. An L task never runs (rule 7.2.4), so its weight only orders the
/// snapshot; it counts as M. The one weight function: `run::estimate` calls it too.
pub fn weight(limits: &RunLimits, task: &Task) -> u64 {
    match (&limits.path_weights, task.size) {
        (Some(w), _) if task.hub => w.hub_secs,
        (Some(w), Size::S) => w.s_secs,
        (Some(w), Size::M | Size::L) => w.m_secs,
        (None, Size::S) => 1,
        (None, Size::M | Size::L) => 3,
    }
}

fn index(run: &Run, id: &str) -> Option<usize> {
    run.tasks.iter().position(|t| t.id() == id)
}

/// For each task, the unfinished tasks that wait for it, declared or implicit.
fn dependents(run: &Run) -> Vec<Vec<usize>> {
    let mut out = vec![Vec::new(); run.tasks.len()];
    for (j, task) in run.tasks.iter().enumerate() {
        if task.state.is_finished() {
            continue;
        }
        for dep in task.spec.deps.iter().chain(&task.implicit_deps) {
            if let Some(i) = index(run, dep)
                && !out[i].contains(&j)
            {
                out[i].push(j);
            }
        }
    }
    out
}

/// `critical_len(t) = weight(t) + max(critical_len(d))` over the unfinished tasks that
/// depend on `t`; 0 for a finished task. The combined graph is acyclic (M8a.5's
/// backstop), and a task on a cycle anyway counts its own weight only.
pub fn critical_lens(run: &Run) -> Vec<u64> {
    let deps = dependents(run);
    let mut memo: Vec<Option<u64>> = vec![None; run.tasks.len()];
    let mut visiting = vec![false; run.tasks.len()];
    fn visit(
        i: usize,
        run: &Run,
        deps: &[Vec<usize>],
        memo: &mut [Option<u64>],
        visiting: &mut [bool],
    ) -> u64 {
        if let Some(v) = memo[i] {
            return v;
        }
        if run.tasks[i].state.is_finished() || visiting[i] {
            return 0;
        }
        visiting[i] = true;
        let tail = deps[i]
            .iter()
            .map(|&j| visit(j, run, deps, memo, visiting))
            .max()
            .unwrap_or(0);
        visiting[i] = false;
        let v = weight(&run.limits, &run.tasks[i]).saturating_add(tail);
        memo[i] = Some(v);
        v
    }
    (0..run.tasks.len())
        .map(|i| visit(i, run, &deps, &mut memo, &mut visiting))
        .collect()
}

/// Every task index in dispatch order: `critical_len` descending, then `priority`
/// descending, then plan order.
pub fn dispatch_order(run: &Run) -> Vec<usize> {
    let lens = critical_lens(run);
    let mut order: Vec<usize> = (0..run.tasks.len()).collect();
    order.sort_by_key(|&i| (Reverse(lens[i]), Reverse(run.tasks[i].spec.priority), i));
    order
}

/// The chain of maximal `critical_len` among unfinished tasks: the first unfinished task
/// in dispatch order, then repeatedly its dependent with the longest remaining chain.
pub fn critical_path(run: &Run) -> Vec<usize> {
    let lens = critical_lens(run);
    let order = dispatch_order(run);
    let rank = |i: usize| order.iter().position(|&o| o == i).unwrap_or(usize::MAX);
    let deps = dependents(run);
    let Some(mut at) = order
        .iter()
        .copied()
        .find(|&i| !run.tasks[i].state.is_finished())
    else {
        return Vec::new();
    };
    let mut path = vec![at];
    while let Some(next) = deps[at]
        .iter()
        .copied()
        .filter(|j| !path.contains(j))
        .min_by_key(|&j| (Reverse(lens[j]), rank(j)))
    {
        path.push(next);
        at = next;
    }
    path
}

/// The longest dependency chain before each task (decision 41's `wave`): 0 with no
/// dependency, else one more than its deepest dependency, declared or implicit, merged
/// ones included; cancelled dependencies do not count.
pub fn waves(run: &Run) -> Vec<u32> {
    let mut memo: Vec<Option<u32>> = vec![None; run.tasks.len()];
    let mut visiting = vec![false; run.tasks.len()];
    fn visit(i: usize, run: &Run, memo: &mut [Option<u32>], visiting: &mut [bool]) -> u32 {
        if let Some(v) = memo[i] {
            return v;
        }
        if visiting[i] {
            return 0;
        }
        visiting[i] = true;
        let task = &run.tasks[i];
        let v = task
            .spec
            .deps
            .iter()
            .chain(&task.implicit_deps)
            .filter_map(|d| index(run, d))
            .filter(|&d| run.tasks[d].state != TaskState::Cancelled)
            .map(|d| visit(d, run, memo, visiting) + 1)
            .max()
            .unwrap_or(0);
        visiting[i] = false;
        memo[i] = Some(v);
        v
    }
    (0..run.tasks.len())
        .map(|i| visit(i, run, &mut memo, &mut visiting))
        .collect()
}
