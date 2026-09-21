//! Painting is asserted by exact strings (decision 9): every test builds a
//! tiny app, paints it, and compares the rendered rows character for
//! character, so a border landing one column off shows up as a diff rather
//! than as a property several wrong outputs could satisfy.

use super::*;
use crate::app::App;
use crate::graph::layout;
use crate::keymap::Keymap;
use crate::tree::NodeKey;
use proto::{Runtime, Status, SubagentInfo, SubagentState, WindowInfo};
use ratatui::style::Modifier;
use ratatui::text::Line;

#[path = "tests/edges.rs"]
mod edges;

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

/// Hangs one finished sub-agent per `kind` under `window`. `Done` keeps the
/// glyph off the spinner, so the painted string does not depend on the frame.
fn with_subagents(mut window: WindowInfo, kinds: &[&str]) -> WindowInfo {
    window.subagents = kinds
        .iter()
        .map(|kind| SubagentInfo {
            id: format!("{}-{kind}", window.id),
            parent_id: None,
            kind: (*kind).to_owned(),
            label: None,
            model: None,
            state: SubagentState::Done,
            tool: None,
            started_secs: 0,
            ended_secs: Some(0),
            needs_permission: false,
        })
        .collect();
    window
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
    // viewport's right edge, so only the project's box can appear. Its right
    // border is a `├` all the same: the window hangs off it, and the edge is
    // painted whether or not the child it runs to is in view (decision 13).
    let lines = paint(&layout, Rect::new(0, 0, 12, 3), Pan::default(), &rows, &app);

    assert_eq!(
        lines_text(&lines),
        vec!["╭──────────╮", "│ ○ shop   ├", "╰──────────╯",]
    );
}

#[test]
fn content_is_glyph_position_then_name() {
    let app = app_with(vec![window(1, "/r/shop", "worker", Status::Idle)]);
    let rows = app.rows();
    let layout = layout(&rows);
    let window_rect = layout.node(&NodeKey::Window(1)).unwrap().rect;

    // Pan to the window tier's own left edge so the project's box, well to
    // its left, falls entirely outside this viewport. The window's left
    // border is the far end of that project's edge, so it reads `┤`.
    let lines = paint(
        &layout,
        Rect::new(0, 0, window_rect.width, window_rect.height),
        Pan {
            x: window_rect.x,
            y: 0,
        },
        &rows,
        &app,
    );

    assert_eq!(
        lines_text(&lines),
        vec!["╭────────────╮", "┤ ○ 1 worker │", "╰────────────╯",]
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

    let lines = paint(&layout, Rect::new(0, 0, 30, 3), Pan::default(), &rows, &app);

    assert_eq!(
        lines_text(&lines),
        vec![
            "╭────────────────────────────╮",
            "│ ○ a-project-name-far-past… ├",
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

    let lines = paint(&layout, Rect::new(0, 0, 24, 3), Pan::default(), &rows, &app);

    assert_eq!(
        lines_text(&lines),
        vec![
            "╭──────────────────────╮",
            "│ ○ 日本語プロジェクト ├",
            "╰──────────────────────╯",
        ]
    );
}

/// A wide grapheme is two slots — the grapheme, then an empty continuation —
/// and either edge of the viewport can fall between them: the pan can clip the
/// grapheme and keep the continuation, and the right edge can keep the
/// grapheme and clip the continuation. Left unhandled the first leaves the
/// line one column short, which ratatui left-aligns, shifting every border to
/// its right out of line with the rows above and below; the second leaves it
/// one column long.
#[test]
fn a_wide_grapheme_cut_by_either_edge_keeps_the_line_the_areas_width() {
    let name = "日本語プロジェクト";
    let app = app_with(vec![window(1, &format!("/r/{name}"), "w", Status::Idle)]);
    let rows = app.rows();
    let layout = layout(&rows);
    // Every horizontal drag position at every width, not one chosen pair:
    // roughly half of them put one edge or the other inside a grapheme.
    for width in 1..=layout.size.0 {
        for pan_x in 0..=layout.size.0 {
            let area = Rect::new(0, 0, width, 3);
            let lines = paint(&layout, area, Pan { x: pan_x, y: 0 }, &rows, &app);
            for (index, text) in lines_text(&lines).iter().enumerate() {
                assert_eq!(
                    UnicodeWidthStr::width(text.as_str()),
                    usize::from(width),
                    "line {index} at pan.x {pan_x} in a {width}-column area: {text:?}"
                );
            }
        }
    }

    // The two cases spelled out. The project's content row is
    // `│ ○ 日本語プロジェクト │`, so its first `日` covers canvas columns 4
    // and 5. A pan of 5 clips that grapheme and leaves its continuation
    // leading the line, and `ロ` at the far end loses its continuation to the
    // right edge: one space each, and eight columns of line.
    let clipped_head = paint(
        &layout,
        Rect::new(0, 0, 8, 3),
        Pan { x: 5, y: 0 },
        &rows,
        &app,
    );
    assert_eq!(lines_text(&clipped_head)[1], " 本語プ ");
    // Three columns from column 2: the glyph, a space, and a `日` whose own
    // continuation is past the right edge.
    let clipped_tail = paint(
        &layout,
        Rect::new(0, 0, 3, 3),
        Pan { x: 2, y: 0 },
        &rows,
        &app,
    );
    assert_eq!(lines_text(&clipped_tail)[1], "○  ");
}

#[test]
fn the_viewport_shows_only_its_window_of_the_canvas() {
    // One project, two windows: the project (tier 0, x 0) centres at y 2
    // between its two children (tier 1, x 15, at y 0 and y 4) — the same
    // arithmetic `a_parent_centres_on_its_children` already pins for a
    // two-child span. Panning to (10, 2) cuts across all three boxes: the
    // tail of the project's box, the bus in the gap, and the head of both
    // windows'.
    let app = app_with(vec![
        window(1, "/r/p", "a", Status::Idle),
        window(2, "/r/p", "b", Status::Idle),
    ]);
    let rows = app.rows();
    let layout = layout(&rows);

    let lines = paint(
        &layout,
        Rect::new(0, 0, 8, 5),
        Pan { x: 10, y: 2 },
        &rows,
        &app,
    );

    assert_eq!(
        lines_text(&lines),
        vec!["─╮ │ ╰──", " ├─┤    ", "─╯ │ ╭──", "   └─┤ ○", "     ╰──",]
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

    let lines = paint(
        &layout,
        Rect::new(0, 0, 10, 5),
        Pan { x: 5, y: 1 },
        &rows,
        &app,
    );

    assert_eq!(
        lines_text(&lines),
        vec![
            "hop   ├───",
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

    let unfocused = paint(&layout, area, pan, &app.rows(), &app);
    assert_eq!(unfocused[0].spans.len(), 1, "one uniform border run");
    assert_eq!(unfocused[0].spans[0].style, crate::theme::border());

    app.focused = Some(1);
    let focused = paint(&layout, area, pan, &app.rows(), &app);
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

    let plain = paint(&layout, area, pan, &app.rows(), &app);
    assert!(
        !plain[0].spans[0]
            .style
            .add_modifier
            .contains(Modifier::REVERSED)
    );

    app.enter_tree();
    app.tree.selected = Some(NodeKey::Window(1));
    let highlighted = paint(&layout, area, pan, &app.rows(), &app);
    assert_eq!(highlighted[0].spans.len(), 1);
    assert!(
        highlighted[0].spans[0]
            .style
            .add_modifier
            .contains(Modifier::REVERSED)
    );
}
