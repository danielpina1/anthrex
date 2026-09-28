//! Milestone 9's plan edits: decision 25's `amend_task deps` (task M9.4) and, from
//! task M9.13a, decision 42's `message` and `refresh`. `run/edits.rs` delegates to
//! these. Pure — no `std::fs`, `std::process`, `std::thread`, `tokio` or
//! `std::time::SystemTime` (M8a design decision 2).

use proto::{BlockReason, TaskState};

use super::edits::{Batch, not_started, state_label};
use super::model::Run;
use super::phases::set_state;
use super::plan::PlanError;

/// Decision 42's `message` and `refresh` until task M9.13a: the whole batch is refused.
const NOT_YET: &str = "message and refresh are not available yet";

/// Decision 25: replaces task `task_id`'s whole dependency list with `deps` (each once,
/// in order), on a `pending`, `queued` or `blocked` task. A `blocked(dep_cancelled)`
/// task whose new list names no cancelled task returns to `pending`. Whether every id
/// exists, is not cancelled and closes no cycle is the batch's validation, exactly as
/// for `add_dep`: the caller records every `(task_id, dep)` as added by the batch.
pub(crate) fn apply_amend_deps(
    run: &mut Run,
    task_id: &str,
    deps: &[String],
    now: u64,
) -> Result<(), PlanError> {
    let Some(i) = run.tasks.iter().position(|t| t.id() == task_id) else {
        return Err(PlanError::new(
            Some(task_id),
            "task_id",
            "13",
            "no such task",
        ));
    };
    let task = &run.tasks[i];
    if !not_started(task.state) {
        let text = format!(
            "task {task_id} is {}; deps can be amended only on pending, queued or blocked tasks",
            state_label(task)
        );
        return Err(PlanError::new(Some(task_id), "", "13", text));
    }
    let mut list: Vec<String> = Vec::new();
    for dep in deps {
        if !list.contains(dep) {
            list.push(dep.clone());
        }
    }
    let cancelled = list.iter().any(|d| {
        run.tasks
            .iter()
            .any(|t| t.id() == d && t.state == TaskState::Cancelled)
    });
    let task = &mut run.tasks[i];
    task.spec.deps = list;
    let dep_cancelled = task
        .block
        .as_ref()
        .is_some_and(|b| b.reason == BlockReason::DepCancelled);
    if task.state == TaskState::Blocked && dep_cancelled && !cancelled {
        set_state(task, TaskState::Pending, now);
        task.block = None;
    }
    Ok(())
}

impl Batch {
    /// `amend_task`'s `deps` on task `i`, after its other fields: every new dependency
    /// counts as added by this batch, so a cancelled one is refused (decision 25).
    pub(super) fn amend_deps(&mut self, i: usize, deps: &[String], changed: &mut Vec<&str>) {
        let id = self.run.tasks[i].id().to_string();
        self.added_deps
            .extend(deps.iter().map(|d| (id.clone(), d.clone())));
        if let Err(error) = apply_amend_deps(&mut self.run, &id, deps, self.now) {
            self.errors.push(error);
        }
        changed.push("deps");
    }

    /// Refuses a `message` or `refresh` edit, and with it the whole batch.
    pub(super) fn not_yet(&mut self) {
        self.errors.push(PlanError::new(None, "op", "13", NOT_YET));
    }
}
