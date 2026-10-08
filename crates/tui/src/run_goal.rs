//! Milestone 9 decision 44 and (9.0.6) decision 39: the goal form `C-b g` opens, pure
//! (`AGENTS.md` hard rule 5). It holds a goal (a text area; `Ctrl-J` inserts a newline),
//! the orchestrator's model chosen from the CLIs' models in the model picker (milestone
//! 9.8 decision 39; `role table` is the orchestrator row's) and its effort, and three
//! toggles that start off: `trust`,
//! `approve at once` and `unconfined checks`, for one project the caller chose. It
//! builds M8b's `RunRequest::StartGoal` exactly as `anthrex run start --goal` sends it:
//! with the toggles off the plan gate stays on and checks stay confined. Opening,
//! sending and the replies are `app/goal.rs`; rendering is `ui/run_goal.rs` and
//! `ui/goal_editor.rs`.
//!
//! Milestone 9.3 decisions 7, 8 and 25 (KG §1, §3.3): the goal is the nano-like editor
//! (`TextArea::on_editor_key`, Enter a newline), Ctrl-S starts from anywhere and Enter
//! from an option row, Esc on a text asks before discarding it, and the orchestrator
//! row continues the project's idle orchestrator (`continue_from`) or starts a new one.

use crate::app::form_picker::cycle_effort;
use crate::app::model_picker::ModelPicker;
use crate::text_area::EditorKey;
use crate::text_area::TextArea;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use proto::models::ModelRef;
use proto::{DeliveryMode, IdleOrchestrator, OrchestratorChoice, RunRequest};
use std::path::PathBuf;

/// The toast `C-b g` shows when no project is selected, no window is focused and the
/// TUI's start directory is empty (milestone 9.0.7 decision 37's fallback).
pub const NO_PROJECT: &str = "select a Git project to start a goal";
/// The inline error of a start (Ctrl-S, or Enter on an option row) with a blank goal.
pub const EMPTY_GOAL: &str = "type a goal first";
/// The inline error after the connection refused the request or the link was lost.
pub const NOT_SENT: &str = "the goal was not sent; press Ctrl-S to retry";
/// Decision 8's confirm page (KG §1.4, exact): its question and its keys.
pub const DISCARD_ASK: &str = "discard this goal text?";
pub const DISCARD_KEYS: &str = "y discard · any other key back";

/// The goal's text area as the dialog draws it (`ui::goal_editor::text_view`): the
/// width it wraps at and its visible rows. The keys and the renderer take the same one.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct EditorView {
    pub width: u16,
    pub rows: u16,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GoalField {
    Goal,
    /// Milestone 9.8 decision 39: `⏎` opens the model picker.
    Model,
    /// The chosen model's catalog efforts, then its default.
    Effort,
    /// Milestone 9.3 decision 25: continue the project's idle orchestrator, or a new one.
    Orchestrator,
    /// Milestone 9.2 ruling R-13: `run start --goal --delivery`.
    Delivery,
    /// Milestone 9.6 (DF §6.2): `run start --goal --design`.
    Design,
    Trust,
    Yes,
    UnconfinedChecks,
}

const FIELDS: [GoalField; 9] = [
    GoalField::Goal,
    GoalField::Model,
    GoalField::Effort,
    GoalField::Orchestrator,
    GoalField::Delivery,
    GoalField::Design,
    GoalField::Trust,
    GoalField::Yes,
    GoalField::UnconfinedChecks,
];

/// What the model row reads before a choice, until the app names the row.
pub const ROLE_TABLE: &str = "role table";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GoalForm {
    /// The project the goal runs in, chosen when the form opened; never changed.
    pub project: PathBuf,
    /// Newlines are typed with `Ctrl-J` and sent as they are.
    pub goal: TextArea,
    /// Milestone 9.8 decision 39: the orchestrator's model; `None` is the role table's
    /// orchestrator row (no choice sent).
    pub model: Option<ModelRef>,
    /// The chosen model as the picker labels it (`Codex · gpt-6.1 sol`).
    pub model_label: String,
    /// `role table (<the orchestrator row's model>)`, set by the app.
    pub role_table: String,
    /// The chosen model's catalog efforts, refreshed by the app before each key.
    pub efforts: Vec<String>,
    /// `None`: the model's default.
    pub effort: Option<String>,
    /// The model picker, while it is open over the form.
    pub picker: Option<ModelPicker>,
    /// Milestone 9.2 ruling R-13: `None` is the repo profile's `[delivery] mode`
    /// (`configured`), else `local` or `pr` for this run (`RunRequest::StartGoal.delivery`).
    pub delivery: Option<DeliveryMode>,
    /// Milestone 9.6: `None` is `configured`, the daemon's choice (DF §1's table, then
    /// `[orchestrator.design].default`), else `full` or `off` (`StartGoal.design`).
    pub design: Option<proto::DesignMode>,
    /// Ruling T18-2: the settings' `[orchestrator.design].default` as of the last
    /// settings cache, which `configured` names; `None` while no cache has arrived.
    pub design_default: Option<proto::DesignMode>,
    pub trust_project: bool,
    /// `approve at once`: the plan gate is skipped.
    pub yes: bool,
    pub unconfined_checks: bool,
    pub focus: GoalField,
    pub error: Option<String>,
    pub submitting: bool,
    /// The id of the tagged `StartGoal` this form waits on (decision 2).
    pub request_id: Option<u64>,
    /// Milestone 9.3 decision 25: the project's idle orchestrator from the snapshot,
    /// whether the goal continues it (the default while there is one), and, with none,
    /// an active chain and its run's short id (`o-3f9a is working on run …`).
    pub idle: Option<IdleOrchestrator>,
    pub continuing: bool,
    pub busy: Option<(String, String)>,
    /// Decision 8: the confirm page `Esc` on a text opened.
    pub discarding: bool,
}

// Protocol 19 grew `ClientMsg`/`RunRequest` (the role table, `OrchestratorChoice.effort`); these
// values are built once per key press, so boxing would only add noise.
#[allow(clippy::large_enum_variant)]
#[derive(Debug, Clone, PartialEq)]
pub enum GoalOutcome {
    Stay,
    /// The dialog closes; the app keeps its text as the project's draft.
    Cancel,
    /// The confirm page's `y`: the dialog closes and the draft goes.
    Discard,
    Submit(RunRequest),
    /// `⏎` on the model row: the app opens the picker.
    Pick,
}

/// What the orchestrator row shows: a choice, or the active chain's muted line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OrchestratorRow {
    Choice(String),
    Busy(String),
}

pub fn field_label(field: GoalField) -> &'static str {
    match field {
        GoalField::Goal => "goal",
        GoalField::Model => "model",
        GoalField::Effort => "effort",
        GoalField::Orchestrator => "orchestrator",
        GoalField::Delivery => "delivery",
        GoalField::Design => "design",
        GoalField::Trust => "trust",
        GoalField::Yes => "approve at once",
        GoalField::UnconfinedChecks => "unconfined checks",
    }
}

#[path = "run_goal_helpers.rs"]
mod helpers;
pub(crate) use helpers::cycle;
use helpers::{is_ctrl, next_delivery, next_design};

impl GoalForm {
    pub fn new(project: PathBuf) -> Self {
        Self {
            project,
            goal: TextArea::editor(""),
            model: None,
            model_label: String::new(),
            role_table: ROLE_TABLE.to_string(),
            efforts: Vec::new(),
            effort: None,
            picker: None,
            delivery: None,
            design: None,
            design_default: None,
            trust_project: false,
            yes: false,
            unconfined_checks: false,
            focus: GoalField::Goal,
            error: None,
            submitting: false,
            request_id: None,
            idle: None,
            continuing: false,
            busy: None,
            discarding: false,
        }
    }

    /// The project's chains from a snapshot (decision 25): a new idle orchestrator is
    /// continued by default; the same one keeps the user's choice.
    pub fn set_chains(&mut self, idle: Option<IdleOrchestrator>, busy: Option<(String, String)>) {
        let same = self.idle.as_ref().map(|i| &i.chain) == idle.as_ref().map(|i| &i.chain);
        if !same {
            self.continuing = idle.is_some();
        }
        self.idle = idle;
        self.busy = busy;
    }

    /// The idle orchestrator this goal continues, while continue is chosen.
    pub fn continues(&self) -> Option<&IdleOrchestrator> {
        self.idle.as_ref().filter(|_| self.continuing)
    }

    /// Decision 25's row (KG §3.3, §3.5, §3.6), its text unsanitised: the renderer
    /// cleans it. The run's short id is cleaned before it is cut (final fix wave C-m8),
    /// so a carrier never shortens the drawn `<h4>`.
    pub fn orchestrator_row(&self) -> OrchestratorRow {
        match (&self.idle, &self.busy) {
            (Some(idle), _) if self.continuing && idle.fresh => {
                OrchestratorRow::Choice(format!("continue {} (fresh session)", idle.chain))
            }
            (Some(idle), _) if self.continuing => OrchestratorRow::Choice(format!(
                "continue {} (after {})",
                idle.chain,
                crate::actions_request::short_id(&crate::safe_text::one_line(&idle.after_run))
            )),
            (None, Some((chain, run))) => OrchestratorRow::Busy(format!(
                "{chain} is working on run {run}; this goal gets a new orchestrator"
            )),
            _ => OrchestratorRow::Choice("new".into()),
        }
    }

    /// Decision 39: the picker chose `model` (`None`: the role table), labelled
    /// `label`; a new model's effort starts at its default, the same one keeps it.
    pub fn choose(&mut self, model: Option<ModelRef>, label: String) {
        // Review I1: the same model again keeps its effort.
        if model.is_some() && model == self.model {
            self.model_label = label;
            return;
        }
        self.model = model;
        self.model_label = label;
        self.effort = None;
        self.efforts.clear();
    }

    fn move_focus(&mut self, delta: isize) {
        let at = FIELDS.iter().position(|f| *f == self.focus).unwrap_or(0) as isize;
        let next = (at + delta).rem_euclid(FIELDS.len() as isize);
        self.focus = FIELDS[next as usize];
    }

    /// [`GoalForm::on_key_in`] with the goal unwrapped.
    pub fn on_key(&mut self, key: KeyEvent) -> GoalOutcome {
        self.on_key_in(key, EditorView::default())
    }

    /// Decisions 7 and 8's keys (KG §1.2, §1.4), the goal drawn as `view` says. The
    /// confirm page takes the next key: `y` discards, any other goes back to the text.
    /// `Esc` and `Ctrl-C` close at once while submitting (only they act then, so a
    /// second start sends nothing) or on an empty text, and ask on any other. Ctrl-S
    /// starts from anywhere; the text takes every editor key (Enter a newline), and Tab
    /// and Shift-Tab, which it hands back, move to the options; on an option row Tab,
    /// Shift-Tab, Up and Down move (wrapping), Enter starts and the rest change it.
    pub fn on_key_in(&mut self, key: KeyEvent, view: EditorView) -> GoalOutcome {
        if self.discarding {
            self.discarding = false;
            if key.code == KeyCode::Char('y')
                && !key
                    .modifiers
                    .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT)
            {
                return GoalOutcome::Discard;
            }
            self.focus = GoalField::Goal;
            return GoalOutcome::Stay;
        }
        if key.code == KeyCode::Esc || is_ctrl(&key, 'c') {
            if self.submitting || self.goal.is_empty() {
                return GoalOutcome::Cancel;
            }
            self.discarding = true;
            return GoalOutcome::Stay;
        }
        if self.submitting {
            return GoalOutcome::Stay;
        }
        if is_ctrl(&key, 's') {
            return self.submit();
        }
        if self.focus == GoalField::Goal {
            if self.goal.on_editor_key(key, view.width, view.rows) == EditorKey::Unhandled {
                match key.code {
                    KeyCode::Tab => self.move_focus(1),
                    KeyCode::BackTab => self.move_focus(-1),
                    _ => {}
                }
            }
            return GoalOutcome::Stay;
        }
        match key.code {
            KeyCode::Tab | KeyCode::Down => self.move_focus(1),
            KeyCode::BackTab | KeyCode::Up => self.move_focus(-1),
            KeyCode::Enter if self.focus == GoalField::Model && self.continues().is_none() => {
                return GoalOutcome::Pick;
            }
            KeyCode::Enter => return self.submit(),
            _ => self.on_field_key(key),
        }
        GoalOutcome::Stay
    }

    fn on_field_key(&mut self, key: KeyEvent) {
        let toggle = matches!(
            key.code,
            KeyCode::Char(' ') | KeyCode::Left | KeyCode::Right
        );
        let locked = self.continues().is_some();
        match self.focus {
            // The text's keys are `on_key_in`'s; continuing, the chain's runtime and
            // model hold (decision 25).
            GoalField::Goal | GoalField::Model => {}
            GoalField::Effort if locked || self.model.is_none() => {}
            GoalField::Orchestrator if toggle && self.idle.is_some() => {
                self.continuing = !self.continuing;
            }
            GoalField::Orchestrator => {}
            GoalField::Effort => match key.code {
                KeyCode::Right | KeyCode::Char(' ') => {
                    self.effort = cycle_effort(&self.efforts, self.effort.as_deref(), true);
                }
                KeyCode::Left => {
                    self.effort = cycle_effort(&self.efforts, self.effort.as_deref(), false);
                }
                _ => {}
            },
            GoalField::Delivery => match key.code {
                KeyCode::Right | KeyCode::Char(' ') => {
                    self.delivery = next_delivery(self.delivery, true)
                }
                KeyCode::Left => self.delivery = next_delivery(self.delivery, false),
                _ => {}
            },
            GoalField::Design => match key.code {
                KeyCode::Right | KeyCode::Char(' ') => self.design = next_design(self.design, true),
                KeyCode::Left => self.design = next_design(self.design, false),
                _ => {}
            },
            GoalField::Trust if toggle => self.trust_project = !self.trust_project,
            GoalField::Yes if toggle => self.yes = !self.yes,
            GoalField::UnconfinedChecks if toggle => {
                self.unconfined_checks = !self.unconfined_checks;
            }
            GoalField::Trust | GoalField::Yes | GoalField::UnconfinedChecks => {}
        }
    }

    /// [`GoalForm::on_paste_in`] with the goal unwrapped.
    pub fn on_paste(&mut self, text: &str) {
        self.on_paste_in(text, EditorView::default());
    }

    /// A bracketed paste into the goal (the editor's, decision 5): it keeps its line
    /// breaks; the goal is drawn as `view` says (the editor's viewport follows it). The
    /// picker's custom name takes a paste while it is open (`app/paste.rs`).
    pub fn on_paste_in(&mut self, text: &str, view: EditorView) {
        if self.submitting || self.discarding {
            return;
        }
        if self.focus == GoalField::Goal {
            self.goal.on_editor_paste(text, view.width, view.rows);
        }
    }

    fn submit(&mut self) -> GoalOutcome {
        match self.request() {
            Ok(request) => {
                self.submitting = true;
                self.error = None;
                GoalOutcome::Submit(request)
            }
            Err((field, message)) => {
                self.focus = field;
                self.error = Some(message.to_string());
                GoalOutcome::Stay
            }
        }
    }

    /// Decision 44's request: what `anthrex run start --goal` sends, with the
    /// orchestrator choice when a model is chosen (milestone 9.8 decision 39: its
    /// runtime, its id, its effort), and decision 39's toggles; while
    /// continuing (decision 25), `continue_from` the chain's last run and no
    /// orchestrator choice, as `run start --goal … --continue` sends it.
    pub fn request(&self) -> Result<RunRequest, (GoalField, &'static str)> {
        let goal = self.goal.text();
        if goal.trim().is_empty() {
            return Err((GoalField::Goal, EMPTY_GOAL));
        }
        let continue_from = self.continues().map(|idle| idle.after_run.clone());
        let orchestrator = (self.model.as_ref())
            .filter(|_| continue_from.is_none())
            .map(|m| OrchestratorChoice {
                runtime: m.runtime,
                model: m.id.clone(),
                effort: self.effort.clone(),
            });
        Ok(RunRequest::StartGoal {
            goal: goal.trim().to_string(),
            dir: self.project.clone(),
            yes: self.yes,
            trust_project: self.trust_project,
            unconfined_checks: self.unconfined_checks,
            orchestrator,
            delivery: self.delivery,
            continue_from,
            design: self.design,
        })
    }
}

#[cfg(test)]
#[path = "run_goal_tests.rs"]
mod tests;
