//! The Profile screen's pages (milestone 9.0.6 decision 35, milestone 9.10 decision 30):
//! the Detect toggles, the Discard and Unset pages, the raw-text and row pages, and the
//! editor fitted to a key's type, with their keys and pastes. Every edit sends `yes:
//! true` (decision 16), on the proposal while the card shows. Split from
//! `app/profile_screen.rs` by responsibility (`AGENTS.md` hard rule 8). Pure.

use super::{Editor, EditorField, ProfileAsk, ProfilePage, Saving};
use crate::app::{App, Effect, ToastLevel};
use crate::profile_view::{self, Kind};
use crate::safe_text::one_line;
use crate::text_area::TextArea;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use proto::ProfileRequest;

/// The editor for `key`, starting from `text` (sanitised: one line, or one per line).
pub(super) fn editor_for(key: &str, text: &str) -> Editor {
    let field = match profile_view::kind_of(key) {
        Kind::Text => EditorField::Line(TextArea::from_text(&one_line(text))),
        Kind::List => {
            let items: Vec<String> = text.lines().map(one_line).collect();
            EditorField::List(TextArea::from_text(&items.join("\n")))
        }
        Kind::Number => EditorField::Digits(text.chars().filter(char::is_ascii_digit).collect()),
        Kind::Choice(options) => EditorField::Choice {
            options,
            at: options.iter().position(|o| *o == text).unwrap_or(0),
        },
        Kind::Env => EditorField::Env {
            name: TextArea::from_text(&one_line(key.strip_prefix("env.").unwrap_or(""))),
            value: TextArea::from_text(&one_line(text)),
            on_value: false,
        },
    };
    Editor {
        key: key.to_string(),
        field,
        error: None,
    }
}

/// A one-line text area's key: its newline (Ctrl-J) is not taken.
fn line_key(area: &mut TextArea, key: KeyEvent, width: u16) {
    let ctrl_j = key.modifiers.contains(KeyModifiers::CONTROL)
        && matches!(key.code, KeyCode::Char('j' | 'J'));
    if !ctrl_j {
        area.on_key_in(key, width);
    }
}

/// The editor's request: `(key, value)`, or why not.
fn edit_of(editor: &Editor) -> Result<(String, String), String> {
    let key = editor.key.clone();
    match &editor.field {
        EditorField::Line(area) | EditorField::List(area) => {
            Ok((key.clone(), profile_view::value_literal(&key, area.text())?))
        }
        EditorField::Digits(text) => Ok((key.clone(), profile_view::value_literal(&key, text)?)),
        EditorField::Choice { options, at } => {
            let value = options.get(*at).copied().unwrap_or_default();
            Ok((key.clone(), profile_view::value_literal(&key, value)?))
        }
        EditorField::Env { name, value, .. } => {
            let name = name.text().trim();
            if name.is_empty() {
                return Err("type a name first".to_string());
            }
            Ok((format!("env.{name}"), value.text().to_string()))
        }
    }
}

impl App {
    pub(super) fn on_profile_page_key(&mut self, key: KeyEvent) -> Vec<Effect> {
        // The editor's text areas as the page draws them over the whole terminal.
        let width = crate::ui::profile::list_width(self.body_area.width);
        let Some(s) = self.profile_screen_mut() else {
            return vec![];
        };
        if key.code == KeyCode::Esc {
            s.page = None;
            return vec![];
        }
        let dir = s.dir.clone();
        let on_proposal = s.showing_card();
        // The lines a scrolled page can scroll through.
        let tail_lines = match &s.page {
            Some(ProfilePage::Row { key, .. }) => output_lines(s, key),
            _ => 0,
        };
        let edit = |key: String, value: Option<String>| ProfileRequest::Edit {
            dir: dir.clone(),
            key,
            value,
            yes: true,
            unconfined_checks: false,
            anyway: false,
            on_proposal,
        };
        let y = key.code == KeyCode::Char('y');
        let enter = key.code == KeyCode::Enter;
        let mut warn = None;
        let request = match s.page.as_mut() {
            Some(ProfilePage::Detect {
                trust_project,
                unconfined_checks,
                focus,
            }) => match key.code {
                KeyCode::Tab
                | KeyCode::BackTab
                | KeyCode::Up
                | KeyCode::Down
                | KeyCode::Char('j' | 'k') => {
                    *focus = 1 - (*focus).min(1);
                    None
                }
                KeyCode::Char(' ') | KeyCode::Left | KeyCode::Right => {
                    let flag = if *focus == 0 {
                        trust_project
                    } else {
                        unconfined_checks
                    };
                    *flag = !*flag;
                    None
                }
                _ if enter => {
                    // Progress ruling: it starts a real agent, so `y` only.
                    warn = Some("press y to start");
                    None
                }
                _ if y => Some((
                    ProfileAsk::Detect,
                    ProfileRequest::Detect {
                        dir: dir.clone(),
                        trust_project: *trust_project,
                        unconfined_checks: *unconfined_checks,
                    },
                )),
                _ => None,
            },
            Some(ProfilePage::Discard) if y => Some((
                ProfileAsk::Discard,
                ProfileRequest::Reject { dir: dir.clone() },
            )),
            Some(ProfilePage::Discard) => {
                warn = enter.then_some("press y to discard");
                None
            }
            Some(ProfilePage::Unset { key: unset }) if y || enter => {
                Some((ProfileAsk::Edit, edit(unset.clone(), None)))
            }
            Some(ProfilePage::Unset { .. }) => None,
            Some(ProfilePage::RawText { text, scroll }) => {
                scroll_by(scroll, key.code, text.lines().count());
                None
            }
            Some(ProfilePage::Row { scroll, .. }) => {
                scroll_by(scroll, key.code, tail_lines);
                None
            }
            Some(ProfilePage::Edit(editor)) => {
                if !enter {
                    on_editor_key(editor, key, width);
                    return vec![];
                }
                match edit_of(editor) {
                    Ok((key, value)) => Some((ProfileAsk::Edit, edit(key, Some(value)))),
                    Err(why) => {
                        editor.error = Some(why);
                        None
                    }
                }
            }
            None => None,
        };
        if let Some(text) = warn {
            self.toast_at(ToastLevel::Warn, text);
        }
        let Some((ask, request)) = request else {
            return vec![];
        };
        // Minor 4: nothing is sent while the link is down; the page keeps what was typed.
        if !self.connected() {
            self.toast_at(ToastLevel::Warn, "not connected");
            return vec![];
        }
        if let Some(s) = self.profile_screen_mut() {
            s.page = None;
            // Decision 31: the edit's outcome is watched for `saved <label>`.
            if let ProfileRequest::Edit { key, .. } = &request {
                let before = s.value_on(on_proposal, key);
                s.saving = Some(Saving {
                    key: key.clone(),
                    on_proposal,
                    before,
                });
            }
        }
        vec![self.profile_send(dir, ask, request)]
    }

    /// A paste while the screen is open goes to an open editor's focused field, and
    /// never reaches the PTY beneath.
    pub(crate) fn on_profile_paste(&mut self, text: &str) {
        let Some(s) = self.profile_screen_mut() else {
            return;
        };
        let Some(ProfilePage::Edit(editor)) = &mut s.page else {
            return;
        };
        match &mut editor.field {
            EditorField::List(area) => area.on_paste(text),
            EditorField::Line(area)
            | EditorField::Env {
                name: area,
                on_value: false,
                ..
            }
            | EditorField::Env {
                value: area,
                on_value: true,
                ..
            } => area.on_paste(&one_line(text)),
            EditorField::Digits(digits) => {
                digits.extend(text.chars().filter(char::is_ascii_digit));
            }
            EditorField::Choice { .. } => {}
        }
    }
}

/// An editor's key other than Enter and Esc.
fn on_editor_key(editor: &mut Editor, key: KeyEvent, width: u16) {
    editor.error = None;
    match &mut editor.field {
        EditorField::Line(area) => line_key(area, key, width.saturating_sub(7)),
        EditorField::List(area) => {
            area.on_key_in(key, width);
        }
        EditorField::Digits(digits) => match key.code {
            KeyCode::Char(c) if c.is_ascii_digit() && digits.len() < 9 => digits.push(c),
            KeyCode::Backspace => {
                digits.pop();
            }
            _ => {}
        },
        EditorField::Choice { options, at } => {
            let n = options.len().max(1);
            match key.code {
                KeyCode::Right | KeyCode::Tab | KeyCode::Char(' ') => *at = (*at + 1) % n,
                KeyCode::Left | KeyCode::BackTab => *at = (*at + n - 1) % n,
                _ => {}
            }
        }
        EditorField::Env {
            name,
            value,
            on_value,
        } => match key.code {
            KeyCode::Tab | KeyCode::BackTab | KeyCode::Up | KeyCode::Down => {
                *on_value = !*on_value;
            }
            _ if *on_value => line_key(value, key, width.saturating_sub(7)),
            _ => line_key(name, key, width.saturating_sub(7)),
        },
    }
}

/// A scrolled page's j/k/PageUp/PageDown over `lines` lines, stopping at the last.
fn scroll_by(scroll: &mut usize, code: KeyCode, lines: usize) {
    let step: isize = match code {
        KeyCode::Char('j') | KeyCode::Down => 1,
        KeyCode::Char('k') | KeyCode::Up => -1,
        KeyCode::PageDown => 10,
        KeyCode::PageUp => -10,
        _ => return,
    };
    *scroll = scroll
        .saturating_add_signed(step)
        .min(lines.saturating_sub(1));
}

/// The output lines a row page shows for `key`: its failed edit's, else its check's.
fn output_lines(s: &super::ProfileScreen, key: &str) -> usize {
    if let Some(proto::RowEditState::Failed { tail, .. }) = s.edit_of(key) {
        return tail.lines().count();
    }
    s.rows()
        .into_iter()
        .find(|r| r.key == key)
        .and_then(|r| r.check)
        .map_or(0, |c| c.tail.lines().count())
}
