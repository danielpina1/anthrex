//! Milestone 9 decision 44 and (9.0.6) decision 39: the goal form `C-b g` opens, pure
//! (`AGENTS.md` hard rule 5). It holds a goal (a text area; `Ctrl-J` inserts a newline),
//! an optional orchestrator runtime, a model picked from that runtime's enabled models
//! (or typed after `custom…`), and three toggles that start off: `trust`,
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

use crate::app::screens::models_of;
use crate::dialog::{TextInput, apply_text_key};
use crate::run_edit::TEXT_MAX_CHARS;
use crate::text_area::EditorKey;
use crate::text_area::TextArea;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use proto::{DeliveryMode, IdleOrchestrator, ModelEntry, OrchestratorChoice, RunRequest, Runtime};
use std::path::PathBuf;

/// The toast `C-b g` shows when no project is selected, no window is focused and the
/// TUI's start directory is empty (milestone 9.0.7 decision 37's fallback).
pub const NO_PROJECT: &str = "select a Git project to start a goal";
/// The inline error of an `Enter` with a blank goal.
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

impl EditorView {
    /// PgUp and PgDn's page (decision 5): the visible rows less one.
    pub fn page(self) -> usize {
        usize::from(self.rows.saturating_sub(1))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GoalField {
    Goal,
    Runtime,
    Model,
    /// Milestone 9.3 decision 25: continue the project's idle orchestrator, or a new one.
    Orchestrator,
    /// Milestone 9.2 ruling R-13: `run start --goal --delivery`.
    Delivery,
    Trust,
    Yes,
    UnconfinedChecks,
}

const FIELDS: [GoalField; 8] = [
    GoalField::Goal,
    GoalField::Runtime,
    GoalField::Model,
    GoalField::Orchestrator,
    GoalField::Delivery,
    GoalField::Trust,
    GoalField::Yes,
    GoalField::UnconfinedChecks,
];

/// The model choice (decision 39): the runtime's own default, one of the roster's
/// enabled models of the chosen runtime (an index into [`GoalForm::models`]), or text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GoalModel {
    Default,
    Pick(usize),
    /// Selects [`GoalForm::custom`], whose text survives a move of the picker.
    Custom,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GoalForm {
    /// The project the goal runs in, chosen when the form opened; never changed.
    pub project: PathBuf,
    /// Newlines are typed with `Ctrl-J` and sent as they are.
    pub goal: TextArea,
    /// `None` is the configured orchestrator (`[orchestrator.agent]`, then the default).
    pub runtime: Option<Runtime>,
    pub model: GoalModel,
    /// Milestone 9.2 ruling R-13: `None` is the repo profile's `[delivery] mode`
    /// (`configured`), else `local` or `pr` for this run (`RunRequest::StartGoal.delivery`).
    pub delivery: Option<DeliveryMode>,
    /// The text typed after `custom…`; kept while the picker moves away and back.
    pub custom: TextInput,
    /// The settings cache's roster as of the last `set_roster` (decision 24); empty
    /// while no cache has arrived.
    pub roster: Vec<ModelEntry>,
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

#[derive(Debug, Clone, PartialEq)]
pub enum GoalOutcome {
    Stay,
    /// The dialog closes; the app keeps its text as the project's draft.
    Cancel,
    /// The confirm page's `y`: the dialog closes and the draft goes.
    Discard,
    Submit(RunRequest),
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
        GoalField::Runtime => "runtime",
        GoalField::Model => "model",
        GoalField::Orchestrator => "orchestrator",
        GoalField::Delivery => "delivery",
        GoalField::Trust => "trust",
        GoalField::Yes => "approve at once",
        GoalField::UnconfinedChecks => "unconfined checks",
    }
}

/// A tab becomes a space, and every control or hidden format character is dropped (a
/// pasted line break too: the custom model is one line).
fn clean_line(text: &str) -> String {
    text.chars()
        .filter_map(|c| match c {
            '\t' => Some(' '),
            c if c.is_control() || crate::safe_text::is_hidden_format(c) => None,
            c => Some(c),
        })
        .collect()
}

fn is_ctrl(key: &KeyEvent, c: char) -> bool {
    key.modifiers.contains(KeyModifiers::CONTROL)
        && matches!(key.code, KeyCode::Char(k) if k.eq_ignore_ascii_case(&c))
}

fn insert_bounded(input: &mut TextInput, text: &str) {
    let room = TEXT_MAX_CHARS.saturating_sub(input.text().chars().count());
    if room == 0 || text.is_empty() {
        return;
    }
    let cut: String = text.chars().take(room).collect();
    input.insert(&cut);
}

/// `configured`, `local`, `pr`, round.
fn next_delivery(value: Option<DeliveryMode>, forward: bool) -> Option<DeliveryMode> {
    let order = [None, Some(DeliveryMode::Local), Some(DeliveryMode::Pr)];
    let at = order.iter().position(|v| *v == value).unwrap_or(0);
    let len = order.len();
    order[if forward {
        (at + 1) % len
    } else {
        (at + len - 1) % len
    }]
}

fn next_runtime(value: Option<Runtime>, forward: bool) -> Option<Runtime> {
    let order = [None, Some(Runtime::Claude), Some(Runtime::Codex)];
    let at = order.iter().position(|v| *v == value).unwrap_or(0);
    let len = order.len();
    order[if forward {
        (at + 1) % len
    } else {
        (at + len - 1) % len
    }]
}

impl GoalForm {
    pub fn new(project: PathBuf) -> Self {
        Self {
            project,
            goal: TextArea::editor(""),
            runtime: None,
            model: GoalModel::Default,
            delivery: None,
            custom: TextInput::default(),
            roster: Vec::new(),
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

    /// Whether the custom model's text row is drawn: `custom…` chosen, and the model not
    /// held by a continued chain.
    pub fn custom_shown(&self) -> bool {
        self.model == GoalModel::Custom && self.continues().is_none()
    }

    /// The idle orchestrator this goal continues, while continue is chosen.
    pub fn continues(&self) -> Option<&IdleOrchestrator> {
        self.idle.as_ref().filter(|_| self.continuing)
    }

    /// Decision 25's row (KG §3.3, §3.5, §3.6), its text unsanitised: the renderer
    /// cleans it.
    pub fn orchestrator_row(&self) -> OrchestratorRow {
        match (&self.idle, &self.busy) {
            (Some(idle), _) if self.continuing && idle.fresh => {
                OrchestratorRow::Choice(format!("continue {} (fresh session)", idle.chain))
            }
            (Some(idle), _) if self.continuing => OrchestratorRow::Choice(format!(
                "continue {} (after {})",
                idle.chain,
                crate::actions_request::short_id(&idle.after_run)
            )),
            (None, Some((chain, run))) => OrchestratorRow::Busy(format!(
                "{chain} is working on run {run}; this goal gets a new orchestrator"
            )),
            _ => OrchestratorRow::Choice("new".into()),
        }
    }

    /// The enabled models of the chosen runtime, in roster order; none with runtime
    /// `configured`, which only offers `default`.
    pub fn models(&self) -> Vec<String> {
        self.runtime
            .map(|runtime| models_of(&self.roster, runtime))
            .unwrap_or_default()
    }

    /// The picker's entries, as drawn: `default`, the models, then `custom…` (only with a
    /// runtime chosen).
    pub fn model_options(&self) -> Vec<String> {
        let models = self.models();
        let mut options = vec!["default".to_string()];
        if self.runtime.is_some() {
            options.extend(models);
            options.push("custom…".to_string());
        }
        options
    }

    /// The picker's current position in [`GoalForm::model_options`].
    pub fn model_at(&self) -> usize {
        match &self.model {
            GoalModel::Default => 0,
            GoalModel::Pick(i) => 1 + i,
            GoalModel::Custom => 1 + self.models().len(),
        }
    }

    /// A new roster (the settings cache changed while the form is open): a picked model
    /// stays picked by name, or falls back to `default` when it left the roster.
    pub fn set_roster(&mut self, roster: Vec<ModelEntry>) {
        let picked = match &self.model {
            GoalModel::Pick(i) => self.models().get(*i).cloned(),
            _ => None,
        };
        self.roster = roster;
        if let Some(name) = picked {
            self.model = match self.models().iter().position(|m| *m == name) {
                Some(i) => GoalModel::Pick(i),
                None => GoalModel::Default,
            };
        }
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
            if self.goal.on_editor_key(key, view.width, view.page()) == EditorKey::Unhandled {
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
            GoalField::Goal => {}
            GoalField::Runtime | GoalField::Model if locked => {}
            GoalField::Orchestrator if toggle && self.idle.is_some() => {
                self.continuing = !self.continuing;
            }
            GoalField::Orchestrator => {}
            GoalField::Runtime => match key.code {
                KeyCode::Right | KeyCode::Char(' ') => self.cycle_runtime(true),
                KeyCode::Left => self.cycle_runtime(false),
                _ => {}
            },
            GoalField::Model => self.on_model_key(key),
            GoalField::Delivery => match key.code {
                KeyCode::Right | KeyCode::Char(' ') => {
                    self.delivery = next_delivery(self.delivery, true)
                }
                KeyCode::Left => self.delivery = next_delivery(self.delivery, false),
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

    fn cycle_runtime(&mut self, forward: bool) {
        self.runtime = next_runtime(self.runtime, forward);
        // A model names one runtime's model.
        self.model = GoalModel::Default;
        self.custom = TextInput::default();
    }

    /// `←`/`→` always move the picker; `Space` does too, except while `custom…` is
    /// chosen, where it and every other character edit the text.
    fn on_model_key(&mut self, key: KeyEvent) {
        match (&self.model, key.code) {
            (_, KeyCode::Right) => self.cycle_model(true),
            (_, KeyCode::Left) => self.cycle_model(false),
            (GoalModel::Custom, _) => {
                let input = &mut self.custom;
                let full = input.text().chars().count() >= TEXT_MAX_CHARS;
                let typing = matches!(key.code, KeyCode::Char(_))
                    && !key.modifiers.contains(KeyModifiers::CONTROL);
                if !(full && typing) {
                    apply_text_key(input, key);
                }
            }
            (_, KeyCode::Char(' ')) => self.cycle_model(true),
            _ => {}
        }
    }

    fn cycle_model(&mut self, forward: bool) {
        let len = self.model_options().len();
        let at = self.model_at();
        let to = if forward {
            (at + 1) % len
        } else {
            (at + len - 1) % len
        };
        self.model = match to {
            0 => GoalModel::Default,
            n if n == len - 1 => GoalModel::Custom,
            n => GoalModel::Pick(n - 1),
        };
    }

    /// [`GoalForm::on_paste_in`] with the goal unwrapped.
    pub fn on_paste(&mut self, text: &str) {
        self.on_paste_in(text, EditorView::default());
    }

    /// A bracketed paste into the goal (the editor's, decision 5) or the custom model:
    /// control characters go; the field stays bounded; the goal keeps its line breaks.
    /// The goal is drawn as `view` says (the editor's viewport follows it).
    pub fn on_paste_in(&mut self, text: &str, view: EditorView) {
        if self.submitting || self.discarding {
            return;
        }
        // `view` reaches the editor's paste with task 9a's viewport (its fix round).
        let _ = view;
        match (self.focus, &self.model) {
            (GoalField::Goal, _) => self.goal.on_editor_paste(text),
            (GoalField::Model, GoalModel::Custom) if self.custom_shown() => {
                insert_bounded(&mut self.custom, &clean_line(text));
            }
            _ => {}
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

    /// The model the request names: none for `default` (or an empty custom text).
    pub fn chosen_model(&self) -> Option<String> {
        match &self.model {
            GoalModel::Default => None,
            GoalModel::Pick(i) => self.models().get(*i).cloned(),
            GoalModel::Custom => {
                let text = self.custom.text().trim();
                (!text.is_empty()).then(|| text.to_string())
            }
        }
    }

    /// Decision 44's request: what `anthrex run start --goal` sends, with the
    /// orchestrator choice when a runtime is chosen, and decision 39's toggles; while
    /// continuing (decision 25), `continue_from` the chain's last run and no
    /// orchestrator choice, as `run start --goal … --continue` sends it.
    pub fn request(&self) -> Result<RunRequest, (GoalField, &'static str)> {
        let goal = self.goal.text();
        if goal.trim().is_empty() {
            return Err((GoalField::Goal, EMPTY_GOAL));
        }
        let continue_from = self.continues().map(|idle| idle.after_run.clone());
        let orchestrator = self
            .runtime
            .filter(|_| continue_from.is_none())
            .map(|runtime| OrchestratorChoice {
                runtime,
                model: self.chosen_model(),
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
        })
    }
}

#[cfg(test)]
#[path = "run_goal_tests.rs"]
mod tests;
