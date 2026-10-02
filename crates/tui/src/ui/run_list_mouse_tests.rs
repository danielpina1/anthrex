//! M9.0.7.10: the compact list's mouse (decision 22, Review focus 3): `row_at` against
//! the drawn rows, clicks, double clicks and the wheel. Split from `run_list_tests.rs`.

use super::tests::{MUL, forty_selected, frame, rows_in, select, task_key, two_stage_run_selected};
use super::{first_row, row_at};
use crate::tree::NodeKey;
use crate::tree::run_fixtures::RUN_ID;
use crate::ui::overview;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::layout::Rect;
use ratatui::{Terminal, backend::TestBackend};

/// Review focus 3: `row_at` names exactly the node `render` drew on each row, at every
/// height and selection, scrolled or not; marks and empty rows name none.
#[test]
fn row_at_maps_exactly_the_rows_render_draws() {
    let mut app = forty_selected("t0");
    for height in 0..=9u16 {
        for selected in [0usize, 1, 2, 5, 20, 38, 39, 40] {
            let rows = app.nav_rows();
            let key = rows[selected].key.clone();
            drop(rows);
            select(&mut app, key);
            let area = Rect::new(3, 2, 30, height);
            let mut terminal = Terminal::new(TestBackend::new(40, 12)).unwrap();
            let rows = app.nav_rows();
            terminal
                .draw(|f| super::render(f, &app, area, &rows))
                .unwrap();
            let drawn = rows_in(terminal.backend().buffer(), area);
            for (r, text) in drawn.iter().enumerate() {
                let y = area.y + u16::try_from(r).unwrap();
                let hit = row_at(area, rows.len(), selected, area.x + 5, y);
                let label = |i: usize| match i {
                    0 => "run 3f9a".to_owned(),
                    i => format!("t{} work", i - 1),
                };
                match hit {
                    Some(i) => assert!(
                        text.contains(&label(i)),
                        "h{height} s{selected} row {r}: {text:?} is not {}",
                        label(i)
                    ),
                    None => assert!(
                        text.is_empty() || text.contains(" more"),
                        "h{height} s{selected} row {r}: {text:?} names no node"
                    ),
                }
            }
            assert_eq!(
                row_at(area, rows.len(), selected, area.x, area.bottom()),
                None
            );
            assert_eq!(
                row_at(area, rows.len(), selected, area.right(), area.y),
                None
            );
        }
    }
}

/// Review focus 3, through `mouse.rs`: a click selects the row drawn under it, also
/// with the list scrolled; a mark and the rows below the last node select nothing.
#[test]
fn a_click_in_the_compact_list_selects_its_row() {
    let mut app = forty_selected("t30");
    let (_, layout) = frame(&mut app, 80, 24);
    let canvas = overview::view(&app, layout.main).canvas;
    let first = first_row(41, 31, usize::from(canvas.height));
    assert!(first > 0, "the list is scrolled");
    assert!(app.on_click(canvas.x + 6, canvas.y + 1, &layout).is_empty());
    let want = format!("t{}", first);
    assert_eq!(app.tree.selected, Some(task_key(RUN_ID, &want)), "row 1");
    let (buffer, layout) = frame(&mut app, 80, 24);
    let canvas = overview::view(&app, layout.main).canvas;
    let rows = rows_in(&buffer, canvas);
    assert!(
        rows.iter()
            .any(|row| row.starts_with('▌') && row.contains(&format!("{want} work"))),
        "{rows:#?}"
    );

    // The `↑` mark selects nothing.
    let before = app.tree.selected.clone();
    assert!(app.on_click(canvas.x + 2, canvas.y, &layout).is_empty());
    assert_eq!(app.tree.selected, before);

    // On a short list, a click under the last node selects nothing; one on the
    // root selects it.
    let mut app = two_stage_run_selected("t3");
    let (_, layout) = frame(&mut app, 80, 24);
    let canvas = overview::view(&app, layout.main).canvas;
    let len = app.nav_rows().len();
    if usize::from(canvas.height) > len {
        let y = canvas.y + u16::try_from(len).unwrap();
        assert!(app.on_click(canvas.x + 2, y, &layout).is_empty());
        assert_eq!(app.tree.selected, Some(task_key(MUL, "t3")));
    }
    assert!(app.on_click(canvas.x + 2, canvas.y, &layout).is_empty());
    assert_eq!(app.tree.selected, Some(NodeKey::Run(MUL.into())));
}

/// Fix round 1 (I1): the first press of a double click re-centres the window on the
/// row it selects; the second opens that node, not the row now under the pointer.
#[test]
fn a_double_click_on_a_scrolled_list_opens_the_row_first_pressed() {
    let mut app = forty_selected("t30");
    let (_, layout) = frame(&mut app, 80, 24);
    let canvas = overview::view(&app, layout.main).canvas;
    let first = first_row(41, 31, usize::from(canvas.height));
    let want = task_key(RUN_ID, &format!("t{first}"));
    let (x, y) = (canvas.x + 6, canvas.y + 1);
    assert!(app.on_click(x, y, &layout).is_empty());
    assert_eq!(app.tree.selected, Some(want.clone()));
    let (_, layout) = frame(&mut app, 80, 24);
    let clicked = app.on_click(x, y, &layout);
    assert_eq!(
        app.tree.selected,
        Some(want.clone()),
        "the double click kept its row"
    );

    let mut by_key = forty_selected(&format!("t{first}"));
    let _ = frame(&mut by_key, 80, 24);
    let entered = by_key.on_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    assert_eq!(clicked, entered);
    assert_eq!(app.tree.selected, by_key.tree.selected);
    assert_eq!(app.run_view, by_key.run_view);
    // Opening a task with no agent says so, naming the task the first press picked.
    let toast = format!("t{first} has no agent yet");
    assert_eq!(app.toast_text(), Some(toast.as_str()));
    assert_eq!(by_key.toast_text(), Some(toast.as_str()));

    // A double click on the `↑` mark opens nothing: its first press picked no row.
    let mut app = forty_selected("t30");
    let (_, layout) = frame(&mut app, 80, 24);
    let canvas = overview::view(&app, layout.main).canvas;
    let mark = (canvas.x + 2, canvas.y);
    assert!(app.on_click(mark.0, mark.1, &layout).is_empty());
    assert!(app.on_click(mark.0, mark.1, &layout).is_empty());
    assert_eq!(app.tree.selected, Some(task_key(RUN_ID, "t30")));
    assert_eq!(app.toast_text(), None, "nothing was opened");
}

/// Decision 25: a selection the wheel moves to starts its panel at the top, as a key's
/// or a click's does.
#[test]
fn the_wheel_forgets_the_panel_state_of_the_row_it_leaves() {
    let mut app = forty_selected("t30");
    let _ = frame(&mut app, 80, 24);
    let t30 = task_key(RUN_ID, "t30");
    app.inspector_scroll = Some((t30.clone(), 2));
    app.brief_expanded = Some(t30.clone());
    let (_, layout) = frame(&mut app, 80, 24);
    let canvas = overview::view(&app, layout.main).canvas;
    let _ = app.on_scroll(false, canvas.x + 2, canvas.y + 1, &layout);
    assert_eq!(app.tree.selected, Some(task_key(RUN_ID, "t33")));
    assert_eq!(app.inspector_scroll, None);
    assert_eq!(app.brief_expanded, None);
    let (_, layout) = frame(&mut app, 80, 24);
    let canvas = overview::view(&app, layout.main).canvas;
    let _ = app.on_scroll(true, canvas.x + 2, canvas.y + 1, &layout);
    assert_eq!(app.tree.selected, Some(t30));
    assert_eq!(
        app.inspector_scroll, None,
        "coming back does not restore it"
    );
    assert_eq!(app.brief_expanded, None);
}

#[test]
fn a_double_click_in_the_list_opens_as_enter_does() {
    let mut by_click = two_stage_run_selected("t3");
    let (_, layout) = frame(&mut by_click, 80, 24);
    let canvas = overview::view(&by_click, layout.main).canvas;
    // Row 2 is `t1`.
    let _ = by_click.on_click(canvas.x + 6, canvas.y + 2, &layout);
    let (_, layout) = frame(&mut by_click, 80, 24);
    let canvas = overview::view(&by_click, layout.main).canvas;
    assert_eq!(by_click.tree.selected, Some(task_key(MUL, "t1")));
    let clicked = by_click.on_click(canvas.x + 6, canvas.y + 2, &layout);

    let mut by_key = two_stage_run_selected("t1");
    let _ = frame(&mut by_key, 80, 24);
    let entered = by_key.on_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    assert_eq!(clicked, entered);
    assert_eq!(by_click.tree.selected, by_key.tree.selected);
}

#[test]
fn the_wheel_moves_the_selection_three_rows() {
    let mut app = forty_selected("t30");
    let (_, layout) = frame(&mut app, 80, 24);
    let canvas = overview::view(&app, layout.main).canvas;
    let (x, y) = (canvas.x + 2, canvas.y + 1);
    assert!(app.on_scroll(false, x, y, &layout).is_empty());
    assert_eq!(app.tree.selected, Some(task_key(RUN_ID, "t33")));
    let _ = app.on_scroll(true, x, y, &layout);
    let _ = app.on_scroll(true, x, y, &layout);
    assert_eq!(app.tree.selected, Some(task_key(RUN_ID, "t27")));

    // At the ends the wheel stops on the first and the last row.
    let mut app = forty_selected("t38");
    let (_, layout) = frame(&mut app, 80, 24);
    let canvas = overview::view(&app, layout.main).canvas;
    let _ = app.on_scroll(false, canvas.x + 2, canvas.y + 1, &layout);
    assert_eq!(app.tree.selected, Some(task_key(RUN_ID, "t39")));
    let _ = app.on_scroll(false, canvas.x + 2, canvas.y + 1, &layout);
    assert_eq!(app.tree.selected, Some(task_key(RUN_ID, "t39")));
    let mut app = forty_selected("t0");
    let (_, layout) = frame(&mut app, 80, 24);
    let canvas = overview::view(&app, layout.main).canvas;
    let _ = app.on_scroll(true, canvas.x + 2, canvas.y + 1, &layout);
    assert_eq!(app.tree.selected, Some(NodeKey::Run(RUN_ID.into())));
    let _ = app.on_scroll(true, canvas.x + 2, canvas.y + 1, &layout);
    assert_eq!(app.tree.selected, Some(NodeKey::Run(RUN_ID.into())));
    // The wheel over the panel below moves nothing.
    let footer = overview::view(&app, layout.main).footer;
    let _ = app.on_scroll(false, footer.x + 2, footer.y + 1, &layout);
    assert_eq!(app.tree.selected, Some(NodeKey::Run(RUN_ID.into())));
}

/// Final fix wave (task 10's deferred minor): a press on the graph forgets the list's
/// pick, so a resize from the graph to the list between two presses on one cell never
/// opens the node an older list press picked.
#[test]
fn a_graph_press_forgets_the_lists_pick() {
    let mut app = forty_selected("t30");
    let (_, layout) = frame(&mut app, 80, 24);
    let list = overview::view(&app, layout.main).canvas;
    // A list press picks the row at row 2.
    assert!(app.on_click(list.x + 6, list.y + 2, &layout).is_empty());
    let picked = app.tree.selected.clone().expect("a pick");
    assert_ne!(picked, task_key(RUN_ID, "t30"), "the press picked a row");
    // The terminal grows to the graph; a press elsewhere on the canvas.
    let (_, layout) = frame(&mut app, 120, 40);
    let graph = overview::view(&app, layout.main);
    assert!(!graph.list, "120x40 draws the graph");
    let (x, y) = (graph.canvas.x + 6, graph.canvas.y + 1);
    let _ = app.on_click(x, y, &layout);
    // It shrinks back to the list; a second press on that cell is a double click.
    let (_, layout) = frame(&mut app, 80, 24);
    assert!(overview::view(&app, layout.main).list);
    let _ = app.on_click(x, y, &layout);
    let toast = format!("{} has no agent yet", node_id(&picked));
    assert_ne!(
        app.toast_text(),
        Some(toast.as_str()),
        "the stale pick opened"
    );
}

fn node_id(key: &NodeKey) -> String {
    match key {
        NodeKey::Task { id, .. } => id.clone(),
        other => format!("{other:?}"),
    }
}
