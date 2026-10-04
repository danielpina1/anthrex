//! M8c.5: run nodes on the canvas — their text, live glyphs, the critical path, the
//! dependency highlight and the dimming of finished and unrelated nodes
//! (decisions 16–20, Interfaces "Glyphs" and "Canvases, exact").

use super::*;
use crate::graph::Layout;
use crate::theme;
use crate::tree::run_fixtures::{
    RUN_ID, gate_fixture, snapshot, task, three_task_fixture, two_hundred_task_run,
};
use crate::tree::{Row, RunFilter, run_rows};
use proto::{
    AgentRole, AgentRoundInfo, DaemonMsg, RunReply, RunState, RunsSnapshot, Size, TaskState,
};
use ratatui::style::Style;
use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthStr;

mod glyphs;
mod patterns;
mod stages;

fn app_of((snapshot, windows): (RunsSnapshot, Vec<WindowInfo>)) -> App {
    let mut app = app_with(windows);
    app.on_daemon(DaemonMsg::Run(RunReply::Snapshot(snapshot)));
    app
}

/// The run view's rows of the snapshot's first run, unfiltered.
fn view_rows(app: &App) -> Vec<Row<'_>> {
    run_rows(&app.runs.runs[0], &app.windows, &app.tree, RunFilter::All)
}

/// The whole run view, painted at the canvas's own size with no pan.
fn paint_view(app: &App) -> (Layout, Vec<Line<'static>>) {
    let rows = view_rows(app);
    let layout = layout(&rows);
    let area = Rect::new(0, 0, layout.size.0, layout.size.1);
    let lines = paint(&layout, area, Pan::default(), &rows, app);
    (layout, lines)
}

/// Expected canvas lines, each padded with spaces to the canvas width.
fn padded(lines: &[&str], width: usize) -> Vec<String> {
    lines.iter().map(|line| format!("{line:<width$}")).collect()
}

/// The grapheme and style of the cell at a column, walking spans by display width.
fn cell_at(lines: &[Line<'_>], x: u16, y: u16) -> (String, Style) {
    let mut column = 0;
    for span in &lines[usize::from(y)].spans {
        for grapheme in span.content.graphemes(true) {
            let width = UnicodeWidthStr::width(grapheme);
            if usize::from(x) >= column && usize::from(x) < column + width.max(1) {
                return (grapheme.to_owned(), span.style);
            }
            column += width;
        }
    }
    panic!("no cell at ({x}, {y})");
}

fn style_at(lines: &[Line<'_>], x: u16, y: u16) -> Style {
    cell_at(lines, x, y).1
}

fn border_cells(rect: Rect) -> Vec<(u16, u16)> {
    let right = rect.x + rect.width - 1;
    let mut cells: Vec<(u16, u16)> = (rect.x..=right)
        .flat_map(|x| [(x, rect.y), (x, rect.y + 2)])
        .collect();
    cells.extend([(rect.x, rect.y + 1), (right, rect.y + 1)]);
    cells
}

fn all_cells(rect: Rect) -> Vec<(u16, u16)> {
    (rect.y..rect.y + rect.height)
        .flat_map(|y| (rect.x..rect.x + rect.width).map(move |x| (x, y)))
        .collect()
}

fn task_key(id: &str) -> NodeKey {
    NodeKey::Task {
        run: RUN_ID.into(),
        id: id.into(),
    }
}

fn round_key(task: &str, role: AgentRole, session: u32, round: u32) -> NodeKey {
    NodeKey::AgentRound {
        run: RUN_ID.into(),
        task: task.into(),
        role,
        session,
        round,
    }
}

fn rect_of(layout: &Layout, key: &NodeKey) -> Rect {
    layout
        .node(key)
        .unwrap_or_else(|| panic!("{key:?} placed"))
        .rect
}

/// The status glyph's cell: after the left border and one space of padding.
fn glyph_of(lines: &[Line<'_>], rect: Rect) -> (String, Style) {
    cell_at(lines, rect.x + 2, rect.y + 1)
}

fn ended(mut round: AgentRoundInfo, at: u64) -> AgentRoundInfo {
    round.ended_at = Some(at);
    round
}

#[test]
fn project_overview_draws_the_run_node() {
    let app = app_of(gate_fixture());
    let rows = app.rows();
    let layout = layout(&rows);
    let lines = paint(&layout, Rect::new(0, 0, 34, 7), Pan::default(), &rows, &app);
    assert_eq!(
        lines_text(&lines),
        padded(
            &[
                "               ╭─────────────────╮",
                "             ┌─┤ ⚑ run 3f9a  0/2 │",
                "╭──────────╮ │ ╰─────────────────╯",
                "│ ⚑ demo   ├─┤",
                "╰──────────╯ │ ╭─────────────────╮",
                "             └─┤ ○ 1 shell       │",
                "               ╰─────────────────╯",
            ],
            34
        )
    );
}

#[test]
fn run_view_draws_tiers_rounds_and_glyphs() {
    let app = app_of(three_task_fixture());
    assert_eq!(app.spinner_frame, 0);
    let rows = view_rows(&app);
    let layout = layout(&rows);
    let lines = paint(
        &layout,
        Rect::new(0, 0, 78, 15),
        Pan::default(),
        &rows,
        &app,
    );
    assert_eq!(
        lines_text(&lines),
        padded(
            &[
                "                                                        ╭────────────────────╮",
                "                                                      ┌─┤ ✓ worker #1 claude │",
                "                          ╭─────────────────────────╮ │ ╰────────────────────╯",
                "                        ┌─┤ ✓ t0 proto M ◆          ├─┤",
                "                        │ ╰─────────────────────────╯ │ ╭────────────────────╮",
                "                        │                             └─┤ ✓ review #1 codex  │",
                "                        │                               ╰────────────────────╯",
                "╭─────────────────────╮ │",
                "│ ◉ orchestrator  1/3 ├─┤ ╭─────────────────────────╮   ╭────────────────────╮",
                "╰─────────────────────╯ ├─┤ ● t1 spawn M  after t0  ├───┤ ● worker #1 claude │",
                "                        │ ╰─────────────────────────╯   ╰────────────────────╯",
                "                        │",
                "                        │ ╭─────────────────────────╮",
                "                        └─┤ ▫ t2 status S  after t0 │",
                "                          ╰─────────────────────────╯",
            ],
            78
        )
    );
}

#[test]
fn the_gate_draws_every_task_as_planned() {
    // A task the gate shows in another state still reads as planned, in the idle
    // colour (a pending task's own colour is the same grey, so t2 is blocked here).
    let (mut snap, windows) = gate_fixture();
    snap.runs[0].tasks[1].state = TaskState::Blocked;
    let app = app_of((snap, windows));
    let (layout, lines) = paint_view(&app);
    for id in ["t1", "t2"] {
        let (glyph, style) = glyph_of(&lines, rect_of(&layout, &task_key(id)));
        assert_eq!(glyph, "○", "{id} is drawn as planned at the gate");
        assert_eq!(style.fg, Some(theme::fg(theme::Role::Muted)));
    }

    // The same pending task outside the gate is `◌`.
    let (mut snap, windows) = gate_fixture();
    snap.runs[0].state = RunState::Running;
    let app = app_of((snap, windows));
    let (layout, lines) = paint_view(&app);
    let (glyph, style) = glyph_of(&lines, rect_of(&layout, &task_key("t1")));
    assert_eq!(glyph, "◌");
    assert_eq!(style.fg, Some(theme::fg(theme::Role::Muted)));
}

#[test]
fn a_working_task_animates_only_while_its_worker_works() {
    let worker_key = round_key("t1", AgentRole::Worker, 1, 1);
    let (snap, mut windows) = three_task_fixture();
    windows[2].status = proto::Status::Working;
    let mut app = app_of((snap, windows));
    app.spinner_frame = 3;
    let (layout, lines) = paint_view(&app);
    let task = glyph_of(&lines, rect_of(&layout, &task_key("t1")));
    assert_eq!(task.0, theme::SPINNER[3]);
    assert_eq!(task.1.fg, Some(theme::fg(theme::Role::Working)));
    assert_eq!(
        glyph_of(&lines, rect_of(&layout, &worker_key)).0,
        theme::SPINNER[3]
    );

    let mut app = app_of(three_task_fixture());
    app.spinner_frame = 3;
    let (layout, lines) = paint_view(&app);
    assert_eq!(glyph_of(&lines, rect_of(&layout, &task_key("t1"))).0, "●");
    assert_eq!(glyph_of(&lines, rect_of(&layout, &worker_key)).0, "●");

    // A `Working` window whose worker round has ended does not animate the task.
    let (mut snap, mut windows) = three_task_fixture();
    windows[2].status = proto::Status::Working;
    snap.runs[0].tasks[1].rounds[0].ended_at = Some(9_990);
    let app = app_of((snap, windows));
    let (layout, lines) = paint_view(&app);
    assert_eq!(glyph_of(&lines, rect_of(&layout, &task_key("t1"))).0, "●");
}

#[test]
fn critical_path_tasks_have_a_bold_border() {
    let app = app_of(three_task_fixture());
    let (layout, lines) = paint_view(&app);
    let t1 = rect_of(&layout, &task_key("t1"));
    for (x, y) in border_cells(t1) {
        assert!(
            style_at(&lines, x, y).add_modifier.contains(Modifier::BOLD),
            "t1's border cell ({x}, {y}) is bold"
        );
    }
    // The border only: the text inside stays regular.
    assert!(
        !style_at(&lines, t1.x + 4, t1.y + 1)
            .add_modifier
            .contains(Modifier::BOLD)
    );
    let t2 = rect_of(&layout, &task_key("t2"));
    for (x, y) in border_cells(t2) {
        assert!(
            !style_at(&lines, x, y).add_modifier.contains(Modifier::BOLD),
            "t2's border cell ({x}, {y}) is not bold"
        );
    }
}

#[test]
fn selecting_a_task_lights_its_dependencies_and_dims_the_rest() {
    let mut app = app_of(three_task_fixture());
    // The overview, where the graph is drawn and holds the keys (decision 1).
    app.enter_overview();
    app.tree.selected = Some(task_key("t1"));
    let (layout, lines) = paint_view(&app);
    // Milestone 9.0.7 decision 1: a lit box is bold on the muted border, never accented.
    let focused = theme::role(theme::Role::Muted, app.palette()).add_modifier(Modifier::BOLD);

    for (x, y) in border_cells(rect_of(&layout, &task_key("t0"))) {
        assert_eq!(style_at(&lines, x, y), focused, "t0 border ({x}, {y})");
    }
    for key in [task_key("t2"), NodeKey::Run(RUN_ID.into())] {
        for (x, y) in all_cells(rect_of(&layout, &key)) {
            assert!(
                style_at(&lines, x, y).add_modifier.contains(Modifier::DIM),
                "{key:?} cell ({x}, {y}) is dim"
            );
        }
    }
    // Its own round is another node too.
    let own = rect_of(&layout, &round_key("t1", AgentRole::Worker, 1, 1));
    assert!(
        style_at(&lines, own.x + 4, own.y + 1)
            .add_modifier
            .contains(Modifier::DIM)
    );
    // Milestone 9.0.7 decision 20: the selection is reversed, but for the bar that
    // takes its left border cell on the content row, in the accent.
    let selected = rect_of(&layout, &task_key("t1"));
    let bar = (selected.x, selected.y + 1);
    assert_eq!(cell_at(&lines, bar.0, bar.1).0, "▌");
    assert_eq!(
        style_at(&lines, bar.0, bar.1),
        theme::role(theme::Role::Accent, app.palette())
    );
    for (x, y) in all_cells(selected).into_iter().filter(|cell| *cell != bar) {
        let style = style_at(&lines, x, y);
        assert!(style.add_modifier.contains(Modifier::REVERSED));
        assert!(!style.add_modifier.contains(Modifier::DIM));
    }
    for (x, y) in all_cells(rect_of(&layout, &task_key("t0"))) {
        assert!(!style_at(&lines, x, y).add_modifier.contains(Modifier::DIM));
    }

    // Selecting t0 lights its dependents, t1 (bold, on the critical path) and t2.
    app.tree.selected = Some(task_key("t0"));
    let (layout, lines) = paint_view(&app);
    let t1 = rect_of(&layout, &task_key("t1"));
    assert_eq!(
        style_at(&lines, t1.x, t1.y),
        focused.add_modifier(Modifier::BOLD)
    );
    let t2 = rect_of(&layout, &task_key("t2"));
    assert_eq!(style_at(&lines, t2.x, t2.y), focused);
    assert!(
        !style_at(&lines, t2.x + 4, t2.y + 1)
            .add_modifier
            .contains(Modifier::DIM)
    );

    // An implicit dependency lights too.
    let (mut snap, windows) = three_task_fixture();
    snap.runs[0].tasks[2].implicit_deps = vec!["t1".into()];
    let mut app = app_of((snap, windows));
    app.enter_tree();
    app.tree.selected = Some(task_key("t2"));
    let (layout, lines) = paint_view(&app);
    let t1 = rect_of(&layout, &task_key("t1"));
    assert_eq!(
        style_at(&lines, t1.x, t1.y),
        focused.add_modifier(Modifier::BOLD)
    );

    // Nothing is dimmed while the selection is not a task, or not shown.
    let mut app = app_of(three_task_fixture());
    app.enter_tree();
    app.tree.selected = Some(NodeKey::Run(RUN_ID.into()));
    let (layout, lines) = paint_view(&app);
    let t2 = rect_of(&layout, &task_key("t2"));
    assert!(
        !style_at(&lines, t2.x, t2.y)
            .add_modifier
            .contains(Modifier::DIM)
    );
    let mut app = app_of(three_task_fixture());
    app.tree.selected = Some(task_key("t1"));
    let (layout, lines) = paint_view(&app);
    let t2 = rect_of(&layout, &task_key("t2"));
    assert_eq!(
        style_at(&lines, t2.x, t2.y),
        theme::role(theme::Role::Muted, theme::Palette::PLAIN)
    );
}

#[test]
fn finished_agent_nodes_are_dim() {
    let app = app_of(three_task_fixture());
    let (layout, lines) = paint_view(&app);
    for key in [
        round_key("t0", AgentRole::Worker, 1, 1),
        round_key("t0", AgentRole::Reviewer, 1, 1),
    ] {
        for (x, y) in all_cells(rect_of(&layout, &key)) {
            assert!(
                style_at(&lines, x, y).add_modifier.contains(Modifier::DIM),
                "{key:?} cell ({x}, {y}) is dim"
            );
        }
    }
    let live = rect_of(&layout, &round_key("t1", AgentRole::Worker, 1, 1));
    for (x, y) in all_cells(live) {
        assert!(!style_at(&lines, x, y).add_modifier.contains(Modifier::DIM));
    }
    // A finished task is not an agent node: merged t0 is not dim.
    let t0 = rect_of(&layout, &task_key("t0"));
    assert!(
        !style_at(&lines, t0.x + 4, t0.y + 1)
            .add_modifier
            .contains(Modifier::DIM)
    );
}

#[test]
fn a_row_that_disagrees_with_its_node_is_skipped() {
    let app = app_of(three_task_fixture());
    let rows = view_rows(&app);
    let layout = layout(&rows);
    let mut disagreeing = rows.clone();
    disagreeing[1].key = NodeKey::Window(999);
    let area = Rect::new(0, 0, layout.size.0, layout.size.1);
    let lines = paint(&layout, area, Pan::default(), &disagreeing, &app);

    let skipped = layout.nodes[1].rect;
    for (x, y) in all_cells(skipped) {
        assert_eq!(cell_at(&lines, x, y).0, " ", "cell ({x}, {y}) is blank");
    }
    // Every other node is still painted.
    let text = lines_text(&lines).join("\n");
    assert!(text.contains("orchestrator  1/3"), "{text}");
    assert!(text.contains("t1 spawn M  after t0"), "{text}");

    // Skip two of the root's three children: the edge is not drawn at all, so no
    // junction lands on the root's border (the M8c.5 review's M1).
    let index = |key: &NodeKey| rows.iter().position(|row| &row.key == key).unwrap();
    let mut two_skipped = rows.clone();
    for id in ["t0", "t2"] {
        two_skipped[index(&task_key(id))].key = NodeKey::Window(999);
    }
    let lines = paint(&layout, area, Pan::default(), &two_skipped, &app);
    let edge = layout
        .edges
        .iter()
        .find(|edge| edge.children.contains(&task_key("t1")))
        .expect("the root's edge");
    let root = rect_of(&layout, &edge.parent);
    let right = root.x + root.width - 1;
    let bottom = root.y + root.height - 1;
    assert_eq!(cell_at(&lines, right, root.y).0, "╮");
    assert_eq!(cell_at(&lines, right, bottom).0, "╯", "root's bottom-right");
    for y in root.y + 1..bottom {
        assert_eq!(
            cell_at(&lines, right, y).0,
            "│",
            "root's right border at {y}"
        );
    }
}

#[test]
fn two_hundred_tasks_paint_without_overflow() {
    let now = 10_000;
    let app = app_of((snapshot(now, vec![two_hundred_task_run()]), vec![]));
    let rows = view_rows(&app);
    let layout = layout(&rows);
    assert_eq!(layout.nodes.len(), 801);
    assert_eq!(layout.size.1, 600 * 4 - 1);
    assert_eq!(layout.size.1, 2399);

    let last = rect_of(&layout, &task_key("t199"));
    let pan = Pan {
        x: 0,
        y: last.y.saturating_sub(20),
    };
    let lines = paint(&layout, Rect::new(0, 0, 120, 40), pan, &rows, &app);
    assert_eq!(lines.len(), 40);
    for line in &lines {
        assert_eq!(line.width(), 120);
    }
    let text = lines_text(&lines).join("\n");
    assert!(text.contains("◐ t199 work S"), "{text}");
}

/// Hostile snapshots: a wide title, an id longer than the box, fifty deps, an empty
/// title, a critical path and deps naming unknown tasks, and a selection that names
/// no row. Nothing panics and every border stays in its column.
#[test]
fn hostile_run_data_paints_in_line() {
    let (mut snap, windows) = three_task_fixture();
    let info = &mut snap.runs[0];
    info.critical_path = vec!["ghost".into(), "t0".into(), "nobody".into()];
    let mut wide = task(
        "t3",
        "漢字漢字漢字漢字漢字漢字😀",
        Size::M,
        TaskState::Queued,
    );
    wide.deps = vec!["ghost".into()];
    let long_id = task(
        "t-an-extremely-long-task-identifier-0123456789",
        "x",
        Size::S,
        TaskState::Queued,
    );
    let mut fifty = task("t4", "fifty", Size::S, TaskState::Queued);
    fifty.deps = (0..50).map(|index| format!("t{index}")).collect();
    let empty = task("t5", "", Size::S, TaskState::Pending);
    info.tasks.extend([wide, long_id, fifty, empty]);

    let mut app = app_of((snap, windows));
    app.enter_tree();
    for selected in [task_key("t3"), task_key("t4"), task_key("ghost")] {
        app.tree.selected = Some(selected);
        let (layout, lines) = paint_view(&app);
        for line in &lines {
            assert_eq!(line.width(), usize::from(layout.size.0));
        }
        for node in &layout.nodes {
            assert!(node.rect.width <= crate::graph::MAX_NODE_WIDTH);
            let right = node.rect.x + node.rect.width - 1;
            let (top, _) = cell_at(&lines, right, node.rect.y);
            let (bottom, _) = cell_at(&lines, right, node.rect.y + 2);
            assert_eq!(
                (top.as_str(), bottom.as_str()),
                ("╮", "╯"),
                "{:?}",
                node.key
            );
            let (side, _) = cell_at(&lines, right, node.rect.y + 1);
            assert!(
                ["│", "├"].contains(&side.as_str()),
                "{:?}: {side}",
                node.key
            );
        }
        let text = lines_text(&lines).join("\n");
        assert!(text.contains("t5 S"), "{text}");
    }
}

#[test]
fn every_task_state_has_its_glyph_and_colour() {
    use theme::Role;
    // Milestone 9.0.7 decisions 3 and 4: the merge queue is `»` (`▸` means
    // collapsed only), and a blocked task no one else answers needs you.
    let cases = [
        (TaskState::Pending, "◌", Role::Muted),
        (TaskState::Queued, "▫", Role::Muted),
        (TaskState::Preparing, "●", Role::Working),
        (TaskState::Working, "●", Role::Working),
        (TaskState::Proof, "◇", Role::Working),
        (TaskState::Check, "◇", Role::Working),
        (TaskState::Review, "◐", Role::Working),
        (TaskState::MergeQueue, "»", Role::Working),
        (TaskState::Merged, "✓", Role::Done),
        (TaskState::Blocked, "⚑", Role::Attention),
        (TaskState::Cancelled, "–", Role::Muted),
    ];
    let look = |state, gate_open, animating| theme::TaskLook {
        state,
        gate_open,
        held: false,
        paused: false,
        animating,
        needs_you: state == TaskState::Blocked,
    };
    for (state, glyph, role) in cases {
        assert_eq!(
            theme::task_look(look(state, false, false), 0, false),
            (glyph, role),
            "{state:?}"
        );
        assert_eq!(
            theme::task_look(look(state, true, true), 0, false).0,
            "○",
            "{state:?} at the gate"
        );
    }
    assert_eq!(
        theme::task_look(look(TaskState::Working, false, true), 12, false).0,
        theme::SPINNER[2]
    );
    // Only `working` animates.
    assert_eq!(
        theme::task_look(look(TaskState::Preparing, false, true), 1, false).0,
        "●"
    );
}

/// Every task glyph is one column, like every status glyph (Risks 5).
#[test]
fn task_glyphs_are_one_column() {
    for state in [
        TaskState::Pending,
        TaskState::Queued,
        TaskState::Preparing,
        TaskState::Proof,
        TaskState::Review,
        TaskState::MergeQueue,
        TaskState::Merged,
        TaskState::Blocked,
        TaskState::Cancelled,
    ] {
        let look = theme::TaskLook {
            state,
            gate_open: false,
            held: false,
            paused: false,
            animating: false,
            needs_you: false,
        };
        let glyph = theme::task_look(look, 0, false).0;
        assert_eq!(UnicodeWidthStr::width(glyph), 1, "{glyph}");
    }
    assert_eq!(
        UnicodeWidthStr::width(theme::glyph(theme::Glyph::Run, false)),
        1
    );
}
