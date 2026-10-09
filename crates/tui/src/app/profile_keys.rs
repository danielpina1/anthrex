//! The Profile screen's keys, out of `app/profile_screen.rs` (milestone 9.10 decision 1,
//! `AGENTS.md` hard rule 8). Pure: every request leaves as an `Effect`.

use super::pages::editor_for;
use super::{ProfilePage, ProfileScreen, ProfileTab, Side};
use crate::app::{App, Effect, ToastLevel};
use crate::profile_view::{self, ENV_ADD};
use crossterm::event::{KeyCode, KeyEvent};
use proto::ProposalState;

impl App {
    /// The screen's keys: a page's first, then the screen's, then the tab's.
    pub(in crate::app) fn on_profile_key(&mut self, key: KeyEvent) -> Vec<Effect> {
        let Some(s) = self.profile_screen_mut() else {
            return vec![];
        };
        if s.page.is_some() {
            return self.on_profile_page_key(key);
        }
        match key.code {
            KeyCode::Esc => self.set_screen(None),
            KeyCode::Tab | KeyCode::BackTab => {
                s.tab = match s.tab {
                    ProfileTab::Status => ProfileTab::Profile,
                    ProfileTab::Profile => ProfileTab::Status,
                };
            }
            KeyCode::Char('s') => s.store_on_pass = !s.store_on_pass,
            KeyCode::Char('d') => {
                s.page = Some(ProfilePage::Detect {
                    trust_project: false,
                    unconfined_checks: false,
                    focus: 0,
                });
            }
            KeyCode::Char('x') => s.page = Some(ProfilePage::Reject),
            KeyCode::Char('c') => match &s.proposal {
                Side::Ready(shown) if s.proposal_state() == Some(&ProposalState::Ready) => {
                    s.page = Some(ProfilePage::Confirm {
                        toml: shown.toml.clone(),
                        scroll: 0,
                    });
                }
                _ => self.toast_at(ToastLevel::Warn, "no proposal is ready to confirm"),
            },
            _ if s.tab == ProfileTab::Profile => self.on_profile_tab_key(key),
            _ => {}
        }
        vec![]
    }

    /// The Profile tab's keys: move, expand a check, switch view, edit, unset.
    fn on_profile_tab_key(&mut self, key: KeyEvent) {
        let Some(s) = self.profile_screen_mut() else {
            return;
        };
        let rows = s.rows();
        let row = rows.get(s.selected).cloned();
        let last = rows.len().saturating_sub(1);
        match key.code {
            KeyCode::Char('j') | KeyCode::Down => s.selected = (s.selected + 1).min(last),
            KeyCode::Char('k') | KeyCode::Up => s.selected = s.selected.saturating_sub(1),
            KeyCode::PageDown => s.selected = (s.selected + 10).min(last),
            KeyCode::PageUp => s.selected = s.selected.saturating_sub(10),
            KeyCode::Char('p') => {
                s.proposed = !s.proposed;
                s.selected = 0;
                s.expanded = None;
            }
            KeyCode::Enter => {
                if let Some(row) = row.filter(|r| r.check.is_some()) {
                    s.expanded = match s.expanded.take() {
                        Some(key) if key == row.key => None,
                        _ => Some(row.key),
                    };
                }
            }
            KeyCode::Char('e') => {
                if let Some(row) = row {
                    // An edit applies to the stored profile, so it starts from there;
                    // the add row starts empty.
                    let text = ProfileScreen::profile_of(&s.stored)
                        .filter(|_| row.key != ENV_ADD)
                        .map(|p| profile_view::edit_text(p, &row.key))
                        .unwrap_or_default();
                    let mut editor = editor_for(&row.key, &text);
                    editor.from_proposal = s.proposed;
                    s.page = Some(ProfilePage::Edit(Box::new(editor)));
                }
            }
            KeyCode::Char('u') => {
                if let Some(row) = row.filter(|r| r.key != ENV_ADD) {
                    s.page = Some(ProfilePage::Unset { key: row.key });
                }
            }
            _ => {}
        }
        s.selected = s.selected.min(last);
    }
}
