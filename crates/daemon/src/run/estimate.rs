//! Milestone 9.5 decisions 14 and 15 (rulings RE-1 to RE-4): the run estimate and its
//! bound ratio, round-aware. Pure: no I/O and no clock (design decision 2).
//!
//! Over the **counted tasks** (the current round's, plus earlier rounds' unfinished
//! fix tasks; never a cancelled one; none counted, no estimate), with `w(t)` =
//! `schedule::weight` and `done(t)` its active seconds so far, never paused time
//! (ruling T12-2):
//! - `remaining(t)` = 0 once merged or reported, else `max(w − done, ⌈w / 10⌉)`;
//! - `left = max(RCP, ⌈Σ_writers remaining / max_writers⌉)`, RCP the longest path of
//!   `remaining` over the unfinished counted tasks; readers count on paths only (RE-3);
//! - `bound = max(CP, ⌈Σ_writers w / max_writers⌉)`, CP the longest path of `w`;
//! - `elapsed` runs from the round's approval, less paused time (decision 15) and, in
//!   `pr` mode, the round's stages' human review time (RE-2);
//! - `bound_ratio_permille = (elapsed + left) × 1000 / max(bound, 1)`.

use proto::{DeliveryMode, TaskState};

use std::collections::HashMap;

use super::engine::pause::active;
use super::engine::schedule::{is_reader_task, weight};
use super::model::{Run, Task};
use super::refit::active_secs;
use super::snapshot_stages::stage_count;

/// What is left of the current round, and how it stands against its bound.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Estimate {
    pub left_secs: u64,
    pub bound_ratio_permille: Option<u32>,
}

/// Decision 14's estimate of `run` at `now`: `None` without history's weights, before
/// the current round is approved, with no counted task (ruling T12-3), and while a
/// `pr` run is delivering with no task running.
pub fn estimate(run: &Run, now: u64) -> Option<Estimate> {
    run.limits.path_weights.as_ref()?;
    let approved = approved_at(run)?;
    if run.delivery.delivering(stage_count(run)) && !run.tasks.iter().any(running) {
        return None;
    }
    // A round that ended stops its clock (its tasks are all finished by then).
    let end = run
        .current_round()
        .and_then(|r| r.ended_at)
        .map_or(now, |at| at.min(now));
    let counted = counted(run);
    if counted.is_empty() {
        return None;
    }
    let w: Vec<u64> = (run.tasks.iter()).map(|t| weight(&run.limits, t)).collect();
    let remaining: Vec<u64> = (run.tasks.iter().enumerate())
        .map(|(i, t)| remaining(run, t, w[i], end))
        .collect();
    let writers = u64::from(run.limits.max_writers.max(1));
    let work = |of: &[u64]| {
        let sum = (counted.iter())
            .filter(|&&i| !is_reader_task(&run.tasks[i]))
            .fold(0u64, |sum, &i| sum.saturating_add(of[i]));
        sum.div_ceil(writers)
    };
    let unfinished: Vec<usize> = (counted.iter().copied())
        .filter(|&i| !run.tasks[i].state.is_finished())
        .collect();
    let left = longest(run, &unfinished, &remaining).max(work(&remaining));
    let bound = longest(run, &counted, &w).max(work(&w));
    let elapsed = elapsed(run, (approved, paused_before(run)), end);
    let ratio = elapsed.saturating_add(left).saturating_mul(1000) / bound.max(1);
    Some(Estimate {
        left_secs: left,
        bound_ratio_permille: u32::try_from(ratio).ok(),
    })
}

/// The run's paused seconds at the current round's approval (decision 15).
fn paused_before(run: &Run) -> u64 {
    run.current_round().map_or(0, |r| r.paused_before)
}

/// The current round's approval (ruling RE-1): round 1's is the run's.
fn approved_at(run: &Run) -> Option<u64> {
    match run.current_round() {
        Some(round) if round.n > 1 => round.approved_at,
        _ => run.approved_at,
    }
}

/// Ruling RE-2's "running": a task queued for, or in, a writer or reader slot, a gate
/// or the merge queue.
fn running(task: &Task) -> bool {
    !task.state.is_finished() && !matches!(task.state, TaskState::Pending | TaskState::Blocked)
}

/// The counted tasks' indices (ruling RE-1).
fn counted(run: &Run) -> Vec<usize> {
    let n = run.round();
    (run.tasks.iter().enumerate())
        .filter(|(_, t)| t.state != TaskState::Cancelled)
        .filter(|(_, t)| {
            t.round == n || (t.round < n && t.fixes.is_some() && !t.state.is_finished())
        })
        .map(|(i, _)| i)
        .collect()
}

/// `remaining(t)`: none once merged or reported, else what its weight leaves of its
/// active seconds, at least a tenth of the weight.
fn remaining(run: &Run, task: &Task, w: u64, end: u64) -> u64 {
    if task.state.is_finished() {
        return 0;
    }
    w.saturating_sub(done(run, task, end)).max(w.div_ceil(10))
}

/// Decision 5's active seconds, plus the open phase's when it is an active one, less
/// the run's paused time inside them (ruling T12-2).
fn done(run: &Run, task: &Task, end: u64) -> u64 {
    let closed = active_secs(&task.phases).saturating_sub(task.paused.active);
    let open = if active(task.state) && task.phase_since > 0 {
        let paused = run.paused_total(end).saturating_sub(task.paused.base);
        end.saturating_sub(task.phase_since).saturating_sub(paused)
    } else {
        0
    };
    closed.saturating_add(open)
}

/// The longest path of `value` over `nodes`, along declared and implicit dependencies
/// between them. A cycle (which validation forbids) counts each task once.
fn longest(run: &Run, nodes: &[usize], value: &[u64]) -> u64 {
    let index: HashMap<&str, usize> = (nodes.iter()).map(|&i| (run.tasks[i].id(), i)).collect();
    // For each node, the nodes that depend on it.
    let mut next: Vec<Vec<usize>> = vec![Vec::new(); run.tasks.len()];
    for &j in nodes {
        let task = &run.tasks[j];
        for dep in task.spec.deps.iter().chain(&task.implicit_deps) {
            if let Some(&i) = index.get(dep.as_str()).filter(|&&i| i != j) {
                next[i].push(j);
            }
        }
    }
    let mut memo: Vec<Option<u64>> = vec![None; run.tasks.len()];
    let mut visiting = vec![false; run.tasks.len()];
    fn visit(
        i: usize,
        (next, value): (&[Vec<usize>], &[u64]),
        memo: &mut [Option<u64>],
        visiting: &mut [bool],
    ) -> u64 {
        if let Some(v) = memo[i] {
            return v;
        }
        if visiting[i] {
            return 0;
        }
        visiting[i] = true;
        let tail = (next[i].iter())
            .map(|&j| visit(j, (next, value), memo, visiting))
            .max()
            .unwrap_or(0);
        visiting[i] = false;
        let v = value[i].saturating_add(tail);
        memo[i] = Some(v);
        v
    }
    (nodes.iter())
        .map(|&i| visit(i, (&next, value), &mut memo, &mut visiting))
        .max()
        .unwrap_or(0)
}

/// Seconds since the round's approval, less the time paused since (decision 15) and,
/// in `pr` mode, the round's stages' human review time (ruling RE-2), with the wait in
/// progress up to a pause and less the wait inside past pauses, so no wait is taken
/// off twice.
fn elapsed(run: &Run, (approved, paused_before): (u64, u64), end: u64) -> u64 {
    let paused = run.paused_total(end).saturating_sub(paused_before);
    let mut secs = end.saturating_sub(approved).saturating_sub(paused);
    let until = run.paused_at.map_or(end, |at| at.min(end));
    if run.delivery.mode == DeliveryMode::Pr {
        let n = run.round();
        let review = (1..=stage_count(run))
            .filter(|&s| run.round_of_stage(s) == n)
            .filter_map(|s| run.delivery.stage(s))
            .map(|d| {
                let open = d.wait_from.map_or(0, |from| until.saturating_sub(from));
                // Ruling FW-2 (b): the wait inside a pause is already paused time.
                (d.review_wait_secs.saturating_add(open)).saturating_sub(d.review_paused_secs)
            })
            .fold(0u64, u64::saturating_add);
        secs = secs.saturating_sub(review);
    }
    secs
}

#[cfg(test)]
#[path = "estimate_tests.rs"]
mod tests;
