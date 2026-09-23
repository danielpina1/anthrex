//! Screen layout and the top-level draw. Spec section 6.1.

pub mod badge;
pub mod conversation;
pub mod dialog;
pub mod modal;
pub mod overview;
pub mod sidebar;
pub mod statusbar;
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
    pub sidebar: Rect,
    pub sidebar_inner: Rect,
    pub sidebar_list: Rect,
    pub sidebar_footer: Rect,
    pub main: Rect,
    pub main_inner: Rect,
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

pub fn layout(area: Rect, sidebar_width: u16) -> Layout {
    let [body, statusbar] =
        RLayout::vertical([Constraint::Min(3), Constraint::Length(1)]).areas(area);
    let (sidebar, main) = if sidebar_width > 0 {
        let [s, m] = RLayout::horizontal([Constraint::Length(sidebar_width), Constraint::Min(10)])
            .areas(body);
        (s, m)
    } else {
        (Rect::new(body.x, body.y, 0, body.height), body)
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
        main,
        main_inner: inset(main),
        statusbar,
    }
}

/// Draws everything and returns the layout so the caller can size the PTY and hit-test the mouse.
pub fn draw(frame: &mut Frame, app: &App) -> Layout {
    let l = layout(
        frame.area(),
        if app.sidebar_visible {
            app.sidebar_width
        } else {
            0
        },
    );
    if app.sidebar_visible {
        sidebar::render(frame, app, &l);
    }
    if app.conversation.is_open() {
        conversation::render(frame, app, l.main);
    } else if app.overview {
        overview::render(frame, app, l.main);
    } else {
        terminal::render(frame, app, l.main);
    }
    statusbar::render(frame, app, l.statusbar);
    if app.modal.is_some() {
        modal::render(frame, app, frame.area());
    }
    l
}

#[cfg(test)]
mod tests;
