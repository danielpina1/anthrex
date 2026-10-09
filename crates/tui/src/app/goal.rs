//! Milestone 9 decision 44: `C-b g`, the goal form's keys, its tagged `StartGoal`
//! (decision 2) and its replies. The form itself is `crate::run_goal`; the run view it
//! opens on success waits in `App::pending_open` for a snapshot naming the run
//! (`app/runs.rs`). Pure: every request leaves as an `Effect`.
//!
//! Milestone 9.3 decisions 8 and 25: the project's draft (`App::goal_drafts`), kept when
//! the dialog closes but by the confirm page's `y`, restored when it opens and cleared
//! by a successful start; and the orchestrator row's chains, from the snapshot.

use super::form_picker::{self, FormPick};
use super::model_picker::PickerFor;
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
                | NodeKey::Round { run: id, .. }
                | NodeKey::AgentRound { run: id, .. },
            ) => run_project(id),
            Some(NodeKey::Chain(chain)) => (self.runs.idle_orchestrators.iter())
                .find(|idle| idle.chain == *chain)
                .map(|idle| idle.project.clone()),
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
                // Final fix wave C-m8: cleaned before it is cut, as the idle row does.
                let id = crate::safe_text::one_line(&run.run_id);
                Some((chain, crate::actions_request::short_id(&id).to_string()))
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
            Some(project) => self.open_goal_form_in(project),
            None => self.toast(NO_PROJECT),
        }
        vec![]
    }

    /// The form on `project`: its draft restored, its orchestrator row from the
    /// snapshot (continue by default when the project has an idle orchestrator). The
    /// idle menu's `new goal here` opens it this way (milestone 9.3 decision 32).
    pub(super) fn open_goal_form_in(&mut self, project: PathBuf) {
        let (idle, busy) = self.goal_chains(&project);
        let draft = self.goal_drafts.get(&project).cloned();
        let mut form = GoalForm::new(project);
        if let Some(cache) = &self.settings_cache {
            form.design_default = cache.doc.design_default;
        }
        form.role_table = self.goal_role_table();
        let row = self.goal_row();
        form.row_aliases = self.catalogs.aliases(&row.model);
        form.row = Some((row.model, row.effort));
        if let Some(draft) = draft {
            form.goal = TextArea::editor(&draft);
        }
        form.set_chains(idle, busy);
        self.modal = Some(Modal::StartGoal(Box::new(form)));
    }

    /// Milestone 9.8 decision 39: the picker's first entry, `role table (<the
    /// orchestrator row's model>)`, from the settings' global table (or the built-in)
    /// and the catalogs' labels.
    pub(super) fn goal_role_table(&self) -> String {
        let row = self.goal_row();
        format!(
            "{} ({})",
            crate::run_goal::ROLE_TABLE,
            self.catalogs.label(&row.model, false)
        )
    }

    /// The orchestrator's row in the settings' global table, else the built-in.
    fn goal_row(&self) -> proto::models::RoleChoice {
        let global = (self.settings_cache.as_ref()).map(|c| c.doc.roles.clone());
        let global = global.unwrap_or_default();
        config::models::resolve(proto::models::Role::Orchestrator, None, &global)
    }

    /// The open goal form's role table entry and its model's efforts, from the
    /// settings and the catalogs as they are now.
    pub(super) fn refresh_goal_models(&mut self) {
        let role_table = self.goal_role_table();
        let row = self.goal_row();
        let catalogs = &self.catalogs;
        if let Some(Modal::StartGoal(form)) = &mut self.modal {
            form.role_table = role_table;
            form.row_aliases = catalogs.aliases(&row.model);
            form.row = Some((row.model, row.effort));
            form.efforts = (form.model.as_ref()).map_or_else(Vec::new, |m| catalogs.efforts(m));
        }
    }

    /// A key while the goal form's picker is open: a choice closes it and is the
    /// form's; `r` asks for a live probe.
    fn on_goal_picker_key(&mut self, key: KeyEvent) -> Vec<Effect> {
        let catalogs = &self.catalogs;
        let Some(Modal::StartGoal(form)) = &mut self.modal else {
            return vec![];
        };
        let Some(picker) = &mut form.picker else {
            return vec![];
        };
        match form_picker::on_key(picker, key) {
            FormPick::Stay => {}
            FormPick::Close => form.picker = None,
            FormPick::Refresh => return vec![form_picker::refresh()],
            FormPick::Chose(model) => {
                let label = (model.as_ref()).map_or_else(String::new, |m| catalogs.label(m, false));
                form.picker = None;
                form.choose(model, label);
            }
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
    /// removes it), or with `keep` false the draft goes (decision 8).
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
    }

    /// Esc while the dialog waits on its request: it closes keeping its draft, and the
    /// request is remembered with the text sent, so its success still clears the draft
    /// (review I1: only this close records it; a reply that closes the dialog does not).
    fn close_goal_form_waiting(&mut self) {
        if let Some(Modal::StartGoal(form)) = &self.modal
            && let (true, Some(id)) = (form.submitting, form.request_id)
        {
            let sent = form.goal.text().to_string();
            self.goal_sent = Some((id, form.project.clone(), sent));
        }
        self.close_goal_form(true);
    }

    /// The open goal form's keys. Ctrl-S (or Enter on an option row) sends one tagged
    /// `StartGoal` and leaves the form open, submitting, until its reply.
    pub(crate) fn on_goal_key(&mut self, key: KeyEvent) -> Vec<Effect> {
        let view = self.goal_view();
        self.refresh_goal_models();
        let catalogs = &self.catalogs;
        let Some(Modal::StartGoal(form)) = &mut self.modal else {
            return vec![];
        };
        if form.picker.is_some() {
            return self.on_goal_picker_key(key);
        }
        match form.on_key_in(key, view) {
            GoalOutcome::Stay => vec![],
            GoalOutcome::Pick => {
                let current = form.model.clone();
                let first = form.role_table.clone();
                form.picker = Some(form_picker::open(
                    PickerFor::Goal,
                    catalogs,
                    current.as_ref(),
                    first,
                ));
                vec![]
            }
            GoalOutcome::Cancel => {
                self.close_goal_form_waiting();
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
    /// opens on the new run as soon as a snapshot names it. `run_id: None` (triage
    /// refused the goal) is a refused start: the form closes keeping its draft, and
    /// only toasts (review m3). A reply for a dialog closed while it waited does the
    /// same to that project's draft; when the project's dialog is open again, its
    /// editor is cleared if it still holds exactly the goal sent, and `run <id>
    /// started` is toasted (review m4). `false` when `request_id` is neither's.
    pub(super) fn goal_started(
        &mut self,
        request_id: Option<u64>,
        run_id: Option<String>,
        message: &str,
    ) -> bool {
        if self.goal_form_waiting_on(request_id).is_some() {
            self.close_goal_form(run_id.is_none());
            self.toast(super::runs::capped(message));
            if let Some(run_id) = run_id {
                self.pending_open = Some(run_id);
                self.open_pending_run();
            }
            return true;
        }
        let Some((_, project, sent)) = self.take_goal_sent(request_id) else {
            return false;
        };
        let Some(run_id) = run_id else {
            self.toast(super::runs::capped(message));
            return true;
        };
        self.goal_drafts.remove(&project);
        match &mut self.modal {
            Some(Modal::StartGoal(form)) if form.project == project && !form.submitting => {
                if form.goal.text() == sent {
                    form.goal = TextArea::editor("");
                }
                self.toast(super::runs::capped(&format!("run {run_id} started")));
            }
            _ => self.toast(super::runs::capped(message)),
        }
        true
    }

    /// The closed dialog's request, when `request_id` is its reply's.
    fn take_goal_sent(&mut self, request_id: Option<u64>) -> Option<(u64, PathBuf, String)> {
        match self.goal_sent.take() {
            Some(sent) if Some(sent.0) == request_id => Some(sent),
            other => {
                self.goal_sent = other;
                None
            }
        }
    }

    /// A refusal ends a closed dialog's wait: its draft stays (decision 8).
    pub(super) fn goal_refused(&mut self, request_id: Option<u64>) {
        self.take_goal_sent(request_id);
    }

    /// The `StartGoal` the goal form waits on was refused by the connection, or its
    /// reply went with a lost link: the form stops submitting and says so inline. A
    /// closed dialog's wait on it ends too (final fix wave C-m9): on a lost link
    /// whatever its id, since no reply can come; its draft stays.
    pub(super) fn goal_not_sent(&mut self, id: Option<u64>) {
        match id {
            Some(_) => {
                self.take_goal_sent(id);
            }
            None => self.goal_sent = None,
        }
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
