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

use super::phases::set_state;
use std::collections::BTreeSet;

use proto::{BlockInfo, BlockReason, PlanEdit, PlanTask, TaskState};

use super::contract::answer_message;
use super::delivery::{reply_edit, validate};
use super::edits_state::is_live;
pub(super) use super::edits_state::{not_started, state_label};
use super::engine::actions::rules;
use super::model::{Run, Task, TaskEvent, task_branch, task_path};
use super::orch::EditSource;
use super::plan::PlanError;
use super::validate::{
    EditScope, combined_cycles, implicit_deps, protected_notes, reserved_new_id,
    resolve_task_lenient, split_child_stage, validate_tasks_with,
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
    /// Milestone 9 decision 42b: queue a `message`'s `text`, recorded last in the
    /// task's messages, for its worker.
    Message {
        task_id: String,
        text: String,
    },
    /// Decision 42b: a `message`'s resolved recipients, for the reply and the edit log.
    Recipients(super::edits_orch::MessageOutcome),
    Pause,
    Resume,
    Finish,
}

/// Applies `edits` in order to a copy of `run` and validates the result (decision 13).
/// `scope` limits where tasks may own files (decision 12); a batch from the orchestrator
/// or a sub-planner (`source`) also meets `orch::rules`; `now` stamps task history.
pub fn apply_edits(
    run: &Run,
    edits: &[PlanEdit],
    scope: &EditScope,
    source: &EditSource,
    now: u64,
) -> Result<(Run, Vec<EditConsequence>), Vec<PlanError>> {
    let mut batch = Batch {
        source: source.clone(),
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
        edited.limits.default_runtime,
    ));
    errors.extend(super::validate_stages::single_layout_rule(&edited));
    errors.extend(super::orch::rules::apply(&mut edited, run, source));
    // Decision 41's implicit dependencies follow the edited graph; the combined check
    // is the same backstop `build_run` runs (M8a.6 fix round 1, F2).
    let implicit = implicit_deps(&edited.tasks);
    for (task, deps) in edited.tasks.iter_mut().zip(implicit) {
        task.implicit_deps = deps;
    }
    requeue_waiting(&mut edited, now);
    if errors.is_empty() {
        errors.extend(combined_cycles(&edited.tasks));
    }
    if errors.is_empty() {
        Ok((edited, consequences))
    } else {
        Err(errors)
    }
}

/// Decision 31: `queued` means runnable. After a batch, a queued task that waits for a
/// declared dependency not merged or reported, or an implicit one not finished
/// (decision 41), goes back to `pending` (`add_dep`, and implicit
/// dependencies gained through a split or an added task, fix round 2, N2).
fn requeue_waiting(run: &mut Run, now: u64) {
    let state_of = |tasks: &[Task], id: &str| tasks.iter().find(|t| t.id() == id).map(|t| t.state);
    let waiting: Vec<usize> = run
        .tasks
        .iter()
        .enumerate()
        .filter(|(_, t)| t.state == TaskState::Queued)
        .filter(|(_, t)| {
            t.spec.deps.iter().any(|d| {
                !matches!(
                    state_of(&run.tasks, d),
                    Some(TaskState::Merged | TaskState::Reported)
                )
            }) || t
                .implicit_deps
                .iter()
                .any(|d| state_of(&run.tasks, d).is_some_and(|s| !s.is_finished()))
        })
        .map(|(i, _)| i)
        .collect();
    for i in waiting {
        set_state(&mut run.tasks[i], TaskState::Pending, now);
    }
}

pub(super) struct Batch {
    pub(super) source: EditSource,
    pub(super) run: Run,
    touched: BTreeSet<String>,
    /// `(task, dep)` pairs this batch adds: the cancelled-dependency rule applies to
    /// these only (F3).
    pub(super) added_deps: BTreeSet<(String, String)>,
    pub(super) errors: Vec<PlanError>,
    pub(super) consequences: Vec<EditConsequence>,
    pub(super) now: u64,
}

impl Batch {
    fn apply(&mut self, edit: &PlanEdit) {
        match edit {
            PlanEdit::AddTask { task } => self.add_owned(task),
            PlanEdit::SplitTask { task_id, into } => self.split_owned(task_id, into),
            PlanEdit::CancelTask { task_id } => self.cancel(task_id),
            // Milestone 9 decisions 42a and 42e.
            PlanEdit::Message { to, text, kind } => self.message(to, text, *kind),
            PlanEdit::Refresh { task_id } => self.refresh(task_id),
            PlanEdit::AmendTask { .. } => self.amend_task(edit),
            PlanEdit::AddDep { task_id, dep } => self.add_dep(task_id, dep),
            PlanEdit::Answer { task_id, text } => self.answer(task_id, text),
            PlanEdit::Pause => self.consequences.push(EditConsequence::Pause),
            PlanEdit::Resume => self.consequences.push(EditConsequence::Resume),
            PlanEdit::Finish => self.consequences.push(EditConsequence::Finish),
            // Milestone 9.2 decision 30.
            PlanEdit::ReplyComment { pr, thread, body } => {
                let edit = (*pr, &thread[..], &body[..]);
                let applied = reply_edit::apply(&mut self.run, edit, &self.source, self.now);
                self.errors.extend(applied.err());
            }
            // Milestone 9.3 decision 30: a round starts only from `run iterate` or
            // `edit_plan`'s `iterate` (task M9.3.4a); never as a batch edit.
            PlanEdit::Iterate { .. } => self.errors.push(PlanError::new(
                None,
                "",
                "43",
                "use anthrex run iterate to start a round",
            )),
        }
    }

    /// The first task with `id`, or an error naming it.
    pub(super) fn find(&mut self, id: &str) -> Option<usize> {
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
        // Milestone 9.2 decision 31: `addresses` makes it a review fix; an amend's are
        // re-checked only where they changed (the final fix wave's I-5).
        let before = (self.run.task(&task.spec.id)).map(|t| t.spec.addresses.clone());
        let review = validate::apply(&mut self.run, &mut task, &self.source, before.as_deref());
        self.errors.extend(review);
        self.touched.insert(task.spec.id.clone());
        task
    }

    /// Records every declared dependency of a new task as added by this batch.
    fn add_deps_of(&mut self, task: &Task) {
        for dep in &task.spec.deps {
            self.added_deps.insert((task.spec.id.clone(), dep.clone()));
        }
    }

    pub(super) fn add_task(&mut self, spec: PlanTask) {
        // Milestone 9.1 decision 39: a new task may not take a `fix<n>` id.
        self.errors.extend(reserved_new_id(&spec.id));
        let task = self.resolve(spec);
        self.add_deps_of(&task);
        self.run.tasks.push(task);
        let last = self.run.tasks.len() - 1;
        self.log(last, "added by a plan edit".to_string());
    }

    fn cancel(&mut self, id: &str) {
        let Some(i) = self.find(id) else { return };
        if let Some(text) = rules::cancel_task(&self.run, id) {
            return self.errors.push(PlanError::new(Some(id), "", "13", text));
        }
        self.cancel_at(i);
    }

    /// Cancels an unfinished task: a live one yields `CancelLive`, it leaves the merge
    /// queue, and every unfinished task that declared it as a dependency becomes
    /// `blocked(dep_cancelled)`.
    fn cancel_at(&mut self, i: usize) {
        let id = self.run.tasks[i].id().to_string();
        // Ruling T14-I1: a task whose merge candidate is in flight is cancelled only if
        // that merge does not land (`engine::merge::candidate_done`).
        let task = &self.run.tasks[i];
        let merging = task.merge_op.is_some_and(|op| {
            self.run
                .pending_ops
                .get(&op)
                .is_some_and(|p| matches!(p.kind, super::engine::OpKind::MergeCandidate { .. }))
        });
        if merging {
            self.run.tasks[i].cancel_deferred = true;
            self.log(i, "cancel deferred: its merge is in flight".to_string());
            return;
        }
        if is_live(&self.run.tasks[i]) {
            self.consequences.push(EditConsequence::CancelLive {
                task_id: id.clone(),
            });
        }
        let task = &mut self.run.tasks[i];
        set_state(task, TaskState::Cancelled, self.now);
        task.block = None;
        // Ruling T14-R3: nothing is handed back to a cancelled task, as `run cancel` has it.
        task.handback_due = false;
        self.log(i, "cancelled by a plan edit".to_string());
        self.run.merge_queue.retain(|q| *q != id);
        // Review m3: its queued deciders are dropped.
        super::engine::deciders::drop_queued(&mut self.run.decider_queue, &id);
        self.run.tasks[i].drop_pending_size_check();
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
            set_state(dependent, TaskState::Blocked, self.now);
            dependent.block = Some(BlockInfo {
                reason: BlockReason::DepCancelled,
                text,
            });
            self.log(j, history);
        }
    }

    /// Cancels `id`, inserts `into` right after it in plan order, and makes every task
    /// that depended on `id` depend on all of `into` instead.
    pub(super) fn split(&mut self, id: &str, into: &[PlanTask]) {
        let Some(i) = self.find(id) else { return };
        if !not_started(&self.run.tasks[i]) {
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
        let parent = self.run.tasks[i].spec.stage;
        let mut specs = into.to_vec();
        for s in &mut specs {
            self.errors.extend(reserved_new_id(&s.id));
            self.errors.extend(split_child_stage(parent, s));
        }
        let children: Vec<Task> = specs.into_iter().map(|s| self.resolve(s)).collect();
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

    fn add_dep(&mut self, id: &str, dep: &str) {
        let Some(i) = self.find(id) else { return };
        if self.epic_being_planned(i) {
            return;
        }
        if !not_started(&self.run.tasks[i]) {
            return self.refuse(
                i,
                "dependencies can be added only on pending, queued or blocked tasks",
            );
        }
        let task = &mut self.run.tasks[i];
        if !task.spec.deps.iter().any(|d| d == dep) {
            task.spec.deps.push(dep.to_string());
        }
        self.touched.insert(id.to_string());
        self.added_deps.insert((id.to_string(), dep.to_string()));
        self.log(i, format!("dependency on {dep} added"));
    }

    /// Answers a `blocked(question)` or `working` task; a blocked one returns to
    /// `working` in the same session.
    fn answer(&mut self, id: &str, text: &str) {
        let Some(i) = self.find(id) else { return };
        if let Some(text) = rules::answer(&self.run, id) {
            return self.errors.push(PlanError::new(Some(id), "", "13", text));
        }
        let task = &mut self.run.tasks[i];
        set_state(task, TaskState::Working, self.now);
        task.block = None;
        // M8b decision 21: an answer before the classification wins.
        task.pending_classification = None;
        self.consequences.push(EditConsequence::Deliver {
            task_id: id.to_string(),
            text: answer_message(text),
        });
        self.log(i, "answered".to_string());
    }
}

#[path = "edits_amend.rs"]
mod amend;

#[cfg(test)]
#[path = "edits_tests.rs"]
mod tests;
