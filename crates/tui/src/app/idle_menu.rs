//! Milestone 9.3 decision 32 (KG §6): the sidebar's idle orchestrator row. Enter focuses
//! its window; `.` opens this two-entry menu, `new goal here` (the goal dialog on the
//! chain's project, which continues the chain by default, decision 25) and `close` (the
//! window's existing kill confirm page, `PendingAction::Kill`). Pure: the kill leaves as
//! the confirm page's own effect.

use super::{App, Effect, Modal, PendingAction};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use std::path::PathBuf;

/// The menu's entries, in order.
pub const ENTRIES: [&str; 2] = ["new goal here", "close"];

/// The open menu: the chain it was opened on, as the snapshot named it then.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IdleMenu {
    pub chain: String,
    pub project: PathBuf,
    pub window_id: u32,
    pub selected: usize,
}

impl App {
    /// The window of `chain` while the snapshot lists it idle and unended, never 0.
    pub(crate) fn idle_window(&self, chain: &str) -> Option<u32> {
        (self.runs.idle_orchestrators.iter())
            .find(|idle| idle.chain == chain && !idle.fresh)
            .and_then(|idle| idle.window_id)
            .filter(|id| *id != 0)
    }

    /// `.` on an idle orchestrator's row.
    pub(crate) fn open_idle_menu(&mut self, chain: &str) -> Vec<Effect> {
        let Some(window_id) = self.idle_window(chain) else {
            return vec![];
        };
        let project = (self.runs.idle_orchestrators.iter())
            .find(|idle| idle.chain == chain)
            .map(|idle| idle.project.clone())
            .unwrap_or_default();
        self.modal = Some(Modal::IdleMenu(IdleMenu {
            chain: chain.to_owned(),
            project,
            window_id,
            selected: 0,
        }));
        vec![]
    }

    /// `j`/`k` and the arrows move, Enter chooses, Esc (or `q`) closes.
    pub(super) fn on_idle_menu_key(&mut self, mut menu: IdleMenu, key: KeyEvent) -> Vec<Effect> {
        if key
            .modifiers
            .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT)
        {
            self.modal = Some(Modal::IdleMenu(menu));
            return vec![];
        }
        match key.code {
            KeyCode::Esc | KeyCode::Char('q') => return vec![],
            KeyCode::Up | KeyCode::Char('k') => menu.selected = menu.selected.saturating_sub(1),
            KeyCode::Down | KeyCode::Char('j') => {
                menu.selected = (menu.selected + 1).min(ENTRIES.len() - 1);
            }
            KeyCode::Enter => return self.choose_idle_entry(&menu),
            _ => {}
        }
        self.modal = Some(Modal::IdleMenu(menu));
        vec![]
    }

    /// Re-checked against the current snapshot: a chain no longer idle on the same
    /// window (a next goal adopted it, or it ended) is told, never acted on.
    fn choose_idle_entry(&mut self, menu: &IdleMenu) -> Vec<Effect> {
        if self.idle_window(&menu.chain) != Some(menu.window_id) {
            let chain = crate::safe_text::one_line(&menu.chain);
            self.toast(format!("{chain} is no longer idle"));
            return vec![];
        }
        if menu.selected == 0 {
            self.open_goal_form_in(menu.project.clone());
            return vec![];
        }
        let name = (self.windows.iter())
            .find(|window| window.id == menu.window_id)
            .map_or_else(|| menu.chain.clone(), |window| window.name.clone());
        self.modal = Some(Modal::Confirm {
            message: format!("Kill '{name}'?"),
            action: PendingAction::Kill(menu.window_id),
        });
        vec![]
    }
}

#[cfg(test)]
#[path = "idle_menu_tests.rs"]
pub(crate) mod tests;
