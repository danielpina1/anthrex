//! The run counts every client shows, defined once (milestone 9.19, decisions 5 and 7;
//! rulings R13 and R14). Pure.

use proto::{RunInfo, TaskInfo, TaskState};

/// How far a run is: tasks that can still merge, and those that have.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Progress {
    /// Tasks in the `Merged` state.
    pub merged: usize,
    /// Tasks that count: every task but a cancelled, reported or replaced one.
    pub total: usize,
    /// Cancelled tasks (replaced ones included), counted apart, in neither number above.
    pub cancelled: usize,
}

/// Whether a task in `state` is in `merged/total`. A `Cancelled` task (a replaced task
/// is always cancelled) and a `Reported` one (a research or review task that merges
/// nothing) are left out, and so is any task with a replacement (`replaced`).
pub fn counts(state: TaskState, replaced: bool) -> bool {
    !replaced && !matches!(state, TaskState::Cancelled | TaskState::Reported)
}

/// [`Progress`] over `tasks`, every round's together.
pub fn tally<'a>(tasks: impl IntoIterator<Item = &'a TaskInfo>) -> Progress {
    let mut progress = Progress::default();
    for task in tasks {
        if task.state == TaskState::Cancelled {
            progress.cancelled += 1;
        }
        if counts(task.state, !task.replaced_by.is_empty()) {
            progress.total += 1;
            progress.merged += usize::from(task.state == TaskState::Merged);
        }
    }
    progress
}

/// [`Progress`] over all of a run's tasks.
pub fn run_progress(run: &RunInfo) -> Progress {
    tally(&run.tasks)
}

/// The number of stages a run has: `run.stages.len()`, or the highest stage a task is
/// planned into when that is larger, and at least 1. Mirrors the daemon's count
/// (`daemon/src/run/snapshot_stages.rs::stage_count`), whose `stages` snapshot has one
/// entry per stage from 1 to that highest, so its length is that count. Every position
/// reads `stage k of <this>`.
pub fn stage_count(run: &RunInfo) -> usize {
    let planned = run.tasks.iter().map(|t| t.stage).max().unwrap_or(1);
    run.stages.len().max(usize::from(planned)).max(1)
}

#[cfg(test)]
#[path = "progress_tests.rs"]
mod tests;
