//! Screen layout and the top-level draw. Spec section 6.1.

pub mod action_forms;
pub mod action_menu;
pub mod alerts;
pub mod alerts_view;
#[cfg(test)]
pub(crate) mod audit;
#[cfg(test)]
mod audit_tests;
pub mod badge;
pub mod conversation;
pub mod dialog;
pub mod doc_gate;
pub mod doc_note;
pub mod doc_text;
pub mod goal_editor;
pub mod help;
pub mod idle_menu;
pub mod kit;
pub mod modal;
pub mod model_picker;
pub mod models_table;
pub mod overview;
pub mod plan_review;
pub mod profile;
pub mod run_edit;
pub mod run_goal;
pub mod run_iterate;
pub mod run_list;
#[cfg(test)]
pub(crate) mod run_pr_tests;
pub mod settings;
pub mod sidebar;
pub mod splash;
pub mod stats;
pub mod statusbar;
mod statusbar_modes;
pub mod terminal;
pub mod tree_view;

use ratatui::layout::{Constraint, Layout as RLayout, Rect};

pub const DEFAULT_SIDEBAR_WIDTH: u16 = 34;
pub const MIN_SIDEBAR_WIDTH: u16 = 24;
pub const MAX_SIDEBAR_WIDTH: u16 = 60;
pub const SIDEBAR_WIDTH_STEP: u16 = 4;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Layout {
    /// The `agents` block: the sidebar column above the Alerts box (milestone 9.0.5
    /// decision 20).
    pub sidebar: Rect,
    pub sidebar_inner: Rect,
    pub sidebar_list: Rect,
    pub sidebar_footer: Rect,
    /// The Alerts box at the bottom of the sidebar column, and its interior; empty
    /// while the sidebar is hidden.
    pub alerts: Rect,
    pub alerts_inner: Rect,
    pub main: Rect,
    pub main_inner: Rect,
    /// Everything above the status bar, the sidebar column included: the plan
    /// review's area (milestone 9.0.5 decision 12).
    pub body: Rect,
    pub statusbar: Rect,
}

fn inset(r: Rect) -> Rect {
    Rect {
        x: r.x.saturating_add(1),
        y: r.y.saturating_add(1),
        width: r.width.saturating_sub(2),
        height: r.height.saturating_sub(2),
    }
}

/// What sizes the sidebar column's two blocks (milestone 9.0.7 decision 9): the
/// agents block's tree rows and the Alerts box's content lines (decision 10's lines
/// for every alert at the box's interior width; 0 with no alert).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct SidebarSizing {
    pub tree_rows: usize,
    pub alert_lines: usize,
}

/// Decision 9: the Alerts box's interior rows in a sidebar column `column_height`
/// rows tall — 1 (`no alerts`) with no alert, else `max(3, min(C, H/2 − 2,
/// H − (R + 3) − 2))`, every term saturating, and at most `H − 2`. `R + 3` is the
/// agents block's need: its rows, two borders and the footer row.
pub fn alerts_box_rows(column_height: u16, sizing: SidebarSizing) -> u16 {
    let h = usize::from(column_height);
    let rows = if sizing.alert_lines == 0 {
        1
    } else {
        let half = (h / 2).saturating_sub(2);
        let free = h
            .saturating_sub(sizing.tree_rows.saturating_add(3))
            .saturating_sub(2);
        sizing.alert_lines.min(half).min(free).max(3)
    };
    u16::try_from(rows.min(h.saturating_sub(2))).unwrap_or(u16::MAX)
}

/// The screen's areas, the Alerts box sized as for `alert_count` two-line alerts
/// beside an empty tree (decision 9): kept for the tests that need only the panes.
/// A test that draws alerts and hit-tests the sidebar calls [`layout_for`].
pub fn layout(area: Rect, sidebar_width: u16, alert_count: usize) -> Layout {
    let sizing = SidebarSizing {
        tree_rows: 0,
        alert_lines: alert_count.saturating_mul(2),
    };
    layout_sized(area, sidebar_width, sizing)
}

/// The screen's areas, the sidebar column split by [`alerts_box_rows`].
pub fn layout_sized(area: Rect, sidebar_width: u16, sizing: SidebarSizing) -> Layout {
    let [body, statusbar] =
        RLayout::vertical([Constraint::Min(3), Constraint::Length(1)]).areas(area);
    let (column, main) = if sidebar_width > 0 {
        let [s, m] = RLayout::horizontal([Constraint::Length(sidebar_width), Constraint::Min(10)])
            .areas(body);
        (s, m)
    } else {
        (Rect::new(body.x, body.y, 0, body.height), body)
    };
    let box_height = if column.width > 0 {
        (alerts_box_rows(column.height, sizing) + 2).min(column.height)
    } else {
        0
    };
    let sidebar = Rect {
        height: column.height - box_height,
        ..column
    };
    let alerts = Rect {
        y: column.y + sidebar.height,
        height: box_height,
        ..column
    };
    let sidebar_inner = inset(sidebar);
    Layout {
        sidebar,
        sidebar_inner,
        // Decision 27: the interior less the footer row; no spacer.
        sidebar_list: Rect {
            height: sidebar_inner.height.saturating_sub(1),
            ..sidebar_inner
        },
        sidebar_footer: Rect {
            y: sidebar_inner.y + sidebar_inner.height.saturating_sub(1),
            height: sidebar_inner.height.min(1),
            ..sidebar_inner
        },
        alerts,
        alerts_inner: inset(alerts),
        main,
        main_inner: inset(main),
        body,
        statusbar,
    }
}

/// `layout_for` and `draw`: `ui/frame.rs`.
mod frame;
pub use frame::{draw, layout_for};

#[cfg(test)]
mod tests;
