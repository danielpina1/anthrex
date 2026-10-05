//! Milestone 9.0.7 decision 34: the help's pure state and keys. The rows and their
//! lines are `ui::help`'s; this file picks where the help opens and moves it.

use super::region::KeyRegion;
use super::screens::Screen;
use super::{App, Effect, Modal};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::layout::Rect;

/// The open help: the line its view starts from. The renderer and the keys both stop
/// it at `ui::help::max_scroll`, so a scroll past the end draws the end.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct HelpView {
    pub scroll: u16,
    /// The line `C-b ?` opened on (its context's group): the scroll may stop there
    /// even past the last page, so that group's name is the first drawn (fix round 1).
    pub opened: u16,
}

impl App {
    /// `C-b ?`: the help, scrolled to the group of whatever had the keys (decision 34).
    pub(crate) fn open_help(&mut self) -> Vec<Effect> {
        let group = self.help_context();
        let groups = crate::ui::help::help_groups(&self.settings.prefix_label);
        let top = crate::ui::help::group_tops(&groups)
            .into_iter()
            .zip(&groups)
            .find(|(_, g)| g.name == group)
            .map_or(0, |(top, _)| top);
        let top = u16::try_from(top).unwrap_or(u16::MAX);
        self.modal = Some(Modal::Help(HelpView {
            scroll: top,
            opened: top,
        }));
        vec![]
    }

    /// The group of decision 34's mapping for the region that has the keys.
    fn help_context(&self) -> &'static str {
        match self.key_region() {
            KeyRegion::Sidebar => "sidebar",
            KeyRegion::Overview if self.run_view.is_some() => "run view",
            KeyRegion::Overview => "sidebar",
            KeyRegion::Review => "plan review",
            KeyRegion::Alerts => "alerts",
            KeyRegion::Conversation => "conversation",
            KeyRegion::Screen => match &self.screen {
                Some(Screen::Profile(_)) => "profile",
                Some(Screen::Settings(_)) => "settings",
                Some(Screen::DocGate(_)) => "document gate",
                _ => "global",
            },
            KeyRegion::Pane | KeyRegion::Dialog => "global",
        }
    }

    /// The whole terminal, which the help is drawn over: the last frame's body and the
    /// status bar's one row under it (`ui::layout_sized`).
    fn help_area(&self) -> Rect {
        Rect {
            height: self.body_area.height.saturating_add(1),
            ..self.body_area
        }
    }

    /// Decision 34's keys: `j`/`k`/Down/Up a line, PgUp/PgDn a page, Tab/BackTab the
    /// (or Shift+Tab) the next/previous group's header, Esc or `q` close. Every other key does nothing.
    /// The scroll is clamped first (a resize may have moved the end), then stops at
    /// the end the renderer draws.
    pub(super) fn on_help_key(&mut self, mut view: HelpView, key: KeyEvent) -> Vec<Effect> {
        if matches!(key.code, KeyCode::Esc | KeyCode::Char('q')) {
            return vec![];
        }
        let area = self.help_area();
        let p = self.palette();
        let groups = crate::ui::help::help_groups(&self.settings.prefix_label);
        let last = crate::ui::help::max_scroll(&groups, &view, area, p);
        let page = crate::ui::help::view_rows(&groups, area, p)
            .saturating_sub(2)
            .max(1);
        let from = usize::from(view.scroll).min(last);
        let tops = crate::ui::help::group_tops(&groups);
        // Shift+Tab arrives as BackTab or as Tab with Shift (`app/action_forms.rs`).
        let back = key.code == KeyCode::BackTab
            || (key.code == KeyCode::Tab && key.modifiers.contains(KeyModifiers::SHIFT));
        let to = match key.code {
            _ if back => tops.iter().rev().copied().find(|&t| t < from).unwrap_or(0),
            KeyCode::Char('j') | KeyCode::Down => from + 1,
            KeyCode::Char('k') | KeyCode::Up => from.saturating_sub(1),
            KeyCode::PageDown => from + page,
            KeyCode::PageUp => from.saturating_sub(page),
            KeyCode::Tab => tops.iter().copied().find(|&t| t > from).unwrap_or(from),
            _ => from,
        };
        view.scroll = u16::try_from(to.min(last)).unwrap_or(u16::MAX);
        self.modal = Some(Modal::Help(view));
        vec![]
    }
}
