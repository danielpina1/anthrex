//! Painting is asserted by exact strings (decision 9): every test builds a
//! tiny app, paints it, and compares the rendered rows character for
//! character, so a border landing one column off shows up as a diff rather
//! than as a property several wrong outputs could satisfy.

use super::*;
use crate::app::App;
use crate::graph::layout;
use crate::keymap::Keymap;
use crate::tree::NodeKey;
use proto::{Runtime, Status, WindowInfo};
use ratatui::style::Modifier;
use ratatui::text::Line;

fn window(id: u32, project: &str, name: &str, status: Status) -> WindowInfo {
    WindowInfo {
        id,
        name: name.to_owned(),
        runtime: Runtime::Claude,
        cwd: project.into(),
        project: project.into(),
        worktree: None,
        branch: None,
        status,
        tool: None,
        since_secs: 0,
        last_output_secs: 0,
        session_id: None,
        model: None,
        subagents: vec![],
        exit: None,
    }
}

fn app_with(windows: Vec<WindowInfo>) -> App {
    App::new(windows, "/tmp".into(), Keymap::default_prefix())
}

/// The plain-text content of every span on every line, joined per line —
/// what a reader watching the terminal would see, with no style attached.
fn lines_text(lines: &[Line<'_>]) -> Vec<String> {
    lines
        .iter()
        .map(|line| {
            line.spans
                .iter()
                .map(|span| span.content.as_ref())
                .collect()
        })
        .collect()
}

#[test]
fn one_box_paints_its_borders_and_content() {
    // "shop" (4 cols) + the glyph and its space (2) = 6, plus borders and
    // padding (4) = 10, held at the MIN_NODE_WIDTH floor of 12.
    let app = app_with(vec![window(1, "/r/shop", "worker", Status::Idle)]);
    let rows = app.rows();
    let layout = layout(&rows);

    // The window tier starts at x = 12 + TIER_GAP(3) = 15, past this
    // viewport's right edge, so only the project's box can appear.
    let lines = paint(&layout, Rect::new(0, 0, 12, 3), Pan::default(), &app);

    assert_eq!(
        lines_text(&lines),
        vec!["╭──────────╮", "│ ○ shop   │", "╰──────────╯",]
    );
}

#[test]
fn content_is_glyph_position_then_name() {
    let app = app_with(vec![window(1, "/r/shop", "worker", Status::Idle)]);
    let rows = app.rows();
    let layout = layout(&rows);
    let window_rect = layout.node(&NodeKey::Window(1)).unwrap().rect;

    // Pan to the window tier's own left edge so the project's box, well to
    // its left, falls entirely outside this viewport.
    let lines = paint(
        &layout,
        Rect::new(0, 0, window_rect.width, window_rect.height),
        Pan {
            x: window_rect.x,
            y: 0,
        },
        &app,
    );

    assert_eq!(
        lines_text(&lines),
        vec!["╭────────────╮", "│ ○ 1 worker │", "╰────────────╯",]
    );
}

#[test]
fn a_long_label_is_truncated_with_an_ellipsis() {
    // 45 columns of name want 51; MAX_NODE_WIDTH caps the tier at 30, so the
    // painter — not the layout — has to shorten what it draws. If the
    // truncation math were off by even one column, the right border below
    // would land on the wrong column and this literal would not match.
    let name = "a-project-name-far-past-the-thirty-column-cap";
    let app = app_with(vec![window(1, &format!("/r/{name}"), "w", Status::Idle)]);
    let rows = app.rows();
    let layout = layout(&rows);

    let lines = paint(&layout, Rect::new(0, 0, 30, 3), Pan::default(), &app);

    assert_eq!(
        lines_text(&lines),
        vec![
            "╭────────────────────────────╮",
            "│ ○ a-project-name-far-past… │",
            "╰────────────────────────────╯",
        ]
    );
}

#[test]
fn a_wide_character_label_keeps_the_border_aligned() {
    // Nine CJK characters are eighteen columns (decision 10); counting them
    // as nine would leave the right border nine columns short of here.
    let name = "日本語プロジェクト";
    let app = app_with(vec![window(1, &format!("/r/{name}"), "w", Status::Idle)]);
    let rows = app.rows();
    let layout = layout(&rows);

    let lines = paint(&layout, Rect::new(0, 0, 24, 3), Pan::default(), &app);

    assert_eq!(
        lines_text(&lines),
        vec![
            "╭──────────────────────╮",
            "│ ○ 日本語プロジェクト │",
            "╰──────────────────────╯",
        ]
    );
}

#[test]
fn the_viewport_shows_only_its_window_of_the_canvas() {
    // One project, two windows: the project (tier 0, x 0) centres at y 2
    // between its two children (tier 1, x 15, at y 0 and y 4) — the same
    // arithmetic `a_parent_centres_on_its_children` already pins for a
    // two-child span. Panning to (10, 2) cuts across all three boxes: the
    // tail of the project's box, the gap, and the head of both windows'.
    let app = app_with(vec![
        window(1, "/r/p", "a", Status::Idle),
        window(2, "/r/p", "b", Status::Idle),
    ]);
    let rows = app.rows();
    let layout = layout(&rows);

    let lines = paint(&layout, Rect::new(0, 0, 8, 5), Pan { x: 10, y: 2 }, &app);

    assert_eq!(
        lines_text(&lines),
        vec!["─╮   ╰──", " │      ", "─╯   ╭──", "     │ ○", "     ╰──",]
    );
}

#[test]
fn a_box_partly_outside_the_viewport_is_clipped_not_dropped() {
    // The project's box sits at the canvas origin; panning to (5, 1) puts
    // its top-left corner above and to the left of the viewport. The box
    // must still show what falls inside — its lower-right portion — not
    // disappear because its own origin is out of view.
    let app = app_with(vec![window(1, "/r/shop", "worker", Status::Idle)]);
    let rows = app.rows();
    let layout = layout(&rows);

    let lines = paint(&layout, Rect::new(0, 0, 10, 5), Pan { x: 5, y: 1 }, &app);

    assert_eq!(
        lines_text(&lines),
        vec![
            "hop   │   ",
            "──────╯   ",
            "          ",
            "          ",
            "          ",
        ]
    );
}

#[test]
fn the_focused_windows_box_uses_the_focused_border_style() {
    let mut app = app_with(vec![window(1, "/r/shop", "worker", Status::Idle)]);
    let rows = app.rows();
    let layout = layout(&rows);
    let rect = layout.node(&NodeKey::Window(1)).unwrap().rect;
    let area = Rect::new(0, 0, rect.width, rect.height);
    let pan = Pan { x: rect.x, y: 0 };

    let unfocused = paint(&layout, area, pan, &app);
    assert_eq!(unfocused[0].spans.len(), 1, "one uniform border run");
    assert_eq!(unfocused[0].spans[0].style, crate::theme::border());

    app.focused = Some(1);
    let focused = paint(&layout, area, pan, &app);
    assert_eq!(focused[0].spans.len(), 1);
    assert_eq!(focused[0].spans[0].style, crate::theme::border_focused());
}

#[test]
fn the_selected_node_is_highlighted() {
    let mut app = app_with(vec![window(1, "/r/shop", "worker", Status::Idle)]);
    let rows = app.rows();
    let layout = layout(&rows);
    let rect = layout.node(&NodeKey::Window(1)).unwrap().rect;
    let area = Rect::new(0, 0, rect.width, rect.height);
    let pan = Pan { x: rect.x, y: 0 };

    let plain = paint(&layout, area, pan, &app);
    assert!(
        !plain[0].spans[0]
            .style
            .add_modifier
            .contains(Modifier::REVERSED)
    );

    app.enter_tree();
    app.tree.selected = Some(NodeKey::Window(1));
    let highlighted = paint(&layout, area, pan, &app);
    assert_eq!(highlighted[0].spans.len(), 1);
    assert!(
        highlighted[0].spans[0]
            .style
            .add_modifier
            .contains(Modifier::REVERSED)
    );
}
