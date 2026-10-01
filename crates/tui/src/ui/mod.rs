//! Screen layout and the top-level draw. Spec section 6.1.

pub mod action_forms;
pub mod action_menu;
pub mod alerts;
pub mod badge;
pub mod conversation;
pub mod dialog;
pub mod kit;
pub mod modal;
pub mod overview;
pub mod plan_review;
pub mod run_edit;
pub mod run_goal;
pub mod sidebar;
pub mod statusbar;
mod statusbar_modes;
pub mod terminal;
pub mod tree_view;

use crate::app::App;
use ratatui::Frame;
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

/// Decision 20: the Alerts box's interior rows for `count` alerts in a sidebar column
/// `height` rows tall — one (`no alerts`) with none, else `min(n, ALERTS_MAX_ROWS,
/// max(1, height / 3 − 2))`.
pub fn alerts_rows(count: usize, height: u16) -> u16 {
    if count == 0 {
        return 1;
    }
    let share = (height / 3).saturating_sub(2).max(1);
    let count = u16::try_from(count).unwrap_or(u16::MAX);
    count.min(alerts::ALERTS_MAX_ROWS).min(share)
}

/// The screen's areas. `alert_count`: how many alerts the box lists (decision 20),
/// which sets its height; `crate::app::alerts(app).len()`.
pub fn layout(area: Rect, sidebar_width: u16, alert_count: usize) -> Layout {
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
        (alerts_rows(alert_count, column.height) + 2).min(column.height)
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
        sidebar_list: Rect {
            height: sidebar_inner.height.saturating_sub(2),
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

/// The layout the frame draws for `app` in `area`: the sidebar's width while it is
/// shown, and the Alerts box sized by the alerts it lists.
pub fn layout_for(app: &App, area: Rect) -> Layout {
    let width = if app.sidebar_visible {
        app.sidebar_width
    } else {
        0
    };
    layout(area, width, crate::app::alerts(app).len())
}

/// Draws everything and returns the layout so the caller can size the PTY and hit-test the mouse.
pub fn draw(frame: &mut Frame, app: &App) -> Layout {
    let l = layout_for(app, frame.area());
    if app.plan_review.is_some() {
        // Milestone 9.0.5 decision 12: the review covers the body; the status bar stays.
        plan_review::render(frame, app, l.body);
    } else {
        if app.sidebar_visible {
            sidebar::render(frame, app, &l);
            alerts::render(frame, app, &l);
        }
        if app.conversation.is_open() {
            conversation::render(frame, app, l.main);
        } else if app.overview {
            overview::render(frame, app, l.main);
        } else {
            terminal::render(frame, app, l.main);
        }
    }
    statusbar::render(frame, app, l.statusbar);
    if app.modal.is_some() {
        modal::render(frame, app, frame.area());
    }
    l
}

#[cfg(test)]
mod tests;
