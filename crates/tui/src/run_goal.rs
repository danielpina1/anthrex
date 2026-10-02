//! Milestone 9 decision 44 and (9.0.6) decision 39: the goal form `C-b g` opens, pure
//! (`AGENTS.md` hard rule 5). It holds a goal (a text area; `Ctrl-J` inserts a newline),
//! an optional orchestrator runtime, a model picked from that runtime's enabled models
//! (or typed after `custom…`), and three toggles that start off: `trust`,
//! `approve at once` and `unconfined checks`, for one project the caller chose. `Enter`
//! builds M8b's `RunRequest::StartGoal` exactly as `anthrex run start --goal` sends it:
//! with the toggles off the plan gate stays on and checks stay confined. Opening,
//! sending and the replies are `app/goal.rs`; rendering is `ui/run_goal.rs`.

use crate::app::screens::models_of;
use crate::dialog::{TextInput, apply_text_key};
use crate::run_edit::TEXT_MAX_CHARS;
use crate::text_area::TextArea;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use proto::{ModelEntry, OrchestratorChoice, RunRequest, Runtime};
use std::path::PathBuf;

/// The toast `C-b g` shows when no project is selected, no window is focused and the
/// TUI's start directory is empty (milestone 9.0.7 decision 37's fallback).
pub const NO_PROJECT: &str = "select a Git project to start a goal";
/// The inline error of an `Enter` with a blank goal.
pub const EMPTY_GOAL: &str = "type a goal first";
/// The inline error after the connection refused the request or the link was lost.
pub const NOT_SENT: &str = "the goal was not sent; press Enter to retry";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GoalField {
    Goal,
    Runtime,
    Model,
    Trust,
    Yes,
    UnconfinedChecks,
}

const FIELDS: [GoalField; 6] = [
    GoalField::Goal,
    GoalField::Runtime,
    GoalField::Model,
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
}

#[derive(Debug, Clone, PartialEq)]
pub enum GoalOutcome {
    Stay,
    Cancel,
    Submit(RunRequest),
}

pub fn field_label(field: GoalField) -> &'static str {
    match field {
        GoalField::Goal => "goal",
        GoalField::Runtime => "runtime",
        GoalField::Model => "model",
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
            goal: TextArea::new(),
            runtime: None,
            model: GoalModel::Default,
            custom: TextInput::default(),
            roster: Vec::new(),
            trust_project: false,
            yes: false,
            unconfined_checks: false,
            focus: GoalField::Goal,
            error: None,
            submitting: false,
            request_id: None,
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
        self.on_key_in(key, 0)
    }

    /// Decision 44's keys. While submitting, only `Esc` and `Ctrl-C` act, as in M8c's
    /// edit form, so a second `Enter` sends nothing. In the goal, Up and Down move a
    /// row of its text area drawn `goal_width` wide (`ui::run_goal::goal_width`), and
    /// the focus only from its first or last row (milestone 9.0.7 decision 35).
    pub fn on_key_in(&mut self, key: KeyEvent, goal_width: u16) -> GoalOutcome {
        if key.code == KeyCode::Esc || is_ctrl(&key, 'c') {
            return GoalOutcome::Cancel;
        }
        if self.submitting {
            return GoalOutcome::Stay;
        }
        let in_goal = self.focus == GoalField::Goal;
        match key.code {
            KeyCode::Up | KeyCode::Down if in_goal && self.goal.on_key_in(key, goal_width) => {}
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
        match self.focus {
            GoalField::Goal => {
                self.goal.on_key(key);
            }
            GoalField::Runtime => match key.code {
                KeyCode::Right | KeyCode::Char(' ') => self.cycle_runtime(true),
                KeyCode::Left => self.cycle_runtime(false),
                _ => {}
            },
            GoalField::Model => self.on_model_key(key),
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

    /// A paste into the goal or the custom model: control characters go; the field stays
    /// bounded; the goal keeps its line breaks.
    pub fn on_paste(&mut self, text: &str) {
        if self.submitting {
            return;
        }
        match (self.focus, &self.model) {
            (GoalField::Goal, _) => self.goal.on_paste(text),
            (GoalField::Model, GoalModel::Custom) => {
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
    /// orchestrator choice when a runtime is chosen, and decision 39's toggles.
    pub fn request(&self) -> Result<RunRequest, (GoalField, &'static str)> {
        let goal = self.goal.text();
        if goal.trim().is_empty() {
            return Err((GoalField::Goal, EMPTY_GOAL));
        }
        let orchestrator = self.runtime.map(|runtime| OrchestratorChoice {
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
            delivery: None,
        })
    }
}

#[cfg(test)]
#[path = "run_goal_tests.rs"]
mod tests;
