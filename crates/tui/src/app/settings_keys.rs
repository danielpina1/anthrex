//! The Settings screen's keys (decision 36), split from `settings_screen.rs` by
//! responsibility (`AGENTS.md` hard rule 8). A page's keys come first, then the
//! `models` section's picker (milestone 9.8: it has every key while open), then the
//! screen's (`esc`, `tab`), then the section's: `models_keys.rs` for the role table, the
//! limits' digits and `w` here. Pure: every request leaves as an `Effect`.

use super::{SettingsPage, SettingsScreen, SettingsSection};
use crate::app::models_keys::{ModelsIntent, models_key};
use crate::app::{App, Effect, ToastLevel};
use crossterm::event::{KeyCode, KeyEvent};
use proto::ClientMsg;

/// The most digits a limit takes.
const DIGITS_MAX: usize = 9;

/// What a key asks of the app beyond the screen.
enum Intent {
    Stay,
    Save,
    Close,
    Warn(&'static str),
    Models(ModelsIntent),
}

impl App {
    /// A bare key while the Settings screen is open.
    pub(crate) fn on_settings_key(&mut self, key: KeyEvent) -> Vec<Effect> {
        let catalogs = self.catalogs.clone();
        let Some(s) = self.settings_screen_mut() else {
            return vec![];
        };
        let intent = if s.page.is_some() {
            page_key(s, key)
        } else {
            screen_key(s, &catalogs, key)
        };
        match intent {
            Intent::Stay | Intent::Models(ModelsIntent::Stay) => vec![],
            Intent::Save | Intent::Models(ModelsIntent::Save) => self.settings_save(),
            Intent::Models(ModelsIntent::SaveRepo) => self.settings_save_repo(),
            Intent::Models(ModelsIntent::AskRepo) => self.settings_repo_ask(),
            Intent::Models(ModelsIntent::Refresh) => vec![Effect::Send(ClientMsg::ListModels {
                runtime: None,
                refresh: true,
            })],
            Intent::Models(ModelsIntent::BrainstormEffort) => {
                let effort = (self.settings_screen_mut())
                    .and_then(|s| s.models.brainstorm().effort)
                    .unwrap_or_else(|| "default".to_string());
                self.toast(format!("brainstorm effort: {effort}"));
                vec![]
            }
            Intent::Close => {
                self.set_screen(None);
                vec![]
            }
            Intent::Warn(text) => {
                self.toast_at(ToastLevel::Warn, text);
                vec![]
            }
        }
    }

    /// A paste goes to the picker's custom model name, as one line, or nowhere.
    pub(crate) fn on_settings_paste(&mut self, text: &str) {
        if let Some(s) = self.settings_screen_mut()
            && let Some(picker) = &mut s.models.picker
        {
            picker.on_paste(text);
        }
    }
}

fn screen_key(
    s: &mut SettingsScreen,
    catalogs: &crate::app::model_picker::Catalogs,
    key: KeyEvent,
) -> Intent {
    if !s.loaded {
        return match key.code {
            KeyCode::Esc => Intent::Close,
            _ => Intent::Stay,
        };
    }
    let models = s.section == SettingsSection::Models;
    if models && s.models.picker.is_some() {
        return Intent::Models(models_key(s, catalogs, key));
    }
    match key.code {
        KeyCode::Esc if s.dirty() => s.page = Some(SettingsPage::Discard),
        KeyCode::Esc => return Intent::Close,
        KeyCode::Tab | KeyCode::BackTab => {
            let all = SettingsSection::ALL;
            let at = all.iter().position(|x| *x == s.section).unwrap_or(0);
            let step = if key.code == KeyCode::Tab {
                1
            } else {
                all.len() - 1
            };
            s.section = all[(at + step) % all.len()];
            s.selected = 0;
        }
        _ if models => return Intent::Models(models_key(s, catalogs, key)),
        KeyCode::Char('w') => return Intent::Save,
        KeyCode::Char('j') | KeyCode::Down => s.selected = (s.selected + 1).min(s.last_row()),
        KeyCode::Char('k') | KeyCode::Up => s.selected = s.selected.saturating_sub(1),
        _ => limit_key(s, key),
    }
    Intent::Stay
}

/// A limit: digits append, `Backspace` removes the last.
fn limit_key(s: &mut SettingsScreen, key: KeyEvent) {
    let Some(field) = s.limits.get_mut(s.selected) else {
        return;
    };
    match key.code {
        KeyCode::Char(c @ '0'..='9') if field.text.len() < DIGITS_MAX => field.text.push(c),
        KeyCode::Backspace => {
            field.text.pop();
        }
        _ => return,
    }
    s.touched();
}

fn page_key(s: &mut SettingsScreen, key: KeyEvent) -> Intent {
    match &mut s.page {
        Some(SettingsPage::Discard) => match key.code {
            KeyCode::Char('y') => return Intent::Close,
            // The milestone-wide ruling: a discard confirms only on `y`.
            KeyCode::Enter => return Intent::Warn("press y to discard"),
            KeyCode::Esc => s.page = None,
            _ => {}
        },
        None => {}
    }
    Intent::Stay
}
