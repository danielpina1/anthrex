//! Plan edits, decision 13. Pure — no `std::fs`, `std::process`, `std::thread`, `tokio`
//! or `std::time::SystemTime` (design decision 2).
//!
//! [`apply_edits`] applies a batch, in order, to a copy of the run, then validates the
//! copy with the plan's own rules: every added, split-in or amended task goes through
//! `resolve_task_lenient`, exactly as a plan task does, and the whole list through
//! `validate_tasks`. Any error rejects the whole batch with every error listed, and the
//! caller's run is untouched. What the engine must then do (deliver a message, kill a
//! live session, pause, resume, finish) comes back as [`EditConsequence`]s; nothing is
//! executed here.

use std::collections::BTreeSet;

use proto::{AgentRole, BlockInfo, BlockReason, PlanEdit, PlanTask, Size, TaskState};

use super::contract::{amend_message, answer_message};
use super::model::{Run, Task, TaskEvent, task_branch, task_path};
use super::plan::PlanError;
use super::roster::pick_reviewer;
use super::validate::{
    EditScope, combined_cycles, implicit_deps, protected_notes, resolve_task_lenient,
    validate_tasks_with,
};

/// What the engine must do after a batch is applied; the model change itself is already
/// in the returned run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EditConsequence {
    /// Kill the task's live session, then salvage and remove its worktree (decision 20).
    CancelLive {
        task_id: String,
    },
    /// Queue `text` for the task's worker as its next turn (decision 29).
    Deliver {
        task_id: String,
        text: String,
    },
    Pause,
    Resume,
    Finish,
}

/// Applies `edits` in order to a copy of `run` and validates the result (decision 13).
/// `scope` limits where added and amended tasks may own files (decision 12); `now`
/// stamps the task history entries the edits write.
pub fn apply_edits(
    run: &Run,
    edits: &[PlanEdit],
    scope: &EditScope,
    now: u64,
) -> Result<(Run, Vec<EditConsequence>), Vec<PlanError>> {
    let mut batch = Batch {
        run: run.clone(),
        touched: BTreeSet::new(),
        added_deps: BTreeSet::new(),
        errors: Vec::new(),
        consequences: Vec::new(),
        now,
    };
    for edit in edits {
        batch.apply(edit);
    }
    let Batch {
        run: mut edited,
        touched,
        added_deps,
        mut errors,
        consequences,
        ..
    } = batch;
    errors.extend(validate_tasks_with(
        &edited.tasks,
        &touched,
        Some(&added_deps),
        scope,
        edited.limits.max_tasks,
        &edited.profile,
    ));
    // Decision 41's implicit dependencies follow the edited graph; the combined check
    // is the same backstop `build_run` runs (M8a.6 fix round 1, F2).
    let implicit = implicit_deps(&edited.tasks);
    for (task, deps) in edited.tasks.iter_mut().zip(implicit) {
        task.implicit_deps = deps;
    }
    if errors.is_empty() {
        errors.extend(combined_cycles(&edited.tasks));
    }
    if errors.is_empty() {
        Ok((edited, consequences))
    } else {
        Err(errors)
    }
}

/// The states in which a task has not started: route, size, test mode and dependencies
/// may change, and it may be split.
fn not_started(state: TaskState) -> bool {
    matches!(
        state,
        TaskState::Pending | TaskState::Queued | TaskState::Blocked
    )
}

/// A task with a session or an engine operation to stop when it is cancelled.
fn is_live(task: &Task) -> bool {
    matches!(
        task.state,
        TaskState::Preparing
            | TaskState::Working
            | TaskState::Proof
            | TaskState::Check
            | TaskState::Review
            | TaskState::MergeQueue
    ) || task.rounds.iter().any(|r| !r.ended)
}

/// A task whose worker can take an amendment as a message.
fn has_live_worker(task: &Task) -> bool {
    task.state == TaskState::Working
        || task
            .rounds
            .iter()
            .any(|r| r.role == AgentRole::Worker && !r.ended)
}

fn block_label(reason: BlockReason) -> &'static str {
    match reason {
        BlockReason::MisSized => "mis_sized",
        BlockReason::Human => "human",
        BlockReason::Conflict => "conflict",
        BlockReason::DepCancelled => "dep_cancelled",
        BlockReason::Question => "question",
        BlockReason::Environment => "environment",
    }
}

/// `working`, `merged`, or `blocked(<reason>)`.
fn state_label(task: &Task) -> String {
    match (task.state, &task.block) {
        (TaskState::Blocked, Some(block)) => format!("blocked({})", block_label(block.reason)),
        (state, _) => state.label().to_string(),
    }
}

struct Batch {
    run: Run,
    touched: BTreeSet<String>,
    /// `(task, dep)` pairs this batch adds: the cancelled-dependency rule applies to
    /// these only (F3).
    added_deps: BTreeSet<(String, String)>,
    errors: Vec<PlanError>,
    consequences: Vec<EditConsequence>,
    now: u64,
}

impl Batch {
    fn apply(&mut self, edit: &PlanEdit) {
        match edit {
            PlanEdit::AddTask { task } => self.add_task(task.clone()),
            PlanEdit::SplitTask { task_id, into } => self.split(task_id, into),
            PlanEdit::CancelTask { task_id } => self.cancel(task_id),
            PlanEdit::AmendTask { .. } => self.amend_task(edit),
            PlanEdit::AddDep { task_id, dep } => self.add_dep(task_id, dep),
            PlanEdit::Answer { task_id, text } => self.answer(task_id, text),
            PlanEdit::Pause => self.consequences.push(EditConsequence::Pause),
            PlanEdit::Resume => self.consequences.push(EditConsequence::Resume),
            PlanEdit::Finish => self.consequences.push(EditConsequence::Finish),
        }
    }

    /// The first task with `id`, or an error naming it.
    fn find(&mut self, id: &str) -> Option<usize> {
        let found = self.run.tasks.iter().position(|t| t.id() == id);
        if found.is_none() {
            self.errors
                .push(PlanError::new(Some(id), "task_id", "13", "no such task"));
        }
        found
    }

    /// Decision 13's per-state refusal: `task <id> is <state>; <why>`.
    fn refuse(&mut self, i: usize, why: &str) {
        let task = &self.run.tasks[i];
        self.errors.push(PlanError::new(
            Some(task.id()),
            "",
            "13",
            format!("task {} is {}; {why}", task.id(), state_label(task)),
        ));
    }

    fn log(&mut self, i: usize, text: String) {
        self.run.tasks[i]
            .history
            .push(TaskEvent { at: self.now, text });
    }

    /// Resolves a spec exactly as `build_run` resolves a plan task, keeping its errors.
    fn resolve(&mut self, spec: PlanTask) -> Task {
        let run = &self.run;
        let (mut task, errors) = resolve_task_lenient(
            spec,
            &run.profile,
            &run.limits,
            &run.roster,
            run.limits.default_runtime,
        );
        self.errors.extend(errors);
        task.branch = task_branch(&run.id, task.id());
        task.worktree = task_path(&run.wt_dir, &run.id, task.id());
        task.notes
            .extend(protected_notes(&task.spec.owns, &run.protected_files));
        self.touched.insert(task.spec.id.clone());
        task
    }

    /// Records every declared dependency of a new task as added by this batch.
    fn add_deps_of(&mut self, task: &Task) {
        for dep in &task.spec.deps {
            self.added_deps.insert((task.spec.id.clone(), dep.clone()));
        }
    }

    fn add_task(&mut self, spec: PlanTask) {
        let task = self.resolve(spec);
        self.add_deps_of(&task);
        self.run.tasks.push(task);
        let last = self.run.tasks.len() - 1;
        self.log(last, "added by a plan edit".to_string());
    }

    fn cancel(&mut self, id: &str) {
        let Some(i) = self.find(id) else { return };
        if self.run.tasks[i].state.is_finished() {
            return self.refuse(i, "only unfinished tasks can be cancelled");
        }
        self.cancel_at(i);
    }

    /// Cancels an unfinished task: a live one yields `CancelLive`, it leaves the merge
    /// queue, and every unfinished task that declared it as a dependency becomes
    /// `blocked(dep_cancelled)`.
    fn cancel_at(&mut self, i: usize) {
        let id = self.run.tasks[i].id().to_string();
        if is_live(&self.run.tasks[i]) {
            self.consequences.push(EditConsequence::CancelLive {
                task_id: id.clone(),
            });
        }
        let task = &mut self.run.tasks[i];
        task.state = TaskState::Cancelled;
        task.block = None;
        self.log(i, "cancelled by a plan edit".to_string());
        self.run.merge_queue.retain(|q| *q != id);
        for j in 0..self.run.tasks.len() {
            let dependent = &mut self.run.tasks[j];
            if j == i || dependent.state.is_finished() || !dependent.spec.deps.contains(&id) {
                continue;
            }
            let text = format!("dependency {id} was cancelled");
            let history = match (&dependent.block, dependent.state) {
                (Some(old), TaskState::Blocked) => format!(
                    "blocked: {text} (was {}: {})",
                    state_label(dependent),
                    old.text
                ),
                _ => format!("blocked: {text}"),
            };
            dependent.state = TaskState::Blocked;
            dependent.block = Some(BlockInfo {
                reason: BlockReason::DepCancelled,
                text,
            });
            self.log(j, history);
        }
    }

    /// Cancels `id`, inserts `into` right after it in plan order, and makes every task
    /// that depended on `id` depend on all of `into` instead.
    fn split(&mut self, id: &str, into: &[PlanTask]) {
        let Some(i) = self.find(id) else { return };
        if !not_started(self.run.tasks[i].state) {
            return self.refuse(i, "only pending, queued or blocked tasks can be split");
        }
        if into.is_empty() {
            self.errors.push(PlanError::new(
                Some(id),
                "into",
                "13",
                "at least one task is required",
            ));
            return;
        }
        let children: Vec<Task> = into.iter().map(|s| self.resolve(s.clone())).collect();
        for child in &children {
            self.add_deps_of(child);
        }
        let child_ids: Vec<String> = children.iter().map(|c| c.spec.id.clone()).collect();
        for task in self.run.tasks.iter_mut() {
            if task.state.is_finished() || !task.spec.deps.iter().any(|d| d == id) {
                continue;
            }
            let mut deps = Vec::new();
            for dep in &task.spec.deps {
                let replaced = if dep == id {
                    child_ids.clone()
                } else {
                    vec![dep.clone()]
                };
                for d in replaced {
                    if !deps.contains(&d) {
                        deps.push(d);
                    }
                }
            }
            task.spec.deps = deps;
        }
        self.cancel_at(i);
        self.log(i, format!("split into {}", child_ids.join(", ")));
        let count = children.len();
        self.run.tasks.splice(i + 1..i + 1, children);
        for j in i + 1..=i + count {
            self.log(j, format!("split from {id}"));
        }
    }

    /// `amend_task`: brief, acceptance and priority on any unfinished task; route, test
    /// mode, its reason and size only on a task that has not started (one refusal per
    /// such field). An amend naming no field is refused. When route, test mode, its
    /// reason or size changed, the task's derived fields are re-resolved from the spec
    /// (decisions 8–10), but never below the engine's own changes (fix round 1, F1): a
    /// size the engine raised (rung 3) is a floor, even for an explicit smaller `size`;
    /// an escalated route (rung 2) stays unless the amend names `route`; the engine's
    /// notes stay. Otherwise only the spec changes. Either way the spec is validated as a
    /// plan task's is. A new brief or new criteria reach a live worker as
    /// `amend_message`.
    fn amend_task(&mut self, edit: &PlanEdit) {
        let PlanEdit::AmendTask {
            task_id,
            brief,
            acceptance,
            route,
            test_mode,
            test_mode_reason,
            priority,
            size,
        } = edit
        else {
            return;
        };
        let Some(i) = self.find(task_id) else { return };
        let nothing = brief.is_none()
            && acceptance.is_none()
            && route.is_none()
            && test_mode.is_none()
            && test_mode_reason.is_none()
            && priority.is_none()
            && size.is_none();
        if nothing {
            self.errors.push(PlanError::new(
                Some(task_id),
                "amend_task",
                "13",
                "nothing to amend",
            ));
            return;
        }
        let state = self.run.tasks[i].state;
        if state.is_finished() {
            return self.refuse(i, "only unfinished tasks can be amended");
        }
        let restricted = [
            ("route", route.is_some()),
            ("test_mode", test_mode.is_some()),
            ("test_mode_reason", test_mode_reason.is_some()),
            ("size", size.is_some()),
        ];
        let reresolve = restricted.iter().any(|(_, set)| *set);
        if reresolve && !not_started(state) {
            for (name, _) in restricted.iter().filter(|(_, set)| *set) {
                self.refuse(
                    i,
                    &format!("{name} can be amended only on pending, queued or blocked tasks"),
                );
            }
            return;
        }

        let mut spec = self.run.tasks[i].spec.clone();
        let mut changed = Vec::new();
        if let Some(v) = brief {
            spec.brief = v.clone();
            changed.push("brief");
        }
        if let Some(v) = acceptance {
            spec.acceptance = v.clone();
            changed.push("acceptance");
        }
        if let Some(v) = route {
            spec.route = v.clone();
            changed.push("route");
        }
        if let Some(v) = test_mode {
            spec.test_mode = Some(*v);
            changed.push("test_mode");
        }
        if let Some(v) = test_mode_reason {
            spec.test_mode_reason = Some(v.clone());
            changed.push("test_mode_reason");
        }
        if let Some(v) = priority {
            spec.priority = *v;
            changed.push("priority");
        }
        if let Some(v) = size {
            spec.size = *v;
            changed.push("size");
        }

        if !reresolve {
            let resolved = self.resolve(spec);
            self.run.tasks[i].spec = resolved.spec;
        } else {
            self.reresolve(i, spec, route.is_some());
        }
        let task = &self.run.tasks[i];
        if (brief.is_some() || acceptance.is_some()) && has_live_worker(task) {
            self.consequences.push(EditConsequence::Deliver {
                task_id: task.id().to_string(),
                text: amend_message(task),
            });
        }
        self.log(i, format!("amended: {}", changed.join(", ")));
    }

    /// Re-resolves task `i` from its amended `spec` without undoing the engine: what the
    /// unamended spec resolves to is the plan's part, and anything the task holds beyond
    /// it (a larger size, a different route, extra notes) is the engine's.
    fn reresolve(&mut self, i: usize, spec: PlanTask, route_named: bool) {
        let run = &self.run;
        let old = &run.tasks[i];
        let (planned, _) = resolve_task_lenient(
            old.spec.clone(),
            &run.profile,
            &run.limits,
            &run.roster,
            run.limits.default_runtime,
        );
        let floor = if old.size > planned.size {
            old.size
        } else {
            Size::S
        };
        let escalated = (old.route != planned.route).then(|| old.route.clone());
        let engine_notes: Vec<String> = old
            .notes
            .iter()
            .filter(|n| !planned.notes.contains(n))
            .cloned()
            .collect();

        let mut sized = spec.clone();
        sized.size = sized.size.max(floor);
        let resolved = self.resolve(sized);
        let route = match escalated {
            Some(route) if !route_named => route,
            _ => resolved.route,
        };
        let task = &mut self.run.tasks[i];
        task.review_route = resolved
            .review_level
            .map(|level| pick_reviewer(&self.run.roster, &route, level));
        task.spec = spec;
        task.size = resolved.size;
        task.hub = resolved.hub;
        task.test_mode = resolved.test_mode;
        task.review_level = resolved.review_level;
        task.route = route;
        task.budget = resolved.budget;
        task.notes = resolved.notes;
        for note in engine_notes {
            if !task.notes.contains(&note) {
                task.notes.push(note);
            }
        }
    }

    fn add_dep(&mut self, id: &str, dep: &str) {
        let Some(i) = self.find(id) else { return };
        if !not_started(self.run.tasks[i].state) {
            return self.refuse(
                i,
                "dependencies can be added only on pending, queued or blocked tasks",
            );
        }
        let dep_merged = self
            .run
            .task(dep)
            .is_some_and(|d| d.state == TaskState::Merged);
        let task = &mut self.run.tasks[i];
        if !task.spec.deps.iter().any(|d| d == dep) {
            task.spec.deps.push(dep.to_string());
        }
        // A queued task is runnable; one that now waits for unmerged work is not.
        if task.state == TaskState::Queued && !dep_merged {
            task.state = TaskState::Pending;
        }
        self.touched.insert(id.to_string());
        self.added_deps.insert((id.to_string(), dep.to_string()));
        self.log(i, format!("dependency on {dep} added"));
    }

    /// Answers a `blocked(question)` or `working` task; a blocked one returns to
    /// `working` in the same session.
    fn answer(&mut self, id: &str, text: &str) {
        let Some(i) = self.find(id) else { return };
        let task = &mut self.run.tasks[i];
        let question = task.state == TaskState::Blocked
            && task
                .block
                .as_ref()
                .is_some_and(|b| b.reason == BlockReason::Question);
        if !question && task.state != TaskState::Working {
            return self.refuse(i, "only blocked(question) or working tasks can be answered");
        }
        task.state = TaskState::Working;
        task.block = None;
        self.consequences.push(EditConsequence::Deliver {
            task_id: id.to_string(),
            text: answer_message(text),
        });
        self.log(i, "answered".to_string());
    }
}

#[cfg(test)]
#[path = "edits_tests.rs"]
mod tests;
