//! Key handling for every `Modal` variant, split out of `app.rs` per the 600-line rule
//! (`AGENTS.md` hard rule 8) — the same shape `tree.rs` gives `tree/rows.rs`.
//!
//! `App::on_key` takes the modal out with `self.modal.take()` before calling in here
//! (decision: the previous `self.modal.clone()` dropped in-place edits, which a text
//! field the user is typing into cannot afford), so every arm below is responsible for
//! putting a modal back — or not, when the key closes it.
//!
//! `Modal::Remove` and `Modal::ForceRemove` are driven by task M5.10 (decisions 35 and
//! 36); the arms here only keep the match exhaustive and leave those modals exactly as
//! they were, since nothing in this task ever opens them.

use crate::app::{App, Effect, Modal};
use crate::dialog::{FormContext, FormDefaults, FormOutcome, NewAgentForm};
use crossterm::event::{KeyCode, KeyEvent};
use proto::ClientMsg;

impl App {
    /// What the next new-agent form should open with (decision 31). Once a form has
    /// been accepted this session, `form_defaults` holds its raw values verbatim. Until
    /// then `form_defaults.dir` is still the empty sentinel `App::new` set it to, so the
    /// directory shown is computed fresh from `default_dir` and `home_dir` instead —
    /// freshly, rather than once at startup, because `home_dir` is not known yet when
    /// `App::new` runs (`tui::run` sets it only after construction).
    pub(super) fn new_agent_defaults(&self) -> FormDefaults {
        if self.form_defaults.dir.is_empty() {
            FormDefaults {
                dir: crate::ui::terminal::shorten_home_with(
                    &self.default_dir,
                    self.home_dir.as_deref(),
                ),
                ..self.form_defaults.clone()
            }
        } else {
            self.form_defaults.clone()
        }
    }

    pub(crate) fn on_modal_key(&mut self, key: KeyEvent) -> Vec<Effect> {
        let Some(modal) = self.modal.take() else {
            return vec![];
        };
        match modal {
            // Any key closes the help overlay; nothing to restore.
            Modal::Help => vec![],
            Modal::Confirm { message, action } => match key.code {
                KeyCode::Char('y') | KeyCode::Char('Y') | KeyCode::Enter => self.perform(action),
                KeyCode::Char('n') | KeyCode::Char('N') | KeyCode::Esc => vec![],
                _ => {
                    self.modal = Some(Modal::Confirm { message, action });
                    vec![]
                }
            },
            Modal::NewAgent(form) => self.on_new_agent_key(form, key),
            Modal::Remove(remove) => {
                self.modal = Some(Modal::Remove(remove));
                vec![]
            }
            Modal::ForceRemove {
                window_id,
                name,
                message,
            } => {
                self.modal = Some(Modal::ForceRemove {
                    window_id,
                    name,
                    message,
                });
                vec![]
            }
        }
    }

    fn on_new_agent_key(&mut self, mut form: NewAgentForm, key: KeyEvent) -> Vec<Effect> {
        let existing_names: Vec<&str> = self.windows.iter().map(|w| w.name.as_str()).collect();
        let ctx = FormContext {
            default_dir: &self.default_dir,
            home: self.home_dir.as_deref(),
            existing_names: &existing_names,
        };
        match form.on_key(key, &ctx) {
            FormOutcome::Cancel => vec![],
            FormOutcome::Stay => {
                self.modal = Some(Modal::NewAgent(form));
                vec![]
            }
            FormOutcome::Submit(spec) => {
                let (cols, rows) = self.term_size;
                self.modal = Some(Modal::NewAgent(form));
                vec![Effect::Send(ClientMsg::CreateWindow {
                    spec,
                    cols: cols.max(1),
                    rows: rows.max(1),
                })]
            }
        }
    }
}
