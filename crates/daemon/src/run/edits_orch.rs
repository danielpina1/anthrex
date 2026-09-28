//! Milestone 9's plan edits: decision 25's `amend_task deps` and decision 22's
//! confinement of a sub-planner's added and split tasks to its epic (task M9.4) and,
//! from task M9.13a, decision 42's `message` and `refresh`. `run/edits.rs` delegates to
//! these. Pure — no `std::fs`, `std::process`, `std::thread`, `tokio` or
//! `std::time::SystemTime` (M8a design decision 2).

use proto::{BlockReason, PlanEdit, PlanTask, TaskState};

use super::edits::{Batch, not_started, state_label};
use super::model::Run;
use super::orch::EditSource;
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

    /// `add_task`, after [`Batch::owned`].
    pub(super) fn add_owned(&mut self, spec: &PlanTask) {
        if let Some(spec) = self.owned(spec) {
            self.add_task(spec);
        }
    }

    /// `split_task`, after [`Batch::owned`] for each new task. Decision 22: a
    /// sub-planner splits only its own epic's tasks. An unknown task is reported as
    /// such before any epic rule (M9.4 second review, ruling 4).
    pub(super) fn split_owned(&mut self, id: &str, into: &[PlanTask]) {
        let Some(i) = self.find(id) else { return };
        if let EditSource::Planner { epic } = &self.source
            && self.run.tasks[i].spec.epic.as_ref() != Some(epic)
        {
            let text = format!("a sub-planner splits only tasks of its own epic {epic}");
            self.errors
                .push(PlanError::new(Some(id), "epic", "2.epic", text));
            return;
        }
        let owned: Vec<Option<PlanTask>> = into.iter().map(|s| self.owned(s)).collect();
        if let Some(into) = owned.into_iter().collect::<Option<Vec<_>>>() {
            self.split(id, &into);
        }
    }

    /// A task this batch's source adds, as the batch holds it. Decision 22: a
    /// sub-planner adds tasks only to its own epic, which an absent `epic` is set to;
    /// one naming another epic is refused (`None`).
    fn owned(&mut self, spec: &PlanTask) -> Option<PlanTask> {
        let mut spec = spec.clone();
        if let EditSource::Planner { epic } = &self.source {
            if spec.epic.as_ref().is_some_and(|named| named != epic) {
                let text = format!("a sub-planner adds tasks only to its own epic {epic}");
                self.errors
                    .push(PlanError::new(Some(&spec.id), "epic", "2.epic", text));
                return None;
            }
            spec.epic = Some(epic.clone());
        }
        Some(spec)
    }
}

/// Decision 22: a sub-planner changes only its own epic's tasks. An `amend_task`,
/// `cancel_task` or `add_dep` of a task of another epic, or of one with no epic (the
/// orchestrator's or the user's), is refused; `add_dep`'s `dep` may name any task. This
/// closes the M8a.6 follow-up that `EditScope::Area` limits only `owns`. A task the
/// batch itself adds is the planner's own, and an unknown one is the batch's to report,
/// so only tasks of the run before the batch are checked. (`split_task` and `add_task`
/// are confined in the batch: `Batch::split_owned`, `Batch::owned`.)
pub(crate) fn planner_confinement(run: &Run, edits: &[PlanEdit], epic: &str) -> Vec<PlanError> {
    edits
        .iter()
        .filter_map(|edit| match edit {
            PlanEdit::AmendTask { task_id, .. }
            | PlanEdit::CancelTask { task_id }
            | PlanEdit::AddDep { task_id, .. } => Some(task_id),
            _ => None,
        })
        .filter(|id| {
            run.task(id)
                .is_some_and(|t| t.spec.epic.as_deref() != Some(epic))
        })
        .map(|id| {
            let text = format!("task {id}: a sub-planner changes only its own epic's tasks");
            PlanError::new(Some(id), "", "2.epic", text)
        })
        .collect()
}
