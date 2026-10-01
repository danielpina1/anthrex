//! Milestone 9 decision 44: `C-b g`, the goal form's keys, its tagged `StartGoal`
//! (decision 2) and its replies. The form itself is `crate::run_goal`; the run view it
//! opens on success waits in `App::pending_open` for a snapshot naming the run
//! (`app/runs.rs`). Pure: every request leaves as an `Effect`.

use super::{App, Effect, Modal};
use crate::run_goal::{GoalForm, GoalOutcome, NO_PROJECT, NOT_SENT};
use crate::tree::NodeKey;
use crossterm::event::KeyEvent;
use std::path::PathBuf;

impl App {
    /// The project a goal starts in: the selected project, or the project of the
    /// selected run (any node of it); else the focused window's. Never a guess.
    pub(super) fn goal_project(&self) -> Option<PathBuf> {
        let run_project = |id: &String| {
            (self.runs.runs.iter())
                .find(|run| run.run_id == *id)
                .map(|run| run.project.clone())
        };
        let selected = match &self.tree.selected {
            Some(NodeKey::Project(root)) => Some(root.clone()),
            Some(
                NodeKey::Run(id)
                | NodeKey::Planner { run: id, .. }
                | NodeKey::Scout { run: id, .. }
                | NodeKey::Task { run: id, .. }
                | NodeKey::Stage { run: id, .. }
                | NodeKey::AgentRound { run: id, .. },
            ) => run_project(id),
            _ => None,
        };
        selected.or_else(|| self.focused_window().map(|window| window.project.clone()))
    }

    /// `C-b g`: the form on the chosen project, or the toast saying there is none.
    pub(super) fn open_goal_form(&mut self) -> Vec<Effect> {
        match self.goal_project() {
            Some(project) => {
                let mut form = GoalForm::new(project);
                if let Some(cache) = &self.settings_cache {
                    form.set_roster(cache.doc.models.clone());
                }
                self.modal = Some(Modal::StartGoal(form));
            }
            None => self.toast(NO_PROJECT),
        }
        vec![]
    }

    /// The open goal form's keys. `Enter` sends one tagged `StartGoal` and leaves the
    /// form open, submitting, until its reply.
    pub(crate) fn on_goal_key(&mut self, key: KeyEvent) -> Vec<Effect> {
        let Some(Modal::StartGoal(form)) = &mut self.modal else {
            return vec![];
        };
        match form.on_key(key) {
            GoalOutcome::Stay => vec![],
            GoalOutcome::Cancel => {
                self.modal = None;
                vec![]
            }
            GoalOutcome::Submit(request) => {
                let (id, effect) = self.tagged_request(request);
                if let Some(Modal::StartGoal(form)) = &mut self.modal {
                    form.request_id = Some(id);
                }
                vec![effect]
            }
        }
    }

    /// The goal form while it waits on the tagged request `id`.
    pub(super) fn goal_form_waiting_on(&mut self, id: Option<u64>) -> Option<&mut GoalForm> {
        match &mut self.modal {
            Some(Modal::StartGoal(form)) if form.submitting && id.is_some() => {
                (form.request_id == id).then_some(form)
            }
            _ => None,
        }
    }

    /// Decision 44's success: the form closes, the daemon's message is toasted, and the
    /// run view opens on the new run as soon as a snapshot names it. `run_id: None`
    /// (triage refused the goal) only toasts.
    pub(super) fn goal_triaged(&mut self, run_id: Option<String>, message: &str) {
        self.modal = None;
        self.toast(super::runs::capped(message));
        if let Some(run_id) = run_id {
            self.pending_open = Some(run_id);
            self.open_pending_run();
        }
    }

    /// The `StartGoal` the goal form waits on was refused by the connection, or its
    /// reply went with a lost link: the form stops submitting and says so inline.
    pub(super) fn goal_not_sent(&mut self, id: Option<u64>) {
        if let Some(Modal::StartGoal(form)) = &mut self.modal
            && form.submitting
            && (id.is_none() || form.request_id == id)
        {
            form.submitting = false;
            form.request_id = None;
            form.error = Some(NOT_SENT.into());
        }
    }
}
