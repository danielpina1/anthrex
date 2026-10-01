//! Milestone 9 decision 44: the goal form `C-b g` opens, pure (`AGENTS.md` hard rule
//! 5). It holds a goal (`Ctrl-J` inserts a newline), an optional orchestrator runtime
//! and model, and a `trust_project` toggle that starts off, for one project the caller
//! chose. `Enter` builds M8b's `RunRequest::StartGoal` exactly as `anthrex run start
//! --goal` sends it: `yes` and `unconfined_checks` are false, so the plan gate stays on.
//! Opening, sending and the replies are `app/goal.rs`; rendering is `ui/run_goal.rs`.

use crate::dialog::{TextInput, apply_text_key};
use crate::run_edit::{NEWLINE_MARK, TEXT_MAX_CHARS};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use proto::{OrchestratorChoice, RunRequest, Runtime};
use std::path::PathBuf;

/// The toast `C-b g` shows when no project is selected and no window is focused.
pub const NO_PROJECT: &str = "select a Git project to start a goal";
/// The inline error of an `Enter` with a blank goal.
pub const EMPTY_GOAL: &str = "type a goal first";
/// The inline error of a model typed with the default runtime.
pub const MODEL_NEEDS_RUNTIME: &str = "choose claude or codex for a model";
/// The inline error after the connection refused the request or the link was lost.
pub const NOT_SENT: &str = "the goal was not sent; press Enter to retry";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GoalField {
    Goal,
    Runtime,
    Model,
    Trust,
}

const FIELDS: [GoalField; 4] = [
    GoalField::Goal,
    GoalField::Runtime,
    GoalField::Model,
    GoalField::Trust,
];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GoalForm {
    /// The project the goal runs in, chosen when the form opened; never changed.
    pub project: PathBuf,
    /// Newlines are drawn as [`NEWLINE_MARK`] and sent as `\n`.
    pub goal: TextInput,
    /// `None` is the configured orchestrator (`[orchestrator.agent]`, then the default).
    pub runtime: Option<Runtime>,
    pub model: TextInput,
    pub trust_project: bool,
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
    }
}

/// Newlines become `newline` (or go, when `None`), a tab a space, and every other
/// control character is dropped.
fn clean(text: &str, newline: Option<char>) -> String {
    let text = text.replace("\r\n", "\n").replace('\r', "\n");
    text.chars()
        .filter_map(|c| match c {
            '\n' => newline,
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
            goal: TextInput::default(),
            runtime: None,
            model: TextInput::default(),
            trust_project: false,
            focus: GoalField::Goal,
            error: None,
            submitting: false,
            request_id: None,
        }
    }

    fn move_focus(&mut self, delta: isize) {
        let at = FIELDS.iter().position(|f| *f == self.focus).unwrap_or(0) as isize;
        let next = (at + delta).rem_euclid(FIELDS.len() as isize);
        self.focus = FIELDS[next as usize];
    }

    /// Decision 44's keys. While submitting, only `Esc` and `Ctrl-C` act, as in M8c's
    /// edit form, so a second `Enter` sends nothing.
    pub fn on_key(&mut self, key: KeyEvent) -> GoalOutcome {
        if key.code == KeyCode::Esc || is_ctrl(&key, 'c') {
            return GoalOutcome::Cancel;
        }
        if self.submitting {
            return GoalOutcome::Stay;
        }
        match key.code {
            KeyCode::Tab | KeyCode::Down => self.move_focus(1),
            KeyCode::BackTab | KeyCode::Up => self.move_focus(-1),
            KeyCode::Enter => return self.submit(),
            _ => match self.focus {
                GoalField::Goal | GoalField::Model => self.on_text_key(key),
                GoalField::Runtime => match key.code {
                    KeyCode::Right | KeyCode::Char(' ') => self.cycle_runtime(true),
                    KeyCode::Left => self.cycle_runtime(false),
                    _ => {}
                },
                GoalField::Trust => {
                    if matches!(
                        key.code,
                        KeyCode::Char(' ') | KeyCode::Left | KeyCode::Right
                    ) {
                        self.trust_project = !self.trust_project;
                    }
                }
            },
        }
        GoalOutcome::Stay
    }

    fn cycle_runtime(&mut self, forward: bool) {
        self.runtime = next_runtime(self.runtime, forward);
        // A model names one runtime's model.
        self.model.clear();
    }

    fn on_text_key(&mut self, key: KeyEvent) {
        let goal = self.focus == GoalField::Goal;
        let input = if goal {
            &mut self.goal
        } else {
            &mut self.model
        };
        if is_ctrl(&key, 'j') {
            if goal {
                insert_bounded(input, &NEWLINE_MARK.to_string());
            }
            return;
        }
        let full = input.text().chars().count() >= TEXT_MAX_CHARS;
        let typing =
            matches!(key.code, KeyCode::Char(_)) && !key.modifiers.contains(KeyModifiers::CONTROL);
        if !(full && typing) {
            apply_text_key(input, key);
        }
    }

    /// A paste into the goal or the model: newlines become `↵` in the goal and go
    /// elsewhere; control characters go; the field stays bounded.
    pub fn on_paste(&mut self, text: &str) {
        if self.submitting {
            return;
        }
        match self.focus {
            GoalField::Goal => insert_bounded(&mut self.goal, &clean(text, Some(NEWLINE_MARK))),
            GoalField::Model => insert_bounded(&mut self.model, &clean(text, None)),
            GoalField::Runtime | GoalField::Trust => {}
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
    /// orchestrator choice when a runtime is chosen.
    pub fn request(&self) -> Result<RunRequest, (GoalField, &'static str)> {
        let goal = self.goal.text().replace(NEWLINE_MARK, "\n");
        if goal.trim().is_empty() {
            return Err((GoalField::Goal, EMPTY_GOAL));
        }
        let model = self.model.text().trim();
        let orchestrator = match self.runtime {
            Some(runtime) => Some(OrchestratorChoice {
                runtime,
                model: (!model.is_empty()).then(|| model.to_string()),
            }),
            None if !model.is_empty() => return Err((GoalField::Runtime, MODEL_NEEDS_RUNTIME)),
            None => None,
        };
        Ok(RunRequest::StartGoal {
            goal: goal.trim().to_string(),
            dir: self.project.clone(),
            yes: false,
            trust_project: self.trust_project,
            unconfined_checks: false,
            orchestrator,
            delivery: None,
        })
    }

    /// A field's value as drawn.
    pub fn value_text(&self, field: GoalField) -> String {
        match field {
            GoalField::Goal => self.goal.text().to_string(),
            GoalField::Runtime => match self.runtime {
                Some(runtime) => format!("‹ {} ›", runtime.label()),
                None => "‹ configured ›".to_string(),
            },
            GoalField::Model if self.model.text().is_empty() => "default".to_string(),
            GoalField::Model => self.model.text().to_string(),
            GoalField::Trust => {
                let mark = if self.trust_project { "x" } else { " " };
                format!("[{mark}] trust the project's own agent settings")
            }
        }
    }
}

#[cfg(test)]
#[path = "run_goal_tests.rs"]
mod tests;
