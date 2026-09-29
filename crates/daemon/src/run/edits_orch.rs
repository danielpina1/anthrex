//! Milestone 9's plan edits: decision 25's `amend_task deps` and decision 22's
//! confinement of a sub-planner's added and split tasks to its epic (task M9.4), and
//! decision 42's `message` and `refresh` (task M9.13a). `run/edits.rs` delegates to
//! these. Pure — no `std::fs`, `std::process`, `std::thread`, `tokio` or
//! `std::time::SystemTime` (M8a design decision 2).

use proto::TaskState;
use proto::{BlockInfo, BlockReason, HoldState, MessageKind, MessageTarget, PlanEdit, PlanTask};

use super::edits::{Batch, EditConsequence, not_started, state_label};
use super::edits_state::{has_live_worker, is_paused, is_reader};
use super::model::{Run, Task, TaskEvent};
use super::orch::contract::{ONE_EDIT_RULE, message_text};
use super::orch::{EditSource, RefreshState, TaskMessage};
use super::phases::set_state;
use super::plan::PlanError;

/// Decision 42a: a message's text is 1 to this many characters.
pub const MESSAGE_TEXT_MAX: usize = 4000;

/// Decision 42a: a `message` names 1 to this many tasks.
pub const MESSAGE_TASKS_MAX: usize = 20;

/// Decision 42b: a `message`'s resolved recipients. `delivered` lists every task that
/// took it (a pending one's next prompt carries it), `refused` every other with its
/// reason, and `queued` the delivered tasks whose worker gets it through the outbox;
/// `text` is the message as saved, one line.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MessageOutcome {
    pub delivered: Vec<String>,
    pub refused: Vec<(String, String)>,
    pub queued: Vec<String>,
    pub text: String,
}

/// Decision 42: a `message` or `refresh` is the only edit of its call, with no
/// `submit` and no `summary`. Checked before anything else, so the refusal has no
/// effect at all.
pub(crate) fn one_edit_rule(
    edits: &[PlanEdit],
    submit: bool,
    summary: bool,
) -> Result<(), PlanError> {
    let special = |e: &PlanEdit| matches!(e, PlanEdit::Message { .. } | PlanEdit::Refresh { .. });
    if edits.iter().any(special) && (edits.len() > 1 || submit || summary) {
        return Err(PlanError::new(None, "", "42", ONE_EDIT_RULE));
    }
    Ok(())
}

/// How one recipient takes a message (decision 42b).
enum Takes {
    /// Queued in the outbox: a live worker, or one whose mail waits for an answer or
    /// a bouncing gate.
    Queue,
    /// Recorded only: the first session's prompt carries it.
    Record,
    /// Queued, and the `paused(message)` task works again (decision 42c).
    Release,
}

/// Decision 42c: the next accepted `message` to a paused task, `info` or `change`.
fn takes(task: &Task, kind: MessageKind, limit: u32) -> Result<Takes, String> {
    let id = task.id();
    if limit == 0 {
        return Err(
            "messages to workers are turned off (orchestrator.message_max_per_turn = 0)".into(),
        );
    }
    if is_reader(task) {
        return Err(reader_refusal(task, "a message would not reach a worker"));
    }
    let question = task
        .block
        .as_ref()
        .is_some_and(|b| b.reason == BlockReason::Question);
    let how = match task.state {
        TaskState::Working => Takes::Queue,
        _ if is_paused(task) && kind == MessageKind::StopAndWait => {
            return Err(format!("task {id} is already paused(message)"));
        }
        _ if is_paused(task) => Takes::Release,
        TaskState::Blocked if question => Takes::Queue,
        TaskState::Proof | TaskState::Check => Takes::Queue,
        TaskState::Pending | TaskState::Queued | TaskState::Preparing => Takes::Record,
        _ => {
            let label = state_label(task);
            return Err(format!(
                "task {id} is {label}; a message would not reach a worker"
            ));
        }
    };
    if kind == MessageKind::StopAndWait && task.state != TaskState::Working {
        let label = state_label(task);
        return Err(format!(
            "task {id} is {label}; stop_and_wait needs a working task"
        ));
    }
    let waiting = task.orch.messages.iter().filter(|m| !m.delivered).count();
    if waiting >= limit as usize {
        return Err(format!(
            "task {id} already has {waiting} messages waiting for its next turn"
        ));
    }
    Ok(how)
}

/// M9.13a review, item 1: a research or review task has no worker. Its scout or
/// reviewer has no message or refresh rule, its mailbox is not a worker's, and a
/// `stop_and_wait` would leave it paused with its report refused; so it takes neither,
/// in any state, and `running` never names it.
fn reader_refusal(task: &Task, why: &str) -> String {
    let kind = crate::run::orch::json::label(&task.spec.kind);
    format!("task {} is a {kind} task; {why}", task.id())
}

/// Decision 42b's recipients, resolved at acceptance: named tasks once each, or, for
/// `running`, every task with a live worker round and no approval hold still waiting,
/// found by state and never by id (a run from before M9 may have a task named
/// `running`, M9.2 re-review).
fn recipients(run: &Run, to: &MessageTarget) -> Result<Vec<String>, PlanError> {
    let error = |text: &str| PlanError::new(None, "", "42", text);
    match to {
        MessageTarget::Stage(_) => Err(error("stage recipients arrive with milestone 9.1")),
        MessageTarget::Tasks(ids) if ids.is_empty() || ids.len() > MESSAGE_TASKS_MAX => Err(error(
            &format!("message: to must name 1 to {MESSAGE_TASKS_MAX} tasks"),
        )),
        MessageTarget::Tasks(ids) => {
            let mut list: Vec<String> = Vec::new();
            for id in ids {
                if !list.contains(id) {
                    list.push(id.clone());
                }
            }
            Ok(list)
        }
        MessageTarget::Running => Ok(run
            .tasks
            .iter()
            .filter(|t| has_live_worker(t) && !is_reader(t) && !held(run, t))
            .map(|t| t.id().to_string())
            .collect()),
    }
}

/// A task in an approval hold that is not `Approved` (decision 28): it has no session.
fn held(run: &Run, task: &Task) -> bool {
    task.orch.gate_hold.as_ref().is_some_and(|id| {
        run.orch
            .gate_holds
            .iter()
            .any(|h| &h.id == id && h.state != HoldState::Approved)
    })
}

/// Decision 42b: records a `message` to each recipient that can take it and refuses the
/// others, one refusal rolling nothing back; a message no recipient takes is refused
/// whole. The text is one line (`messages::one_line`: every control character and
/// line separator a space; ruling 2 of the M9.5 review, M9.13a review item 5), so it
/// cannot forge a second `[anthrex]` line or carry a terminal sequence. A `stop_and_wait` pauses its
/// working task (decision 42c) and a later message releases a paused one. Only the
/// orchestrator and the user message workers (decision 22).
pub(crate) fn apply_message(
    run: &mut Run,
    to: &MessageTarget,
    text: &str,
    kind: MessageKind,
    source: &EditSource,
    now: u64,
) -> Result<MessageOutcome, PlanError> {
    let error = |text: String| PlanError::new(None, "", "42", text);
    if matches!(source, EditSource::Planner { .. }) {
        return Err(error("a sub-planner cannot message workers".into()));
    }
    let text = crate::run::messages::one_line(text);
    if !(1..=MESSAGE_TEXT_MAX).contains(&text.chars().count()) {
        return Err(error(format!(
            "message: text must be 1 to {MESSAGE_TEXT_MAX} characters"
        )));
    }
    let limit = run.limits.orch.message_max_per_turn;
    let mut outcome = MessageOutcome {
        text: text.clone(),
        ..MessageOutcome::default()
    };
    for id in recipients(run, to)? {
        let Some(i) = run.tasks.iter().position(|t| t.id() == id) else {
            outcome
                .refused
                .push((id.clone(), format!("no such task {id}")));
            continue;
        };
        match takes(&run.tasks[i], kind, limit) {
            Ok(how) => record(run, i, (&text, kind, source), how, now, &mut outcome),
            Err(reason) => outcome.refused.push((id, reason)),
        }
    }
    if outcome.delivered.is_empty() {
        let reasons: Vec<&str> = outcome.refused.iter().map(|(_, r)| r.as_str()).collect();
        let reasons = if reasons.is_empty() {
            "no task has a live worker".to_string()
        } else {
            reasons.join("; ")
        };
        return Err(error(format!(
            "message: no recipient can take it: {reasons}"
        )));
    }
    Ok(outcome)
}

fn record(
    run: &mut Run,
    i: usize,
    (text, kind, source): (&str, MessageKind, &EditSource),
    how: Takes,
    now: u64,
    outcome: &mut MessageOutcome,
) {
    let task = &mut run.tasks[i];
    let id = task.id().to_string();
    task.orch.messages.push(TaskMessage {
        at: now,
        source: source.clone(),
        kind,
        text: text.to_string(),
        delivered: false,
        outbox: None,
    });
    let label = crate::run::orch::json::label(&kind);
    let event = format!("message ({label}) from {}", source.label());
    task.history.push(TaskEvent {
        at: now,
        text: event,
    });
    if matches!(how, Takes::Queue | Takes::Release) {
        outcome.queued.push(id.clone());
    }
    if matches!(how, Takes::Release) {
        release(task, now);
    }
    if kind == MessageKind::StopAndWait {
        let first: String = text.chars().take(120).collect();
        set_state(task, TaskState::Blocked, now);
        task.block = Some(BlockInfo {
            reason: BlockReason::MessagePause,
            text: format!("asked to stop and wait: {first}"),
        });
    }
    outcome.delivered.push(id);
}

/// Decision 42c: a `paused(message)` task works again in the same session.
pub(crate) fn release(task: &mut Task, now: u64) {
    if is_paused(task) {
        set_state(task, TaskState::Working, now);
        task.block = None;
        task.history.push(TaskEvent {
            at: now,
            text: "resumed from paused(message)".into(),
        });
    }
}

/// Decision 42e's pure half: a refresh of a `working` or `paused(message)` task that
/// waits on no dependency, resolves no conflict, and has no gate, no hand-back and no
/// other refresh in flight is due at its worker's next turn boundary. Only the
/// orchestrator and the user refresh (decision 22).
pub(crate) fn apply_refresh(
    run: &mut Run,
    task_id: &str,
    source: &EditSource,
    now: u64,
) -> Result<(), PlanError> {
    if matches!(source, EditSource::Planner { .. }) {
        let text = "a sub-planner cannot refresh a task";
        return Err(PlanError::new(Some(task_id), "", "42", text));
    }
    let Some(i) = run.tasks.iter().position(|t| t.id() == task_id) else {
        return Err(PlanError::new(
            Some(task_id),
            "task_id",
            "13",
            "no such task",
        ));
    };
    let hand_back = run.pending_ops.values().any(|p| {
        p.task_id.as_deref() == Some(task_id)
            && matches!(p.kind, super::engine::OpKind::HandBack { .. })
    });
    let task = &run.tasks[i];
    if is_reader(task) {
        let text = reader_refusal(task, "refresh needs a code or docs task");
        return Err(PlanError::new(Some(task_id), "", "42", text));
    }
    let fits = (task.state == TaskState::Working || is_paused(task))
        && !task.awaiting_deps
        && !task.resolving
        && task.gate_op.is_none()
        && task.merge_op.is_none()
        && task.orch.refresh.is_none()
        && !hand_back;
    if !fits {
        let text = format!(
            "task {task_id} is {}; refresh needs a working or paused task",
            state_label(task)
        );
        return Err(PlanError::new(Some(task_id), "", "42", text));
    }
    let task = &mut run.tasks[i];
    task.orch.refresh = Some(RefreshState::Due);
    task.history.push(TaskEvent {
        at: now,
        text: "refresh requested".into(),
    });
    Ok(())
}

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
    if !not_started(task) {
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

    /// Decision 42a's `message` (see [`apply_message`]).
    pub(super) fn message(&mut self, to: &MessageTarget, text: &str, kind: MessageKind) {
        let source = self.source.clone();
        match apply_message(&mut self.run, to, text, kind, &source, self.now) {
            Ok(outcome) => {
                let text = message_text(&source, kind, &outcome.text);
                for task_id in &outcome.queued {
                    self.consequences.push(EditConsequence::Message {
                        task_id: task_id.clone(),
                        text: text.clone(),
                    });
                }
                self.consequences.push(EditConsequence::Recipients(outcome));
            }
            Err(error) => self.errors.push(error),
        }
    }

    /// Decision 42e's `refresh` (see [`apply_refresh`]).
    pub(super) fn refresh(&mut self, task_id: &str) {
        let source = self.source.clone();
        if let Err(error) = apply_refresh(&mut self.run, task_id, &source, self.now) {
            self.errors.push(error);
        }
    }

    /// Decision 42c: an `amend_task` of `brief` or `acceptance` releases a paused task.
    pub(super) fn release_pause(&mut self, i: usize) {
        release(&mut self.run.tasks[i], self.now);
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
        if self.epic_being_planned(i) {
            return;
        }
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

    /// Rule `2.epic` (M9.8 second review, ruling 2): the orchestrator does not split,
    /// or add a dependency onto, a task of an epic whose sub-planner is queued or
    /// planning, as it adds no task naming one; that work is the planner's, and its
    /// round is the planner's to submit. Records the error and returns true.
    pub(super) fn epic_being_planned(&mut self, i: usize) -> bool {
        if self.source != EditSource::Orchestrator {
            return false;
        }
        let task = &self.run.tasks[i];
        let Some(epic) = task.spec.epic.clone() else {
            return false;
        };
        let live = self
            .run
            .orch
            .epics
            .iter()
            .any(|e| e.epic == epic && e.phase.is_live());
        if live {
            let id = task.id().to_string();
            let text = format!("epic {epic} is being planned by its sub-planner");
            self.errors
                .push(PlanError::new(Some(&id), "epic", "2.epic", text));
        }
        live
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
