//! Opening and closing the graph overview, what a frame of it draws, and
//! the footer that spells the selected node out. The gestures over it are
//! in `overview/mouse.rs`.

use super::*;
use crate::graph::Pan;
use crate::tree::NodeKey;
use crate::ui::overview;
use ratatui::backend::TestBackend;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;

#[path = "overview/mouse.rs"]
mod mouse;

fn toggle(app: &mut App) -> Vec<Effect> {
    prefix(app);
    press(app, KeyCode::Char('T'), KeyModifiers::NONE)
}

/// Opens the overview and reports the layout the next frame will draw into,
/// with both viewports already set the way `lib::draw` sets them.
fn opened_at(width: u16, height: u16) -> (App, crate::ui::Layout) {
    let mut app = app_with(tree::example_windows());
    assert!(toggle(&mut app).is_empty());
    let layout = crate::ui::layout(Rect::new(0, 0, width, height), app.sidebar_width);
    app.set_tree_viewports(layout.sidebar_list.height, layout.main_inner.height);
    app.set_graph_viewport(layout.main);
    (app, layout)
}

fn opened() -> (App, crate::ui::Layout) {
    opened_at(120, 30)
}

fn assert_closed(app: &App) {
    assert!(!app.overview);
    assert_eq!(app.tree_input, None);
    assert!(!app.keymap.tree_mode());
    assert_eq!(app.tree.selected, None);
    assert!(app.tree.filter.is_empty());
}

fn select(app: &mut App, key: NodeKey) {
    let rows = tree::build(&app.windows, &app.tree);
    app.tree.select(&rows, key);
    app.reveal_tree_anchor();
}

/// The screen cell at the middle of `key`'s box, which must be on screen.
fn box_middle(app: &App, main: Rect, key: &NodeKey) -> (u16, u16) {
    let view = overview::view(app, main);
    let rect = view.layout.node(key).expect("a visible row is placed").rect;
    let cell = (
        view.canvas.x + (rect.x + rect.width / 2) - view.pan.x,
        view.canvas.y + (rect.y + 1) - view.pan.y,
    );
    assert!(
        view.canvas.contains(cell.into()),
        "{key:?} at {rect:?} is off screen under pan {:?}",
        view.pan
    );
    cell
}

fn drawn(app: &App, width: u16, height: u16) -> Buffer {
    let mut terminal = ratatui::Terminal::new(TestBackend::new(width, height)).unwrap();
    terminal
        .draw(|frame| {
            crate::ui::draw(frame, app);
        })
        .unwrap();
    terminal.backend().buffer().clone()
}

/// Every cell inside `rect`, rows joined by newlines.
fn text_in(buffer: &Buffer, rect: Rect) -> String {
    (rect.y..rect.bottom())
        .map(|y| {
            (rect.x..rect.right())
                .map(|x| buffer[(x, y)].symbol())
                .collect::<String>()
        })
        .collect::<Vec<_>>()
        .join("\n")
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
fn the_overview_draws_boxes() {
    // Wide enough that the whole canvas fits, so nothing is clipped away.
    let (app, layout) = opened_at(200, 50);
    let view = overview::view(&app, layout.main);
    assert_eq!(view.pan, Pan::default(), "the canvas fits: no pan");
    let buffer = drawn(&app, 200, 50);
    let canvas = text_in(&buffer, view.canvas);
    for glyph in ['╭', '╮', '╰', '╯'] {
        assert!(canvas.contains(glyph), "no {glyph} in:\n{canvas}");
    }
    // `┤` is where an edge meets a child's left border. The sidebar's guides
    // draw `├`, `│` and `└`, so only the graph can have produced this one.
    assert!(canvas.contains('┤'), "no edge junction in:\n{canvas}");
    assert!(canvas.contains("1 api-worker"), "{canvas}");
    // The aligned columns are gone from the rows; the footer carries them now.
    assert!(!canvas.contains("claude-opus-5"), "{canvas}");
}

#[test]
fn the_pan_follows_the_selection() {
    let (mut app, layout) = opened();
    let canvas = overview::view(&app, layout.main).canvas;
    assert_eq!(app.graph_pan, Pan::default());

    // The deepest tier lies past the viewport's right edge, and nothing else.
    let deep = NodeKey::Subagent {
        window_id: 4,
        id: "b3".into(),
    };
    select(&mut app, deep.clone());
    let view = overview::view(&app, layout.main);
    assert!(
        app.graph_pan.x > 0,
        "the deepest tier is off the right edge of a {}-column viewport: {:?}",
        canvas.width,
        app.graph_pan
    );
    assert_eq!(app.graph_pan.y, 0, "it was already vertically visible");
    let rect = view.layout.node(&deep).unwrap().rect;
    assert!(
        rect.x >= view.pan.x && rect.right() <= view.pan.x + canvas.width,
        "{rect:?} is not wholly inside the viewport at {:?}",
        view.pan
    );

    // The last project's window lies past the bottom edge, and back to the left.
    select(&mut app, NodeKey::Window(8));
    let view = overview::view(&app, layout.main);
    assert!(app.graph_pan.y > 0, "{:?}", app.graph_pan);
    let rect = view.layout.node(&NodeKey::Window(8)).unwrap().rect;
    assert!(
        rect.y >= view.pan.y && rect.bottom() <= view.pan.y + canvas.height,
        "{rect:?} is not wholly inside the viewport at {:?}",
        view.pan
    );
    assert!(rect.x >= view.pan.x && rect.right() <= view.pan.x + canvas.width);
}

#[test]
fn the_footer_shows_the_selected_node_in_full() {
    let (mut app, layout) = opened_at(200, 50);
    let label = "grep every handler in the repository";
    app.windows[0].subagents[1].label = Some(label.into());
    app.windows[0].subagents[1].model = Some("claude-haiku-4-5".into());
    let key = NodeKey::Subagent {
        window_id: 1,
        id: "a2".into(),
    };
    select(&mut app, key.clone());
    let full = format!("general-purpose: {label}");

    let view = overview::view(&app, layout.main);
    let buffer = drawn(&app, 200, 50);
    let footer = text_in(&buffer, view.footer);
    assert!(footer.contains(&full), "footer {footer:?} misses {full:?}");
    for field in ["claude-haiku-4-5", "running", "20s"] {
        assert!(footer.contains(field), "footer {footer:?} misses {field:?}");
    }
    // The box itself cannot hold the label: that is what the footer is for.
    let canvas = text_in(&buffer, view.canvas);
    assert!(!canvas.contains(&full), "{canvas}");
    // Scoped to the selected node's own box, not any elided box in the
    // fixture: the truncation under test is this one's, not a neighbour's.
    let rect = view
        .layout
        .node(&key)
        .expect("the selected node is placed")
        .rect;
    let node_screen = Rect {
        x: view.canvas.x + rect.x.saturating_sub(view.pan.x),
        y: view.canvas.y + rect.y.saturating_sub(view.pan.y),
        width: rect.width,
        height: rect.height,
    };
    let node_text = text_in(&buffer, node_screen);
    assert!(node_text.contains('…'), "{node_text}");
}

/// The whole footer line for the current selection, drawn at 200 x 50 so
/// nothing in it is cut off, with the padding to the right trimmed away.
fn footer_text(app: &App, layout: &crate::ui::Layout) -> String {
    let view = overview::view(app, layout.main);
    text_in(&drawn(app, 200, 50), view.footer)
        .trim_end()
        .to_owned()
}

/// A stopped sub-agent's footer: `started_secs` and `ended_secs` are both
/// *ages*, so the run is the older age minus the newer one. Subtracting them
/// the other way round reads zero for every sub-agent that ever ran.
#[test]
fn the_footer_times_a_stopped_subagent_from_the_two_ages() {
    let (mut app, layout) = opened_at(200, 50);
    let key = NodeKey::Subagent {
        window_id: 1,
        id: "a3".into(),
    };
    // a3 started 60 seconds ago and ended 15 seconds ago: it ran for 45.
    assert_eq!(app.windows[0].subagents[2].started_secs, 60);
    assert_eq!(app.windows[0].subagents[2].ended_secs, Some(15));
    select(&mut app, key.clone());
    assert_eq!(
        footer_text(&app, &layout),
        "✓ tests: run unit suite  -  done  45s"
    );

    // An end that never arrived reads as no elapsed time rather than as the
    // sub-agent's own age.
    app.windows[0].subagents[2].ended_secs = None;
    assert_eq!(
        footer_text(&app, &layout),
        "✓ tests: run unit suite  -  done  0s"
    );
}

/// The three sub-agent states name themselves in the footer, and a stopped
/// sub-agent's name is not its neighbour's.
#[test]
fn the_footer_names_the_subagent_state() {
    let (mut app, layout) = opened_at(200, 50);
    let done = NodeKey::Subagent {
        window_id: 1,
        id: "a3".into(),
    };
    let running = NodeKey::Subagent {
        window_id: 1,
        id: "a1".into(),
    };
    select(&mut app, done.clone());
    assert!(footer_text(&app, &layout).contains("  done  "));

    app.windows[0].subagents[2].state = proto::SubagentState::Failed;
    let failed = footer_text(&app, &layout);
    assert!(failed.contains("  failed  "), "{failed}");
    assert!(!failed.contains("done"), "{failed}");
    // A failed sub-agent is still timed from the two ages, not from its age.
    assert!(failed.ends_with("45s"), "{failed}");

    select(&mut app, running);
    let running = footer_text(&app, &layout);
    assert!(running.contains("  running  "), "{running}");
    assert!(!running.contains("done"), "{running}");
}

/// The project arm: the root, shortened, and the per-runtime counts.
#[test]
fn the_footer_spells_a_project_out_with_its_root_and_counts() {
    let (mut app, layout) = opened_at(200, 50);
    select(&mut app, NodeKey::Project("/r/shop".into()));
    // Seven windows: four Claude, three Codex. The project's own status is
    // its most urgent window's.
    assert_eq!(
        footer_text(&app, &layout),
        "◆ shop  /r/shop  attention  cl 4 · cx 3"
    );

    // A project at the home directory reads `~`, not the bare `~/` that
    // shortening a path that *is* the home directory leaves behind.
    let home = dirs::home_dir().expect("the test host has a home directory");
    let mut window = win(1, "home-agent", Status::Idle);
    window.cwd = home.clone();
    window.project = home.clone();
    let mut app = app_with(vec![window]);
    assert!(toggle(&mut app).is_empty());
    app.set_graph_viewport(layout.main);
    select(&mut app, NodeKey::Project(home));
    let footer = footer_text(&app, &layout);
    assert!(footer.contains("  ~  idle  sh 1"), "{footer}");
    assert!(!footer.contains("~/"), "{footer}");
}

#[test]
fn elapsed_duration_saturates_when_extrapolating_old_lists() {
    let mut app = app_with(tree::example_windows());
    app.windows_received_at = Instant::now() - Duration::from_secs(5);
    let mut window = app.windows[0].clone();
    window.since_secs = u64::MAX;
    assert_eq!(app.elapsed_secs(&window), u64::MAX);
}
