//! `ActionForm`'s methods (milestone 9.0.6 decision 15), moved from `action_forms.rs`
//! (milestone 9.15 decision 40): a form's keys, paste, input and confirmation rows.

use super::{ActionForm, FormOutcome, cycle_kind, is_ctrl_j, kind_word, sent};
use crate::actions_request::ActionInput;
use crate::safe_text::one_line;
use crate::text_area::TextArea;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use proto::ActionInfo;

impl ActionForm {
    pub fn info(&self) -> &ActionInfo {
        match self {
            ActionForm::Answer(f) => &f.info,
            ActionForm::Message(f) => &f.info,
            ActionForm::Override(f) => &f.info,
            ActionForm::Resume(f) => &f.info,
            ActionForm::Promote(f) => &f.info,
        }
    }

    pub fn set_info(&mut self, info: ActionInfo) {
        match self {
            ActionForm::Answer(f) => f.info = info,
            ActionForm::Message(f) => f.info = info,
            ActionForm::Override(f) => f.info = info,
            ActionForm::Resume(f) => f.info = info,
            ActionForm::Promote(f) => f.info = info,
        }
    }

    #[cfg(test)]
    pub fn on_key(&mut self, key: KeyEvent) -> FormOutcome {
        self.on_key_in(key, 0)
    }

    /// The form's own keys; Enter, Esc and what the form does not use are decided here.
    /// Up and Down move a row of its text area drawn `width` wide (decision 35).
    pub fn on_key_in(&mut self, key: KeyEvent, width: u16) -> FormOutcome {
        match key.code {
            KeyCode::Esc => return FormOutcome::Back,
            KeyCode::Enter => {
                // A blank text says why on the form (final review I2), as a blank
                // reason always did.
                let ready = self.input().is_ok();
                match self {
                    ActionForm::Answer(f) => f.blank = !ready,
                    ActionForm::Message(f) => f.blank = !ready,
                    ActionForm::Override(f) => f.blank = !ready,
                    ActionForm::Resume(_) | ActionForm::Promote(_) => {}
                }
                return if ready {
                    FormOutcome::Page
                } else {
                    FormOutcome::Stay
                };
            }
            _ => {}
        }
        let back = key.code == KeyCode::BackTab
            || (key.code == KeyCode::Tab && key.modifiers.contains(KeyModifiers::SHIFT));
        let forward = key.code == KeyCode::Tab && !back;
        match self {
            ActionForm::Answer(f) => {
                if f.text.on_key_in(key, width) {
                    f.blank = false;
                }
            }
            ActionForm::Message(f) => {
                if forward || back {
                    f.kind = cycle_kind(f.kind, back);
                } else if f.text.on_key_in(key, width) {
                    f.blank = false;
                }
            }
            ActionForm::Override(f) => {
                // One line: a newline has no place in a reason.
                if !is_ctrl_j(&key) && f.reason.on_key_in(key, width) {
                    f.blank = false;
                }
            }
            ActionForm::Resume(f) => {
                if matches!(
                    key.code,
                    KeyCode::Tab
                        | KeyCode::BackTab
                        | KeyCode::Left
                        | KeyCode::Right
                        | KeyCode::Char(' ')
                ) {
                    f.rebaseline = !f.rebaseline;
                }
            }
            ActionForm::Promote(f) => {
                let n = f.options.len().max(1);
                match key.code {
                    KeyCode::Right | KeyCode::Tab if !back => f.at = (f.at + 1) % n,
                    KeyCode::Left | KeyCode::BackTab | KeyCode::Tab => f.at = (f.at + n - 1) % n,
                    _ => {}
                }
            }
        }
        FormOutcome::Stay
    }

    pub fn on_paste(&mut self, text: &str) {
        match self {
            ActionForm::Answer(f) => {
                f.text.on_paste(text);
                f.blank = false;
            }
            ActionForm::Message(f) => {
                f.text.on_paste(text);
                f.blank = false;
            }
            ActionForm::Override(f) => {
                // A reason is one line: a paste's breaks become spaces.
                f.reason.on_paste(&text.replace(['\r', '\n'], " "));
                f.blank = false;
            }
            ActionForm::Resume(_) | ActionForm::Promote(_) => {}
        }
    }

    /// What the request is built from, or why the form is not ready.
    pub fn input(&self) -> Result<ActionInput, &'static str> {
        match self {
            ActionForm::Answer(f) => match sent(&f.text) {
                t if t.is_empty() => Err("type an answer first"),
                text => Ok(ActionInput::Answer(text)),
            },
            ActionForm::Message(f) => match sent(&f.text) {
                t if t.is_empty() => Err("type a message first"),
                text => Ok(ActionInput::Message { kind: f.kind, text }),
            },
            ActionForm::Override(f) => match sent(&f.reason) {
                t if t.is_empty() => Err("type a reason first"),
                reason => Ok(ActionInput::Reason(reason)),
            },
            ActionForm::Resume(f) => Ok(ActionInput::Resume {
                rebaseline: f.rebaseline,
            }),
            ActionForm::Promote(f) => Ok(ActionInput::Promote(
                f.options.get(f.at).and_then(|o| o.choice.clone()),
            )),
        }
    }

    /// The confirmation page's rows for what was typed or picked.
    pub fn details(&self, ascii: bool) -> Vec<(String, String)> {
        let ellipsis = if ascii { "..." } else { "…" };
        let preview =
            |area: &TextArea| crate::ui::kit::cut(&one_line(area.text().trim()), 160, ellipsis);
        let row = |l: &str, v: String| (l.to_string(), v);
        match self {
            ActionForm::Answer(f) => {
                vec![row("task", f.task.clone()), row("answer", preview(&f.text))]
            }
            ActionForm::Message(f) => vec![
                row("to", f.to.clone()),
                row("kind", kind_word(f.kind).to_string()),
                row("message", preview(&f.text)),
            ],
            ActionForm::Override(f) => vec![row("reason", preview(&f.reason))],
            ActionForm::Resume(f) => vec![row(
                "rebaseline",
                if f.rebaseline { "on" } else { "off" }.to_string(),
            )],
            ActionForm::Promote(f) => vec![row(
                "orchestrator",
                f.options
                    .get(f.at)
                    .map(|o| o.label.clone())
                    .unwrap_or_default(),
            )],
        }
    }
}
