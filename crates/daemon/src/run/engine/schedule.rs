//! Decision 41's scheduler, as pure functions of a run: runnability, the critical-path
//! order, writer and reader slots, the hub rule, and the snapshot's critical path and
//! waves. Pure — no `std::fs`, `std::process`, `std::thread`, `tokio` or
//! `std::time::SystemTime` (design decision 2).

use std::cmp::Reverse;

use proto::{AgentRole, Size, TaskState};

use super::OpKind;
use crate::run::model::{Run, Task};

/// Decision 41: a writer slot is held from `preparing` through `check` (a handed-back
/// task is `working` again, so it holds one too). `review` and `merge_queue` hold none.
pub fn holds_writer(state: TaskState) -> bool {
    matches!(
        state,
        TaskState::Preparing | TaskState::Working | TaskState::Proof | TaskState::Check
    )
}

pub fn writers_busy(run: &Run) -> usize {
    run.tasks.iter().filter(|t| holds_writer(t.state)).count()
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
pub fn hub_started(run: &Run) -> bool {
    run.tasks.iter().any(|t| {
        t.hub && !t.state.is_finished() && (holds_writer(t.state) || t.start_commit.is_some())
    })
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
    task.state == TaskState::Review
        && (task
            .rounds
            .iter()
            .any(|r| r.role == AgentRole::Reviewer && !r.ended)
            || resumable_reviewer(task)
            || op_in_flight(run, task.id(), |k| {
                matches!(k, OpKind::PrepareReview { .. })
            }))
}

/// The task's last reviewer round, ended but resumable, with no verdict yet.
fn resumable_reviewer(task: &Task) -> bool {
    task.rounds
        .iter()
        .rfind(|r| r.role == AgentRole::Reviewer)
        .is_some_and(|r| {
            r.ended
                && !r.retiring
                && r.session_id.is_some()
                && !task
                    .reviews
                    .iter()
                    .any(|rv| rv.round == r.round && rv.verdict.is_some())
        })
}

pub fn readers_busy(run: &Run) -> usize {
    run.tasks.iter().filter(|t| holds_reader(run, t)).count()
}

/// A task in `review` with no reviewer yet.
pub fn needs_reviewer(run: &Run, task: &Task) -> bool {
    task.state == TaskState::Review && !holds_reader(run, task)
}

fn state_of(run: &Run, id: &str) -> Option<TaskState> {
    run.task(id).map(|t| t.state)
}

/// The dependencies `task` still waits for: declared ones not `merged`, implicit ones
/// neither `merged` nor `cancelled` (decision 41).
pub fn unfinished_deps(run: &Run, task: &Task) -> Vec<String> {
    let declared = task
        .spec
        .deps
        .iter()
        .filter(|d| state_of(run, d) != Some(TaskState::Merged));
    let implicit = task
        .implicit_deps
        .iter()
        .filter(|d| state_of(run, d).is_some_and(|s| !s.is_finished()));
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

/// Decision 41's weights until M9.5's history exists: S = 1, M = 3. An L task never
/// runs (rule 7.2.4), so its weight only orders the snapshot; it counts as M.
pub fn weight(task: &Task) -> u32 {
    match task.size {
        Size::S => 1,
        Size::M | Size::L => 3,
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
pub fn critical_lens(run: &Run) -> Vec<u32> {
    let deps = dependents(run);
    let mut memo: Vec<Option<u32>> = vec![None; run.tasks.len()];
    let mut visiting = vec![false; run.tasks.len()];
    fn visit(
        i: usize,
        run: &Run,
        deps: &[Vec<usize>],
        memo: &mut [Option<u32>],
        visiting: &mut [bool],
    ) -> u32 {
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
        let v = weight(&run.tasks[i]) + tail;
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
