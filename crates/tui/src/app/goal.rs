//! Milestone 9 decision 44: `C-b g`, the goal form's keys, its tagged `StartGoal`
//! (decision 2) and its replies. The form itself is `crate::run_goal`; the run view it
//! opens on success waits in `App::pending_open` for a snapshot naming the run
//! (`app/runs.rs`). Pure: every request leaves as an `Effect`.
//!
//! Milestone 9.3 decisions 8 and 25: the project's draft (`App::goal_drafts`), kept when
//! the dialog closes but by the confirm page's `y`, restored when it opens and cleared
//! by a successful start; and the orchestrator row's chains, from the snapshot.

use super::{App, Effect, Modal};
use crate::run_goal::{EditorView, GoalForm, GoalOutcome, NO_PROJECT, NOT_SENT};
use crate::text_area::TextArea;
use crate::tree::NodeKey;
use crossterm::event::KeyEvent;
use proto::IdleOrchestrator;
use std::path::{Path, PathBuf};

impl App {
    /// The project a goal starts in: the selected project, or the project of the
    /// selected run (any node of it); else the focused window's; else, on an empty
    /// session (9.0.7 decision 37), the TUI's start directory when it has one. Whether
    /// that is a Git repository is the daemon's to say (`not a git repository: …`); the
    /// client probes nothing.
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
        selected
            .or_else(|| self.focused_window().map(|window| window.project.clone()))
            .or_else(|| {
                (!self.default_dir.as_os_str().is_empty()).then(|| self.default_dir.clone())
            })
    }

    /// Decision 25: `project`'s idle orchestrator in the snapshot, else an active chain
    /// there (a run not terminal that carries a chain) with its run's short id.
    fn goal_chains(&self, project: &Path) -> (Option<IdleOrchestrator>, Option<(String, String)>) {
        let snapshot = &self.runs;
        let idle = (snapshot.idle_orchestrators.iter())
            .find(|idle| idle.project == project)
            .cloned();
        let busy = (snapshot.runs.iter())
            .filter(|run| run.project == project && !run.state.is_terminal())
            .find_map(|run| {
                let chain = run.chain.clone()?;
                Some((
                    chain,
                    crate::actions_request::short_id(&run.run_id).to_string(),
                ))
            });
        (idle, busy)
    }

    /// A new snapshot: the open goal form's orchestrator row follows it.
    pub(super) fn refresh_goal_chains(&mut self) {
        let Some(Modal::StartGoal(form)) = &self.modal else {
            return;
        };
        let (idle, busy) = self.goal_chains(&form.project.clone());
        if let Some(Modal::StartGoal(form)) = &mut self.modal {
            form.set_chains(idle, busy);
        }
    }

    /// `C-b g`: the form on the chosen project, its draft restored with the cursor at
    /// its end (decision 8), or the toast saying there is none (an empty start
    /// directory).
    pub(super) fn open_goal_form(&mut self) -> Vec<Effect> {
        match self.goal_project() {
            Some(project) => {
                let (idle, busy) = self.goal_chains(&project);
                let draft = self.goal_drafts.get(&project).cloned();
                let mut form = GoalForm::new(project);
                if let Some(cache) = &self.settings_cache {
                    form.set_roster(cache.doc.models.clone());
                }
                if let Some(draft) = draft {
                    form.goal = TextArea::editor(&draft);
                }
                form.set_chains(idle, busy);
                self.modal = Some(Modal::StartGoal(form));
            }
            None => self.toast(NO_PROJECT),
        }
        vec![]
    }

    /// The open goal dialog's text area as the last frame drew it (decision 7): the
    /// terminal is the body plus the status bar's row. The keys and the paste take it.
    pub(super) fn goal_view(&self) -> EditorView {
        let Some(Modal::StartGoal(form)) = &self.modal else {
            return EditorView::default();
        };
        let area = self.body_area;
        let rows = if area.height == 0 { 0 } else { area.height + 1 };
        crate::ui::goal_editor::text_view(form, area.width, rows)
    }

    /// Closes the goal dialog: its text becomes the project's draft (an empty text
    /// removes it), or with `keep` false the draft goes. A request still in flight is
    /// remembered, so its success clears the draft (decision 8).
    fn close_goal_form(&mut self, keep: bool) {
        let Some(Modal::StartGoal(form)) = self.modal.take() else {
            return;
        };
        let text = form.goal.text();
        if keep && !text.is_empty() {
            self.goal_drafts
                .insert(form.project.clone(), text.to_string());
        } else {
            self.goal_drafts.remove(&form.project);
        }
        if let (true, Some(id)) = (form.submitting, form.request_id) {
            self.goal_sent = Some((id, form.project));
        }
    }

    /// The open goal form's keys. Ctrl-S (or Enter on an option row) sends one tagged
    /// `StartGoal` and leaves the form open, submitting, until its reply.
    pub(crate) fn on_goal_key(&mut self, key: KeyEvent) -> Vec<Effect> {
        let view = self.goal_view();
        let Some(Modal::StartGoal(form)) = &mut self.modal else {
            return vec![];
        };
        match form.on_key_in(key, view) {
            GoalOutcome::Stay => vec![],
            GoalOutcome::Cancel => {
                self.close_goal_form(true);
                vec![]
            }
            GoalOutcome::Discard => {
                self.close_goal_form(false);
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

    /// Decision 44's success (`Triaged`, or milestone 9.3's `Started` for a continued
    /// goal): the form closes, the draft goes, `message` is toasted, and the run view
    /// opens on the new run as soon as a snapshot names it. `run_id: None` only toasts.
    /// A success for a dialog closed while it waited clears that project's draft and is
    /// toasted. `false` when `request_id` is neither's.
    pub(super) fn goal_started(
        &mut self,
        request_id: Option<u64>,
        run_id: Option<String>,
        message: &str,
    ) -> bool {
        if self.goal_form_waiting_on(request_id).is_some() {
            self.close_goal_form(false);
            self.toast(super::runs::capped(message));
            if let Some(run_id) = run_id {
                self.pending_open = Some(run_id);
                self.open_pending_run();
            }
            return true;
        }
        match self.goal_sent.take() {
            Some((id, project)) if Some(id) == request_id => {
                self.goal_drafts.remove(&project);
                self.toast(super::runs::capped(message));
                true
            }
            other => {
                self.goal_sent = other;
                false
            }
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
