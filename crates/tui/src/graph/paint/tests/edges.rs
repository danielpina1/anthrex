//! The edges between the boxes (decision 13): a straight run to one child,
//! a bus to several, every junction the two can meet in, and what survives
//! when the viewport cuts across them.
//!
//! Every tier here is twelve columns wide — the `MIN_NODE_WIDTH` floor — so
//! tier 0 owns columns 0..=11, the gap owns 12..=14 and tier 1 starts at 15.
//! The bus therefore sits in column 13, the middle of the gap, and every
//! expected string below can be read off column by column.
//!
//! A bus cell's glyph is the arms that meet in it, and the parent's run adds
//! the one pointing left when it arrives on that row. Each of the three arms
//! it can add has a test of its own: `┌` becomes `┬` at the top end, `├`
//! becomes `┼` at a child between the ends, and `└` becomes `┴` at the
//! bottom end.

use super::*;

#[test]
fn one_child_is_a_straight_run() {
    // One child centres the parent on it (decision 6), so the two middle rows
    // coincide and the edge is a horizontal line with no bus column at all.
    let app = app_with(vec![window(1, "/r/p", "w", Status::Idle)]);
    let rows = app.rows();
    let layout = layout(&rows);

    let lines = paint(&layout, Rect::new(0, 0, 27, 3), Pan::default(), &rows, &app);

    assert_eq!(
        lines_text(&lines),
        vec![
            "╭──────────╮   ╭──────────╮",
            "│ ○ p      ├───┤ ○ 1 w    │",
            "╰──────────╯   ╰──────────╯",
        ]
    );
}

#[test]
fn three_children_use_a_bus() {
    // `a` carries two sub-agents of its own, so it is centred on them at row
    // 2 while `b` and `c` take the rows below. That lifts the whole span and
    // leaves the project's own middle row — 8 — on none of its children's
    // rows, which is what keeps the middle junction a plain `├` rather than
    // the `┼` three leaf children would produce. The bus's own ends are
    // therefore `┌` and `└`: the parent's run arrives at neither, and reaches
    // the bus on row 8 as a `┤`.
    let app = app_with(vec![
        with_subagents(window(1, "/r/p", "a", Status::Idle), &["x", "y"]),
        window(2, "/r/p", "b", Status::Idle),
        window(3, "/r/p", "c", Status::Idle),
    ]);
    let rows = app.rows();
    let layout = layout(&rows);

    let lines = paint(
        &layout,
        Rect::new(0, 0, 42, 15),
        Pan::default(),
        &rows,
        &app,
    );

    assert_eq!(
        lines_text(&lines),
        vec![
            "                              ╭──────────╮",
            "                            ┌─┤ ✓ x      │",
            "               ╭──────────╮ │ ╰──────────╯",
            "             ┌─┤ ○ 1 a    ├─┤             ",
            "             │ ╰──────────╯ │ ╭──────────╮",
            "             │              └─┤ ✓ y      │",
            "             │                ╰──────────╯",
            "╭──────────╮ │                            ",
            "│ ○ p      ├─┤ ╭──────────╮               ",
            "╰──────────╯ ├─┤ ○ 2 b    │               ",
            "             │ ╰──────────╯               ",
            "             │                            ",
            "             │ ╭──────────╮               ",
            "             └─┤ ○ 3 c    │               ",
            "               ╰──────────╯               ",
        ]
    );
}

#[test]
fn the_parent_row_coinciding_with_the_bus_uses_a_cross() {
    // Three leaf children put the parent's middle row exactly on the middle
    // child's, so the bus, the child's run and the parent's run all meet in
    // one cell.
    let app = app_with(vec![
        window(1, "/r/p", "a", Status::Idle),
        window(2, "/r/p", "b", Status::Idle),
        window(3, "/r/p", "c", Status::Idle),
    ]);
    let rows = app.rows();
    let layout = layout(&rows);

    let lines = paint(
        &layout,
        Rect::new(0, 0, 27, 11),
        Pan::default(),
        &rows,
        &app,
    );

    assert_eq!(
        lines_text(&lines),
        vec![
            "               ╭──────────╮",
            "             ┌─┤ ○ 1 a    │",
            "             │ ╰──────────╯",
            "             │             ",
            "╭──────────╮ │ ╭──────────╮",
            "│ ○ p      ├─┼─┤ ○ 2 b    │",
            "╰──────────╯ │ ╰──────────╯",
            "             │             ",
            "             │ ╭──────────╮",
            "             └─┤ ○ 3 c    │",
            "               ╰──────────╯",
        ]
    );
}

#[test]
fn the_parent_row_coinciding_with_the_first_child_uses_a_tee() {
    // `layout` centres a parent on its children, so with two or more of them
    // the parent's row is always at least two rows below the bus's top: this
    // geometry cannot arise from `layout` today and the rectangle is moved by
    // hand. It is not dead weight — decision 6's centring rule is not a law of
    // nature, and this is what stops the glyph going wrong silently if it ever
    // changes. The top end's `┌` gains the arm the parent arrives on and
    // becomes a `┬`; everything below it is untouched.
    let app = app_with(vec![
        window(1, "/r/p", "a", Status::Idle),
        window(2, "/r/p", "b", Status::Idle),
        window(3, "/r/p", "c", Status::Idle),
    ]);
    let rows = app.rows();
    let mut layout = layout(&rows);
    let project = layout
        .nodes
        .iter_mut()
        .find(|node| node.depth == 0)
        .expect("the project is placed");
    project.rect.y = 0;

    let lines = paint(
        &layout,
        Rect::new(0, 0, 27, 11),
        Pan::default(),
        &rows,
        &app,
    );

    assert_eq!(
        lines_text(&lines),
        vec![
            "╭──────────╮   ╭──────────╮",
            "│ ○ p      ├─┬─┤ ○ 1 a    │",
            "╰──────────╯ │ ╰──────────╯",
            "             │             ",
            "             │ ╭──────────╮",
            "             ├─┤ ○ 2 b    │",
            "             │ ╰──────────╯",
            "             │             ",
            "             │ ╭──────────╮",
            "             └─┤ ○ 3 c    │",
            "               ╰──────────╯",
        ]
    );
}

#[test]
fn the_parent_row_coinciding_with_the_last_child_uses_an_upward_tee() {
    // The mirror of the test above, and unreachable from `layout` for the same
    // reason: the bottom end's `└` gains the arm the parent arrives on and
    // becomes a `┴`. Without this the corner would swallow the parent's run.
    let app = app_with(vec![
        window(1, "/r/p", "a", Status::Idle),
        window(2, "/r/p", "b", Status::Idle),
        window(3, "/r/p", "c", Status::Idle),
    ]);
    let rows = app.rows();
    let mut layout = layout(&rows);
    let project = layout
        .nodes
        .iter_mut()
        .find(|node| node.depth == 0)
        .expect("the project is placed");
    project.rect.y = 8;

    let lines = paint(
        &layout,
        Rect::new(0, 0, 27, 11),
        Pan::default(),
        &rows,
        &app,
    );

    assert_eq!(
        lines_text(&lines),
        vec![
            "               ╭──────────╮",
            "             ┌─┤ ○ 1 a    │",
            "             │ ╰──────────╯",
            "             │             ",
            "             │ ╭──────────╮",
            "             ├─┤ ○ 2 b    │",
            "             │ ╰──────────╯",
            "             │             ",
            "╭──────────╮ │ ╭──────────╮",
            "│ ○ p      ├─┴─┤ ○ 3 c    │",
            "╰──────────╯   ╰──────────╯",
        ]
    );
}

#[test]
fn a_border_an_edge_meets_becomes_a_junction() {
    // Two children: the parent's run leaves its right border at row 3 and
    // each child's run arrives at its left border at rows 1 and 5. Those
    // three cells are borders in the box painter's output and junctions in
    // this one, and each keeps the style of the box it belongs to — the
    // focused child's `┤` is drawn in the focused border style, not the plain
    // one the gap's own lines use.
    let mut app = app_with(vec![
        window(1, "/r/p", "a", Status::Idle),
        window(2, "/r/p", "b", Status::Idle),
    ]);
    app.focused = Some(1);
    let rows = app.rows();
    let layout = layout(&rows);

    let lines = paint(&layout, Rect::new(0, 0, 27, 7), Pan::default(), &rows, &app);
    let text = lines_text(&lines);

    assert_eq!(
        text,
        vec![
            "               ╭──────────╮",
            "             ┌─┤ ○ 1 a    │",
            "╭──────────╮ │ ╰──────────╯",
            "│ ○ p      ├─┤             ",
            "╰──────────╯ │ ╭──────────╮",
            "             └─┤ ○ 2 b    │",
            "               ╰──────────╯",
        ]
    );

    let cell = |row: usize, column: usize| text[row].chars().nth(column).expect("column is drawn");
    assert_eq!(cell(3, 11), '├', "the parent's right border");
    assert_eq!(cell(1, 15), '┤', "the first child's left border");
    assert_eq!(cell(5, 15), '┤', "the last child's left border");

    let junction = lines[1]
        .spans
        .iter()
        .find(|span| span.content.starts_with('┤'))
        .expect("the focused child's junction is its own span");
    assert_eq!(junction.style, crate::theme::border_focused());
}

#[test]
fn edges_are_clipped_with_the_viewport() {
    // The same two-child canvas, 27 x 7, seen through two viewports that cut
    // it on all four sides.
    let app = app_with(vec![
        window(1, "/r/p", "a", Status::Idle),
        window(2, "/r/p", "b", Status::Idle),
    ]);
    let rows = app.rows();
    let layout = layout(&rows);

    // Above and left of the parent's middle row: all that survives of the
    // parent's box is the last two columns of its top border, `─╮` on the
    // third line, and its own run into the bus is a row below the viewport,
    // so none of it shows. The part of the first child's edge that falls
    // inside does.
    let cut_top_left = paint(
        &layout,
        Rect::new(0, 0, 8, 3),
        Pan { x: 10, y: 0 },
        &rows,
        &app,
    );
    assert_eq!(
        lines_text(&cut_top_left),
        vec!["     ╭──", "   ┌─┤ ○", "─╮ │ ╰──"]
    );

    // Right and bottom: column 17 cuts both children's boxes in half, and row
    // 4 is the last drawn, so the last child's `└─┤` on row 5 and its bottom
    // border on row 6 are dropped rather than wrapped or panicked on.
    let cut_bottom_right = paint(
        &layout,
        Rect::new(0, 0, 16, 4),
        Pan { x: 2, y: 1 },
        &rows,
        &app,
    );
    assert_eq!(
        lines_text(&cut_bottom_right),
        vec![
            "           ┌─┤ ○",
            "─────────╮ │ ╰──",
            "○ p      ├─┤    ",
            "─────────╯ │ ╭──",
        ]
    );
}
