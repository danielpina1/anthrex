//! The Settings screen's keys (decision 36), split from `settings_screen.rs` by
//! responsibility (`AGENTS.md` hard rule 8). A page's keys come first, then the screen's
//! (`esc`, `tab`, `w`), then the section's: a model table's `space`, `←`/`→` and `⏎`, the
//! orchestrator's choices, the limits' digits. Pure: a save leaves as an `Effect`.

use super::{CustomModel, SettingsPage, SettingsScreen, SettingsSection, next_strength};
use crate::app::{App, Effect, ToastLevel};
use crate::safe_text::one_line;
use crate::text_area::TextArea;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use proto::{Runtime, Strength};

/// The most digits a limit takes.
const DIGITS_MAX: usize = 9;

/// The custom dialog's refusal of an empty name.
pub const NAME_FIRST: &str = "type a model name first";

/// What a key asks of the app beyond the screen.
enum Intent {
    Stay,
    Save,
    Close,
    Warn(&'static str),
}

impl App {
    /// A bare key while the Settings screen is open.
    pub(crate) fn on_settings_key(&mut self, key: KeyEvent) -> Vec<Effect> {
        let Some(s) = self.settings_screen_mut() else {
            return vec![];
        };
        let intent = if s.page.is_some() {
            page_key(s, key)
        } else {
            screen_key(s, key)
        };
        match intent {
            Intent::Stay => vec![],
            Intent::Save => self.settings_save(),
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

    /// A paste goes to the custom dialog's model name, as one line, or nowhere.
    pub(crate) fn on_settings_paste(&mut self, text: &str) {
        if let Some(s) = self.settings_screen_mut()
            && let Some(SettingsPage::Custom(c)) = &mut s.page
            && !c.on_strength
        {
            c.model.on_paste(&text.replace(['\r', '\n'], " "));
            c.error = None;
        }
    }
}

fn screen_key(s: &mut SettingsScreen, key: KeyEvent) -> Intent {
    if !s.loaded {
        return match key.code {
            KeyCode::Esc => Intent::Close,
            _ => Intent::Stay,
        };
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
        KeyCode::Char('w') => return Intent::Save,
        KeyCode::Char('j') | KeyCode::Down => s.selected = (s.selected + 1).min(s.last_row()),
        KeyCode::Char('k') | KeyCode::Up => s.selected = s.selected.saturating_sub(1),
        _ => match s.section.runtime() {
            Some(runtime) => model_key(s, runtime, key),
            None if s.section == SettingsSection::Orchestrator => orchestrator_key(s, key),
            None => limit_key(s, key),
        },
    }
    Intent::Stay
}

/// A model table: `space` toggles a row, `←`/`→` change a custom row's strength, `⏎` on
/// `custom…` opens the dialog.
fn model_key(s: &mut SettingsScreen, runtime: Runtime, key: KeyEvent) {
    let at = s.selected;
    let on_custom = at == s.rows(runtime).len();
    match key.code {
        KeyCode::Char(' ') if !on_custom => {
            let row = &mut s.rows_mut(runtime)[at];
            row.enabled = !row.enabled;
            s.touched();
        }
        KeyCode::Left | KeyCode::Right if !on_custom && s.rows(runtime)[at].custom => {
            let row = &mut s.rows_mut(runtime)[at];
            row.entry.strength = next_strength(row.entry.strength, key.code == KeyCode::Right);
            s.touched();
        }
        KeyCode::Enter if on_custom => {
            s.page = Some(SettingsPage::Custom(CustomModel {
                runtime,
                model: TextArea::new(),
                strength: Strength::Standard,
                on_strength: false,
                error: None,
            }));
        }
        _ => {}
    }
}

/// `runtime ‹ configured ›` and `model ‹ default ›`: `←`, `→` or `space` change the
/// selected one; a new runtime resets the model to `default`.
fn orchestrator_key(s: &mut SettingsScreen, key: KeyEvent) {
    let forward = match key.code {
        KeyCode::Right | KeyCode::Char(' ') => true,
        KeyCode::Left => false,
        _ => return,
    };
    if s.selected == 0 {
        const RUNTIMES: [Option<Runtime>; 3] = [None, Some(Runtime::Claude), Some(Runtime::Codex)];
        s.runtime = step(&RUNTIMES, &s.runtime, forward);
        s.model = String::new();
    } else {
        let options = s.model_options();
        s.model = step(&options, &s.model, forward);
    }
    s.touched();
}

/// The option after (or before) `now` in `options`, wrapping; a value not among them
/// goes to the first (or the last).
fn step<T: Clone + PartialEq>(options: &[T], now: &T, forward: bool) -> T {
    let n = options.len();
    let next = match (options.iter().position(|o| o == now), forward) {
        (Some(i), true) => (i + 1) % n,
        (Some(i), false) => (i + n - 1) % n,
        (None, true) => 0,
        (None, false) => n - 1,
    };
    options[next].clone()
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
        Some(SettingsPage::Custom(c)) => match key.code {
            KeyCode::Esc => s.page = None,
            KeyCode::Tab | KeyCode::BackTab => c.on_strength = !c.on_strength,
            KeyCode::Enter => add_custom(s),
            KeyCode::Left | KeyCode::Right | KeyCode::Char(' ') if c.on_strength => {
                c.strength = next_strength(c.strength, key.code != KeyCode::Left);
            }
            _ if c.on_strength => {}
            // One line: Ctrl-J is the text area's newline, not this field's.
            KeyCode::Char('j') if key.modifiers.contains(KeyModifiers::CONTROL) => {}
            _ => {
                if c.model.on_key(key) {
                    c.error = None;
                }
            }
        },
        None => {}
    }
    Intent::Stay
}

/// The dialog's `⏎`: the named model enabled in its table with its strength, as a new
/// custom row unless the table already has a row of that name.
fn add_custom(s: &mut SettingsScreen) {
    let Some(SettingsPage::Custom(c)) = &mut s.page else {
        return;
    };
    let name = one_line(c.model.text()).trim().to_string();
    if name.is_empty() {
        c.error = Some(NAME_FIRST.into());
        return;
    }
    let (runtime, strength) = (c.runtime, c.strength);
    let rows = s.rows_mut(runtime);
    let at = match rows.iter().position(|r| r.entry.model == name) {
        Some(at) => {
            let row = &mut rows[at];
            row.enabled = true;
            if row.custom {
                row.entry.strength = strength;
            }
            at
        }
        None => {
            rows.push(super::ModelRow {
                entry: proto::ModelEntry {
                    runtime,
                    model: name.clone(),
                    strength,
                    note: String::new(),
                },
                label: name,
                enabled: true,
                custom: true,
            });
            rows.len() - 1
        }
    };
    s.page = None;
    s.selected = at;
    s.touched();
}
