//! The inspector wired into the overview: the panel under the canvas, what it
//! says about the node the keys or the mouse selected, the `i` toggle, and the
//! collapse to milestone 4.6's single line on a short terminal.
//!
//! The panel is asserted through a real frame, read out of the rect
//! `overview::view` reports — the same geometry the click path hit-tests
//! against, so a panel drawn somewhere other than where the view says it is
//! shows up here.

use super::*;
use crate::inspector::{INSPECTOR_HEIGHT, MIN_INTERIOR_FOR_PANEL};

/// A terminal one row taller than the shortest that still earns a panel, and
/// one row shorter. The overview's interior is the terminal minus the status
/// bar and its own two border rows.
const TALL: u16 = MIN_INTERIOR_FOR_PANEL + 3;
const SHORT: u16 = TALL - 1;

/// The rect below the canvas, drawn at `width` x `height`: the panel when it is
/// shown, the single line when it is not.
fn below_the_canvas(app: &App, width: u16, height: u16) -> String {
    let sidebar_width = if app.sidebar_visible {
        app.sidebar_width
    } else {
        0
    };
    let layout = crate::ui::layout(Rect::new(0, 0, width, height), sidebar_width);
    let view = overview::view(app, layout.main);
    text_in(&drawn(app, width, height), view.footer)
}

/// The panel's title row, trimmed: its glyph and the name of the node it is
/// showing.
fn panel_title(panel: &str) -> String {
    panel
        .lines()
        .nth(1)
        .expect("a panel has a title row")
        .trim_matches(|c| c == '│' || c == ' ')
        .to_owned()
}

#[test]
fn the_overview_shows_the_inspector_for_the_selected_node() {
    let (app, layout) = opened_at(200, 50);
    let view = overview::view(&app, layout.main);
    assert_eq!(view.footer.height, INSPECTOR_HEIGHT);

    let panel = below_the_canvas(&app, 200, 50);
    let rows: Vec<&str> = panel.lines().collect();
    assert!(
        rows[0].starts_with('╭') && rows[0].ends_with('╮'),
        "the panel is a rounded box:\n{panel}"
    );
    assert_eq!(panel_title(&panel), "⠋ 1 api-worker", "{panel}");
    for field in [
        "status",
        "working",
        "model",
        "claude-opus-5",
        "sub-agents",
        "3, 2 running",
        "runtime",
        "claude",
        "dir",
        "/r/shop",
    ] {
        assert!(panel.contains(field), "{field:?} missing from:\n{panel}");
    }
    // The single line the panel replaced is not drawn beside it.
    assert!(!panel.contains("claude  claude-opus-5  working"), "{panel}");
}

#[test]
fn selecting_another_node_changes_the_panel() {
    let (mut app, _) = opened_at(200, 50);
    assert_eq!(
        panel_title(&below_the_canvas(&app, 200, 50)),
        "⠋ 1 api-worker"
    );

    select(
        &mut app,
        NodeKey::Subagent {
            window_id: 1,
            id: "a2".into(),
        },
    );
    let panel = below_the_canvas(&app, 200, 50);
    assert_eq!(panel_title(&panel), "⠋ grep handlers", "{panel}");
    // A sub-agent's own fields, and the parent `a2` was spawned by.
    assert!(panel.contains("spawned by"), "{panel}");
    assert!(panel.contains("map routes"), "{panel}");
    assert!(panel.contains("general-purpose"), "{panel}");
}

/// The behaviour the milestone exists for: click a sub-agent three levels down
/// and the panel names who spawned it, which the tree cannot say.
#[test]
fn clicking_a_subagent_fills_the_panel() {
    let (mut app, layout) = opened_at(200, 50);
    let key = NodeKey::Subagent {
        window_id: 4,
        id: "b2".into(),
    };
    let (x, y) = box_middle(&app, layout.main, &key);
    assert!(
        app.on_click(x, y, &layout).is_empty(),
        "a click only selects"
    );
    assert_eq!(app.tree.selected, Some(key));

    let panel = below_the_canvas(&app, 200, 50);
    assert_eq!(panel_title(&panel), "⠋ find tokens", "{panel}");
    assert!(panel.contains("spawned by"), "{panel}");
    assert!(
        panel.contains("style pass"),
        "b2 was spawned by b1, not by its window:\n{panel}"
    );
    assert!(panel.contains("depth"), "{panel}");
}

#[test]
fn i_toggles_the_inspector_and_the_single_line_returns() {
    let (mut app, layout) = opened_at(200, 50);
    assert!(app.inspector_visible, "the panel is on by default");
    let with_panel = overview::view(&app, layout.main).canvas.height;

    press(&mut app, KeyCode::Char('i'), KeyModifiers::NONE);
    assert!(!app.inspector_visible);
    let view = overview::view(&app, layout.main);
    assert_eq!(
        view.footer.height, 1,
        "off means milestone 4.6's single line"
    );
    assert_eq!(view.canvas.height, with_panel + INSPECTOR_HEIGHT - 1);
    let footer = below_the_canvas(&app, 200, 50);
    assert!(
        footer.contains("⠋ 1 api-worker  claude  claude-opus-5  working"),
        "{footer}"
    );
    assert!(!footer.contains('╭'), "no panel is drawn:\n{footer}");

    press(&mut app, KeyCode::Char('i'), KeyModifiers::NONE);
    assert!(app.inspector_visible);
    assert_eq!(
        panel_title(&below_the_canvas(&app, 200, 50)),
        "⠋ 1 api-worker"
    );

    // The choice outlives the overview it was made in (decision 7).
    press(&mut app, KeyCode::Char('i'), KeyModifiers::NONE);
    assert!(toggle(&mut app).is_empty());
    assert_closed(&app);
    assert!(toggle(&mut app).is_empty());
    assert!(!app.inspector_visible, "the session remembers the choice");
}

#[test]
fn a_short_terminal_collapses_to_one_line() {
    let (app, _) = opened_at(200, SHORT);
    let layout = crate::ui::layout(Rect::new(0, 0, 200, SHORT), app.sidebar_width);
    let view = overview::view(&app, layout.main);
    assert_eq!(layout.main_inner.height, MIN_INTERIOR_FOR_PANEL - 1);
    assert_eq!(view.footer.height, 1);
    assert!(
        view.canvas.height > 0,
        "a short terminal loses the panel, never the canvas"
    );
    let footer = below_the_canvas(&app, 200, SHORT);
    assert!(footer.contains("1 api-worker"), "{footer}");
    assert!(!footer.contains('╭'), "{footer}");
    assert!(app.inspector_visible, "the collapse is not a toggle");

    // One row taller and the panel is back.
    let (app, _) = opened_at(200, TALL);
    let layout = crate::ui::layout(Rect::new(0, 0, 200, TALL), app.sidebar_width);
    assert_eq!(layout.main_inner.height, MIN_INTERIOR_FOR_PANEL);
    let view = overview::view(&app, layout.main);
    assert_eq!(view.footer.height, INSPECTOR_HEIGHT);
    assert!(view.canvas.height > 0);
    assert!(below_the_canvas(&app, 200, TALL).contains('╭'));
}

#[test]
fn the_canvas_loses_exactly_the_inspectors_height() {
    let (mut app, layout) = opened_at(200, 50);
    let interior = layout.main_inner;
    let view = overview::view(&app, layout.main);
    assert_eq!(view.footer.height, INSPECTOR_HEIGHT);
    assert_eq!(view.footer.y, view.canvas.bottom(), "pinned to the bottom");
    assert_eq!(view.footer.width, interior.width, "the full width");
    assert_eq!(
        view.canvas.height + view.footer.height,
        interior.height,
        "the panel and the canvas are the whole interior"
    );
    // The viewport every mouse gesture hit-tests against is the canvas the
    // frame was drawn with, panel or no panel.
    assert_eq!(app.graph_area, view.canvas);

    // And the rows the panel took are no longer the canvas's. The cell clicked
    // is the middle of a box the layout puts just below the canvas, so a mouse
    // path still hit-testing against the taller canvas selects that box; the
    // panel is in front of it, and nothing is selected.
    let (mut small, small_layout) = opened_at(120, 30);
    let small_view = overview::view(&small, small_layout.main);
    assert_eq!(small_view.pan, Pan::default());
    let behind = small_view
        .layout
        .nodes
        .iter()
        .find(|node| {
            (small_view.canvas.height..small_view.canvas.height + small_view.footer.height)
                .contains(&(node.rect.y + 1))
        })
        .expect("the fixture is taller than a 120x30 canvas");
    let cell = (
        small_view.canvas.x + behind.rect.x + behind.rect.width / 2,
        small_view.canvas.y + behind.rect.y + 1,
    );
    assert!(
        small_view.footer.contains(cell.into()),
        "{cell:?} is on the panel"
    );
    let selected = small.tree.selected.clone();
    assert!(small.on_click(cell.0, cell.1, &small_layout).is_empty());
    assert_eq!(small.tree.selected, selected, "the panel is not the canvas");

    app.inspector_visible = false;
    let line = overview::view(&app, layout.main);
    assert_eq!(
        line.canvas.height,
        view.canvas.height + INSPECTOR_HEIGHT - 1,
        "the canvas pays exactly the panel's height, less the line it replaced"
    );
    app.set_graph_viewport(layout.main);
    assert_eq!(app.graph_area, line.canvas);
}

/// Milestone 4.6's guard, with the panel in the way: every degenerate size
/// either side of the collapse threshold, with the inspector both ways, drawn
/// and clicked and dragged and scrolled.
#[test]
fn no_panic_at_degenerate_sizes() {
    for inspector_visible in [false, true] {
        let (mut app, _) = opened_at(200, 50);
        app.inspector_visible = inspector_visible;
        for width in [1, 2, 3, 4, 12, 40, 61] {
            for height in 0..=(MIN_INTERIOR_FOR_PANEL + 4) {
                let layout = crate::ui::layout(Rect::new(0, 0, width, height), app.sidebar_width);
                app.set_tree_viewports(layout.sidebar_list.height, layout.main_inner.height);
                app.set_graph_viewport(layout.main);
                let view = overview::view(&app, layout.main);
                assert!(
                    view.footer.height != INSPECTOR_HEIGHT
                        || view.canvas.height >= MIN_INTERIOR_FOR_PANEL - INSPECTOR_HEIGHT,
                    "{width}x{height}: the panel left the canvas {} rows",
                    view.canvas.height
                );
                assert_eq!(
                    view.canvas.height + view.footer.height,
                    layout.main_inner.height,
                    "{width}x{height}: the split lost a row"
                );
                drawn(&app, width, height);
                for (x, y) in [
                    (0, 0),
                    (width / 2, height / 2),
                    (width - 1, height.saturating_sub(1)),
                ] {
                    app.on_click(x, y, &layout);
                    app.on_drag(x.saturating_sub(3), y.saturating_add(2), &layout);
                    app.on_scroll(true, x, y, &layout);
                    app.on_scroll(false, x, y, &layout);
                }
                // A double click on a node leaves the overview; reopen it so
                // the next size is drawn as a graph and not a terminal.
                if !app.overview {
                    assert!(toggle(&mut app).is_empty());
                }
            }
        }
    }
}
