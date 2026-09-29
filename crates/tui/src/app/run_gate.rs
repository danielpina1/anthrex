//! Milestone 8c decisions 32–34: the plan gate's keys (`a`, `x`, `e`, `d`), the task
//! edit form's keys and its replies, and the stale-modal check after each snapshot.
//! Moved out of `app/runs.rs` (`AGENTS.md` hard rule 8) before milestone 9 adds holds.

use super::{App, Effect, Modal, PendingAction};
use crate::run_edit::{EditOutcome, TaskEditForm};
use crate::tree::NodeKey;
use crossterm::event::KeyEvent;
use proto::{ClientMsg, RunInfo, RunPath, RunRequest, RunState, TaskInfo, TaskState};

use super::runs::state_text;

fn confirm(message: String, action: PendingAction) -> Modal {
    Modal::Confirm { message, action }
}

/// Decision 32's approve confirm: the tasks not `cancelled` are the ones that start.
fn approve_message(run: &RunInfo) -> String {
    let run_id = &run.run_id;
    let starting = run.tasks.iter().filter(|t| t.state != TaskState::Cancelled);
    match starting.count() {
        1 => format!("Approve run {run_id}? 1 task starts."),
        n => format!("Approve run {run_id}? {n} tasks start."),
    }
}

impl App {
    /// Decision 32: `Ok` with the run while it awaits approval — the gate is open —
    /// else the toast saying why it is closed.
    fn gate_run(&self, run_id: &str) -> Result<&RunInfo, String> {
        let closed = |why: &str| format!("the plan gate is closed: run {run_id} is {why}");
        match self.runs.runs.iter().find(|run| run.run_id == run_id) {
            Some(run) if run.state == RunState::AwaitingApproval => Ok(run),
            Some(run) if run.path == Some(RunPath::Fast) => Err(closed("on the fast path")),
            Some(run) => Err(closed(state_text(run.state))),
            None => Err(closed("gone")),
        }
    }

    /// Decision 32's four keys: `a` and `x` ask to approve or reject the run, `d` to
    /// remove the selected task, `e` opens the edit form on it. Nothing is sent here.
    pub(super) fn on_gate_key(&mut self, run_id: String, key: char) {
        let run = match self.gate_run(&run_id) {
            Ok(run) => run,
            Err(text) => return self.toast(text),
        };
        let task = match &self.tree.selected {
            Some(NodeKey::Task { run: r, id }) if *r == run_id => {
                run.tasks.iter().find(|task| task.id == *id)
            }
            _ => None,
        };
        let modal = match (key, task) {
            ('a', _) => confirm(approve_message(run), PendingAction::ApproveRun(run_id)),
            ('x', _) => confirm(
                format!(
                    "Reject run {run_id}? Its branches and worktrees are removed; \
                     salvage refs are kept."
                ),
                PendingAction::RejectRun(run_id),
            ),
            ('e', Some(task)) => Modal::EditTask(TaskEditForm::new(&run_id, task)),
            (_, Some(task)) => confirm(
                format!("Remove {} from run {run_id}'s plan?", task.id),
                PendingAction::RemoveTask {
                    task_id: task.id.clone(),
                    run_id,
                },
            ),
            (_, None) => return self.toast("select a task to edit or remove"),
        };
        self.modal = Some(modal);
    }

    /// Decision 33: the open edit form's keys. `Enter` sends the one `Edit` and leaves
    /// the form open, submitting, until its reply (decision 34).
    pub(crate) fn on_edit_task_key(&mut self, key: KeyEvent) -> Vec<Effect> {
        let Some(Modal::EditTask(form)) = &mut self.modal else {
            return vec![];
        };
        match form.on_key(key) {
            EditOutcome::Stay => vec![],
            EditOutcome::Cancel => {
                self.modal = None;
                vec![]
            }
            EditOutcome::Unchanged => {
                self.modal = None;
                self.toast("nothing changed");
                vec![]
            }
            EditOutcome::Submit(edits) => {
                let run_id = form.run_id.clone();
                vec![Effect::Send(ClientMsg::Run(RunRequest::Edit {
                    run_id,
                    edits,
                    submit: false,
                }))]
            }
        }
    }

    /// The edit form while it waits for its `run edit` reply.
    pub(super) fn submitting_form(&mut self) -> Option<&mut TaskEditForm> {
        match &mut self.modal {
            Some(Modal::EditTask(form)) if form.submitting => Some(form),
            _ => None,
        }
    }

    /// Whole-branch review M2: the `Edit` a submitting form waits on was refused by the
    /// connection, or its reply went with a lost link, so no reply will come. The form
    /// stops submitting and says so inline; `Enter` sends it again.
    pub(super) fn edit_not_sent(&mut self) {
        if let Some(form) = self.submitting_form() {
            form.submitting = false;
            form.error = Some("the edit was not sent; press Enter to retry".into());
        }
    }

    /// Decision 32's task while the gate is open, else the toast saying why not.
    fn gate_task(&self, run_id: &str, task_id: &str) -> Result<&TaskInfo, String> {
        let gone = || format!("{task_id} is no longer in run {run_id}'s plan");
        let run = self.gate_run(run_id)?;
        run.tasks.iter().find(|t| t.id == task_id).ok_or_else(gone)
    }

    /// After every snapshot (review I1 and M6): an open gate `Confirm` or edit form
    /// whose request could now only be refused, or would do other than it said, closes
    /// with a toast saying why. A `y` or `Enter` after it sends nothing.
    pub(super) fn close_gate_modal_if_stale(&mut self) {
        let text = match &self.modal {
            Some(Modal::EditTask(form)) => self.stale_form(form),
            Some(Modal::Confirm { message, action }) => self.stale_confirm(message, action),
            _ => None,
        };
        if let Some(text) = text {
            self.modal = None;
            self.toast(text);
        }
    }

    /// The gate closed, the task gone, or — while not submitting, since our own edit
    /// changes these — the task's values changed elsewhere, so its route would revert.
    fn stale_form(&self, form: &TaskEditForm) -> Option<String> {
        let (run_id, task_id) = (&form.run_id, &form.task_id);
        match self.gate_task(run_id, task_id) {
            Err(text) => Some(text),
            Ok(task) if !form.submitting && !form.opened_from(task) => {
                Some(format!("{task_id} changed in run {run_id}; press e again"))
            }
            Ok(_) => None,
        }
    }

    /// The gate closed; for a remove, its task gone or finished; for an approve, a
    /// different number of tasks would start than the confirm says.
    fn stale_confirm(&self, message: &str, action: &PendingAction) -> Option<String> {
        match action {
            PendingAction::ApproveRun(run_id) => match self.gate_run(run_id) {
                Err(text) => Some(text),
                Ok(run) if approve_message(run) != message => {
                    Some(format!("run {run_id}'s plan changed; press a again"))
                }
                Ok(_) => None,
            },
            PendingAction::RejectRun(run_id) => self.gate_run(run_id).err(),
            PendingAction::RemoveTask { run_id, task_id } => {
                match self.gate_task(run_id, task_id) {
                    Err(text) => Some(text),
                    Ok(task) if task.state.is_finished() => {
                        Some(format!("{task_id} is no longer in run {run_id}'s plan"))
                    }
                    Ok(_) => None,
                }
            }
            _ => None,
        }
    }
}
