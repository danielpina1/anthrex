//! The Profile screen's keys (milestone 9.10 decision 29), out of
//! `app/profile_screen.rs` (decision 1, `AGENTS.md` hard rule 8). One page: the card's
//! keys while it shows (Enter **Use this**, `e` edit the proposal, `x` discard, Esc
//! **Later**), else the profile's (Enter opens a row, `d` detects again, `s` and `r` on a
//! ✗ row). Pure: every request leaves as an `Effect`.

use super::pages::editor_for;
use super::{ADVANCED_ROW, ProfileAsk, ProfilePage, ProfileScreen, Side};
use crate::app::{App, Effect, ToastLevel};
use crate::profile_view::{self, ENV_ADD, Row};
use crossterm::event::{KeyCode, KeyEvent};
use proto::{ProfileRequest, RowEdit, RowEditState};

impl App {
    /// The screen's keys: a page's first, then the card's or the profile's.
    pub(in crate::app) fn on_profile_key(&mut self, key: KeyEvent) -> Vec<Effect> {
        let Some(s) = self.profile_screen_mut() else {
            return vec![];
        };
        if s.page.is_some() {
            return self.on_profile_page_key(key);
        }
        let rows = s.rows();
        let row = rows.get(s.selected).cloned();
        let last = rows.len().saturating_sub(1);
        let card = s.showing_card();
        let on_row = row.as_ref().filter(|r| r.key != ADVANCED_ROW);
        match key.code {
            KeyCode::Esc => self.set_screen(None),
            KeyCode::Char('j') | KeyCode::Down => s.selected = (s.selected + 1).min(last),
            KeyCode::Char('k') | KeyCode::Up => s.selected = s.selected.saturating_sub(1),
            KeyCode::PageDown => s.selected = (s.selected + 10).min(last),
            KeyCode::PageUp => s.selected = s.selected.saturating_sub(10),
            KeyCode::Char('a') => toggle_advanced(s),
            KeyCode::Enter if card => return self.profile_use_this(),
            KeyCode::Enter => match &row {
                Some(r) if r.key == ADVANCED_ROW => toggle_advanced(s),
                Some(r) => {
                    s.page = Some(ProfilePage::Row {
                        key: r.key.clone(),
                        scroll: 0,
                    });
                }
                None => {
                    if let Some(status) = s.status.as_ref().filter(|st| st.unparseable.is_some()) {
                        let text = status.unreadable_text.clone();
                        let text = text.or_else(|| status.unparseable.clone());
                        s.page = Some(ProfilePage::RawText {
                            text: text.unwrap_or_default(),
                            scroll: 0,
                        });
                    }
                }
            },
            KeyCode::Char('e') => {
                if let Some(row) = on_row {
                    // The card edits the proposal; the profile, the stored profile. The
                    // add row starts empty.
                    let side = if card { &s.proposal } else { &s.stored };
                    let text = ProfileScreen::profile_of(side)
                        .filter(|_| row.key != ENV_ADD)
                        .map(|p| profile_view::edit_text(p, &row.key))
                        .unwrap_or_default();
                    s.page = Some(ProfilePage::Edit(Box::new(editor_for(&row.key, &text))));
                }
            }
            KeyCode::Char('u') => {
                if let Some(row) = on_row.filter(|r| r.key != ENV_ADD) {
                    s.page = Some(ProfilePage::Unset {
                        key: row.key.clone(),
                    });
                }
            }
            KeyCode::Char('o') => {
                if let Some(row) = on_row.filter(|r| has_output(s, r)) {
                    s.expanded = match s.expanded.take() {
                        Some(key) if key == row.key => None,
                        _ => Some(row.key.clone()),
                    };
                }
            }
            KeyCode::Char('x') if s.review_proposal() => s.page = Some(ProfilePage::Discard),
            KeyCode::Char('d') if !card => {
                s.page = Some(ProfilePage::Detect {
                    trust_project: false,
                    unconfined_checks: false,
                    focus: 0,
                });
            }
            KeyCode::Char(c @ ('s' | 'r')) => {
                let failed = on_row.and_then(|r| failed_edit(s, &r.key)).cloned();
                if let Some(edit) = failed {
                    return self.profile_failed_row(c == 's', edit);
                }
            }
            _ => {}
        }
        if let Some(s) = self.profile_screen_mut() {
            s.selected = s.selected.min(s.rows().len().saturating_sub(1));
        }
        vec![]
    }

    /// Decision 29: **Use this**: `Confirm` with the proposal's `Shown.toml`, so only
    /// what was shown is stored.
    fn profile_use_this(&mut self) -> Vec<Effect> {
        let Some(s) = self.profile_screen_mut() else {
            return vec![];
        };
        let Side::Ready(shown) = &s.proposal else {
            self.toast_at(ToastLevel::Warn, "the proposal is still loading");
            return vec![];
        };
        let request = ProfileRequest::Confirm {
            dir: s.dir.clone(),
            shown: Some(shown.toml.clone()),
        };
        self.profile_send_now(ProfileAsk::Confirm, request)
    }

    /// Decisions 18 and 19: `s` stores (or applies) the failed edit anyway; `r` reverts.
    fn profile_failed_row(&mut self, anyway: bool, edit: RowEdit) -> Vec<Effect> {
        let Some(s) = self.profile_screen_mut() else {
            return vec![];
        };
        let dir = s.dir.clone();
        if !anyway {
            return self
                .profile_send_now(ProfileAsk::RevertEdit, ProfileRequest::RevertEdit { dir });
        }
        let request = ProfileRequest::Edit {
            dir,
            key: edit.key,
            value: edit.value,
            yes: true,
            unconfined_checks: false,
            anyway: true,
            on_proposal: s.review_proposal(),
        };
        self.profile_send_now(ProfileAsk::Edit, request)
    }

    /// One request from a key, unless the link is down (minor 4).
    pub(super) fn profile_send_now(
        &mut self,
        ask: ProfileAsk,
        request: ProfileRequest,
    ) -> Vec<Effect> {
        if !self.connected() {
            self.toast_at(ToastLevel::Warn, "not connected");
            return vec![];
        }
        let Some(s) = self.profile_screen_mut() else {
            return vec![];
        };
        let dir = s.dir.clone();
        vec![self.profile_send(dir, ask, request)]
    }
}

/// `a`: Advanced opens or folds; the selection stays on its row, or on the `Advanced`
/// line when its row folds away.
fn toggle_advanced(s: &mut ProfileScreen) {
    let key = s.rows().get(s.selected).map(|r| r.key.clone());
    s.advanced = !s.advanced;
    let at = key.and_then(|k| s.index_of(&k));
    s.selected = at.or_else(|| s.index_of(ADVANCED_ROW)).unwrap_or(0);
}

/// The ✗ row edit of `key` (decision 31), when it has one.
fn failed_edit<'a>(s: &'a ProfileScreen, key: &str) -> Option<&'a RowEdit> {
    s.row_edit()
        .filter(|e| e.key == key && matches!(e.state, RowEditState::Failed { .. }))
}

/// Whether `row` has output to show (`o`): its check, or its failed edit's.
fn has_output(s: &ProfileScreen, row: &Row) -> bool {
    row.check.is_some() || failed_edit(s, &row.key).is_some()
}
