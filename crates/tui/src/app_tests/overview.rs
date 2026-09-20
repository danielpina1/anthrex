use super::*;
use crate::tree::NodeKey;
use ratatui::layout::Rect;

fn toggle(app: &mut App) -> Vec<Effect> {
    prefix(app);
    press(app, KeyCode::Char('T'), KeyModifiers::NONE)
}

fn opened() -> (App, crate::ui::Layout) {
    let mut app = app_with(tree::example_windows());
    assert!(toggle(&mut app).is_empty());
    let layout = crate::ui::layout(Rect::new(0, 0, 120, 30), app.sidebar_width);
    app.set_tree_viewports(layout.sidebar_list.height, layout.main_inner.height);
    (app, layout)
}

fn assert_closed(app: &App) {
    assert!(!app.overview);
    assert_eq!(app.tree_input, None);
    assert!(!app.keymap.tree_mode());
    assert_eq!(app.tree.selected, None);
    assert!(app.tree.filter.is_empty());
}

#[test]
fn overview_opens_tree_mode_and_closes_with_it() {
    let mut app = app_with(tree::example_windows());
    for close_with_escape in [false, true] {
        assert!(toggle(&mut app).is_empty());
        assert!(app.overview);
        assert_eq!(app.tree_input, Some(TreeInput::Navigate));
        assert!(app.keymap.tree_mode());
        let effects = if close_with_escape {
            press(&mut app, KeyCode::Esc, KeyModifiers::NONE)
        } else {
            toggle(&mut app)
        };
        assert!(effects.is_empty());
        assert_closed(&app);
    }
}

#[test]
fn overview_does_not_resize_the_pty() {
    let mut app = app_with(tree::example_windows());
    for _ in 0..2 {
        assert!(toggle(&mut app).is_empty());
        assert_eq!(app.term_size, (80, 24));
        assert_eq!(app.parser.screen().size(), (24, 80));
        assert!(app.set_terminal_size(80, 24).is_empty());
        assert!(app.pending_resize.is_none());
    }
}

#[test]
fn overview_keeps_a_hidden_sidebar_hidden_and_preserves_pty_size() {
    let mut app = app_with(tree::example_windows());
    prefix(&mut app);
    assert!(press(&mut app, KeyCode::Char('s'), KeyModifiers::NONE).is_empty());
    let area = Rect::new(0, 0, 82, 27);
    let before = crate::ui::layout(area, 0);
    assert_eq!(
        (before.main_inner.width, before.main_inner.height),
        (80, 24)
    );
    for _ in 0..2 {
        assert!(toggle(&mut app).is_empty());
        assert!(!app.sidebar_visible);
        let after = crate::ui::layout(
            area,
            if app.sidebar_visible {
                app.sidebar_width
            } else {
                0
            },
        );
        assert_eq!(after.main_inner, before.main_inner);
        assert!(
            app.set_terminal_size(after.main_inner.width, after.main_inner.height)
                .is_empty()
        );
        assert_eq!(app.term_size, (80, 24));
        assert_eq!(app.parser.screen().size(), (24, 80));
        assert!(app.pending_resize.is_none());
    }
    assert_closed(&app);
    prefix(&mut app);
    assert!(press(&mut app, KeyCode::Char('t'), KeyModifiers::NONE).is_empty());
    assert!(
        app.sidebar_visible,
        "ordinary tree mode still reveals the sidebar"
    );
}

#[test]
fn a_click_in_the_overview_acts_like_enter() {
    let (mut app, layout) = opened();
    assert_eq!(
        app.on_click(layout.main_inner.x, layout.main_inner.y + 5, &layout),
        vec![Effect::Send(ClientMsg::Subscribe {
            window_id: 2,
            cols: 80,
            rows: 24,
        })]
    );
    assert_eq!(app.focused, Some(2));
    assert_closed(&app);
}

#[test]
fn overview_clicks_activate_projects_subagents_and_the_focused_window() {
    let (mut app, layout) = opened();
    assert!(
        app.on_click(layout.main_inner.x, layout.main_inner.y, &layout)
            .is_empty()
    );
    assert!(app.tree.is_collapsed(&NodeKey::Project("/r/shop".into())));
    assert_eq!(app.tree.selected, Some(NodeKey::Project("/r/shop".into())));
    assert!(app.overview);
    assert_eq!(app.tree_input, Some(TreeInput::Navigate));

    let (mut app, layout) = opened();
    assert!(
        app.on_click(layout.main_inner.x, layout.main_inner.y + 1, &layout)
            .is_empty()
    );
    assert_closed(&app);

    let (mut app, layout) = opened();
    assert_eq!(
        app.on_click(layout.main_inner.x, layout.main_inner.y + 9, &layout),
        vec![Effect::Send(ClientMsg::Subscribe {
            window_id: 4,
            cols: 80,
            rows: 24,
        })]
    );
    assert_eq!(app.focused, Some(4));
    assert_closed(&app);
}

#[test]
fn overview_click_activates_while_filtering_and_with_sidebar_hidden() {
    let (mut app, _) = opened();
    app.sidebar_visible = false;
    let layout = crate::ui::layout(Rect::new(0, 0, 120, 30), 0);
    press(&mut app, KeyCode::Char('/'), KeyModifiers::NONE);
    assert!(app.on_paste("billing".into()).is_empty());
    assert_eq!(app.tree_input, Some(TreeInput::Filter));
    assert_eq!(
        app.on_click(layout.main_inner.x, layout.main_inner.y + 1, &layout),
        vec![Effect::Send(ClientMsg::Subscribe {
            window_id: 2,
            cols: 80,
            rows: 24,
        })]
    );
    assert_closed(&app);
}

#[test]
fn overview_clicks_respect_modal_borders_and_undrawn_rows() {
    let (mut app, layout) = opened();
    for (x, y) in [
        (layout.main.x, layout.main_inner.y + 5),
        (layout.main_inner.x, layout.main.y),
        (layout.main.right() - 1, layout.main_inner.y + 5),
        (layout.main_inner.x, layout.main.bottom() - 1),
        (layout.main_inner.x, layout.main_inner.y + 16),
    ] {
        assert!(app.on_click(x, y, &layout).is_empty());
        assert!(app.overview);
        assert_eq!(app.focused, Some(1));
    }
    app.modal = Some(Modal::Help);
    assert!(
        app.on_click(layout.main_inner.x, layout.main_inner.y + 5, &layout)
            .is_empty()
    );
    assert!(app.overview);
    assert_eq!(app.focused, Some(1));
}

#[test]
fn overview_click_uses_its_own_scrolled_viewport() {
    let (mut app, _) = opened();
    let layout = crate::ui::layout(Rect::new(0, 0, 120, 13), app.sidebar_width);
    app.set_tree_viewports(layout.sidebar_list.height, layout.main_inner.height);
    app.tree.overview.top = 4;
    app.tree.sidebar.top = 0;
    assert_eq!(
        app.on_click(layout.main_inner.x, layout.main_inner.y + 1, &layout),
        vec![Effect::Send(ClientMsg::Subscribe {
            window_id: 2,
            cols: 80,
            rows: 24,
        })]
    );
    assert_closed(&app);
}

#[test]
fn elapsed_duration_saturates_when_extrapolating_old_lists() {
    let mut app = app_with(tree::example_windows());
    app.windows_received_at = Instant::now() - Duration::from_secs(5);
    let mut window = app.windows[0].clone();
    window.since_secs = u64::MAX;
    assert_eq!(app.elapsed_secs(&window), u64::MAX);
}
