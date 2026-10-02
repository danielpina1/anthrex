//! The top-level draw and the layout it draws with (moved out of `ui/mod.rs` by the
//! final fix wave, which builds a frame's alerts once here: task 5's deferred minor).

use super::alerts::Shown;
use super::{
    Layout, SidebarSizing, alerts, alerts_view, conversation, layout_sized, modal, overview,
    plan_review, profile, settings, sidebar, stats, statusbar, terminal,
};
use crate::app::App;
use ratatui::Frame;
use ratatui::layout::Rect;

/// The layout the frame draws for `app` in `area`, and the one every mouse path
/// hit-tests against (Review focus 3): the sidebar's width while it is shown, the
/// Alerts box sized by the tree's rows and the alerts' lines at the box's own
/// interior width (decision 9).
pub fn layout_for(app: &App, area: Rect) -> Layout {
    layout_shown(app, area).0
}

/// [`layout_for`], and the alerts it sized the box with, for the frame to share.
fn layout_shown(app: &App, area: Rect) -> (Layout, Shown) {
    if !app.sidebar_visible {
        let layout = layout_sized(area, 0, SidebarSizing::default());
        return (layout, Shown::of(app, None));
    }
    let width = app.sidebar_width;
    // The column can be narrower than asked on a narrow terminal: measure the alerts
    // at the interior the box gets.
    let interior = layout_sized(area, width, SidebarSizing::default())
        .alerts_inner
        .width;
    let shown = Shown::of(app, Some(interior));
    let sizing = SidebarSizing {
        tree_rows: app.rows().len(),
        alert_lines: shown.lines(),
    };
    (layout_sized(area, width, sizing), shown)
}

/// Draws everything and returns the layout so the caller can size the PTY and hit-test the mouse.
pub fn draw(frame: &mut Frame, app: &App) -> Layout {
    let (l, shown) = layout_shown(app, frame.area());
    use crate::app::screens::Screen;
    if let Some(Screen::Profile(screen)) = &app.screen {
        // Milestone 9.0.6 decision 33: a screen covers the body; the status bar stays.
        profile::render(frame, app, screen, l.body);
    } else if let Some(Screen::Settings(screen)) = &app.screen {
        settings::render(frame, app, screen, l.body);
    } else if let Some(Screen::Stats(screen)) = &app.screen {
        stats::render(frame, app, screen, l.body);
    } else if app.plan_review.is_some() {
        // Milestone 9.0.5 decision 12: the review covers the body; the status bar stays.
        plan_review::render(frame, app, l.body);
    } else {
        if app.sidebar_visible {
            sidebar::render(frame, app, &l);
            alerts::render_shown(frame, app, &l, &shown);
        }
        if app.alerts_focus.is_some() {
            // Milestone 9.0.7 decision 11: the Alerts view covers the main pane; what
            // is under it is as it was when the view closes.
            alerts_view::render_with(frame, app, l.main, &shown.all);
        } else if app.conversation.is_open() {
            conversation::render(frame, app, l.main);
        } else if app.overview {
            overview::render(frame, app, l.main);
        } else {
            terminal::render(frame, app, l.main);
        }
    }
    statusbar::render_with(frame, app, l.statusbar, &shown.all);
    if app.modal.is_some() {
        modal::render(frame, app, frame.area());
    }
    l
}
