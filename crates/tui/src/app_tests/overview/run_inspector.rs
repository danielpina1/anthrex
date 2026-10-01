//! M8c.8: the run view's tall panel (decision 28) wired into the overview — its height
//! steps, the `i` toggle, the single line, and the degenerate-size guard with the run
//! view open.

use super::run_view::{select_nav, three};
use super::*;
use crate::inspector::{INSPECTOR_HEIGHT, MIN_INTERIOR_FOR_PANEL, RUN_CANVAS_MIN};
use crate::tree::RunFilter;
use crate::tree::run_fixtures::{RUN_ID, gemini_fixture};

use super::super::runs::{app_with_runs, open_run_view};

/// A terminal whose overview interior is `interior` rows: the status bar and the
/// overview's two border rows are the rest.
const CHROME: u16 = 3;
const WIDTH: u16 = 200;

/// Sets both viewports for a `width` x `height` terminal, as `lib::draw` does.
fn laid_out(app: &mut App, width: u16, height: u16) -> crate::ui::Layout {
    let layout = crate::ui::layout(
        Rect::new(0, 0, width, height),
        app.sidebar_width,
        crate::app::alerts(app).len(),
    );
    app.set_tree_viewports(layout.sidebar_list.height, layout.main_inner.height);
    app.set_graph_viewport(layout.main);
    layout
}

/// The Gemini run's view, open on `r1`, with `t2` selected.
fn gemini_view() -> App {
    let (snapshot, windows) = gemini_fixture();
    let mut app = app_with_runs(windows, snapshot);
    open_run_view(&mut app, "r1");
    let key = NodeKey::Task {
        run: "r1".into(),
        id: "t2".into(),
    };
    let rows = tree::run_rows(&app.runs.runs[0], &app.windows, &app.tree, RunFilter::All);
    app.tree.select(&rows, key.clone());
    assert_eq!(app.tree.selected, Some(key), "t2 is a run-view row");
    app
}

/// The footer's height at an interior of `interior` rows, after checking that the
/// canvas and the footer are the whole interior and that the viewport every gesture
/// hit-tests against is the canvas the frame is drawn with.
fn footer_at(app: &mut App, interior: u16) -> u16 {
    let layout = laid_out(app, WIDTH, interior + CHROME);
    assert_eq!(layout.main_inner.height, interior);
    let view = overview::view(app, layout.main);
    assert_eq!(view.canvas.height + view.footer.height, interior);
    assert_eq!(
        app.graph_area, view.canvas,
        "set_graph_viewport and the frame agree on the footer"
    );
    view.footer.height
}

/// What is drawn below the canvas at an interior of `interior` rows.
fn below_the_canvas(app: &mut App, interior: u16) -> String {
    let layout = laid_out(app, WIDTH, interior + CHROME);
    let view = overview::view(app, layout.main);
    text_in(&drawn(app, WIDTH, interior + CHROME), view.footer)
}

/// Milestone 9.0.7 decision 17: the run view's panel takes the rows its node needs,
/// here all of `t2`'s (its title, nineteen body rows before the detail lands, and the
/// footer) and its borders: 21 of an interior of 30, nothing cut.
#[test]
fn the_run_view_gets_the_tall_panel() {
    let mut app = gemini_view();
    assert_eq!(footer_at(&mut app, 30), 21);
    let panel = below_the_canvas(&mut app, 30);
    let lines: Vec<&str> = panel.lines().collect();
    assert_eq!(lines.len(), 21);
    assert!(
        lines[0].starts_with('╭') && !lines[0].contains("PgUp"),
        "{panel}"
    );
    assert!(
        lines[1].contains("◐ t2  map Gemini hook events to status")
            && lines[1]
                .trim_end_matches(['│', ' '])
                .ends_with("in review · r2"),
        "{panel}"
    );
    assert!(
        lines[2].contains("OUTCOME") && lines[6].contains("EVIDENCE"),
        "{panel}"
    );
    assert!(
        lines[18].contains("│ history   12:31 review r1 changes"),
        "{panel}"
    );
    assert!(lines[19].contains("│ cx default · M · tdd "), "{panel}");
    assert!(
        lines[20].starts_with('╰') && lines[20].ends_with(" . actions ╯"),
        "{panel}"
    );

    // The project overview at the same size keeps milestone 4.7's eight rows, even with
    // the run's own node selected.
    press(&mut app, KeyCode::Esc, KeyModifiers::NONE);
    assert!(app.run_view.is_none() && app.overview);
    assert_eq!(app.tree.selected, Some(NodeKey::Run("r1".into())));
    assert_eq!(footer_at(&mut app, 30), INSPECTOR_HEIGHT);
    let panel = below_the_canvas(&mut app, 30);
    assert!(panel.contains("◉ r1  Add Gemini runtime"), "{panel}");
    assert!(panel.contains("gate      plan approved 11:02"), "{panel}");
    assert!(
        !panel.contains("attention"),
        "five fields, then dropped:\n{panel}"
    );
}

#[test]
fn a_short_terminal_steps_down() {
    let mut app = gemini_view();
    assert_eq!(footer_at(&mut app, 18), 18 - RUN_CANVAS_MIN);
    assert_eq!(footer_at(&mut app, 17), 17 - RUN_CANVAS_MIN);
    assert_eq!(
        footer_at(&mut app, MIN_INTERIOR_FOR_PANEL),
        INSPECTOR_HEIGHT
    );
    // The eight-row panel is still the panel, not the single line.
    let panel = below_the_canvas(&mut app, MIN_INTERIOR_FOR_PANEL);
    let lines: Vec<&str> = panel.lines().collect();
    assert_eq!(lines.len(), usize::from(INSPECTOR_HEIGHT));
    assert!(
        lines[0].starts_with('╭') && lines[7].starts_with('╰'),
        "{panel}"
    );
    assert!(lines[1].contains("◐ t2  map Gemini hook events"), "{panel}");
    // Milestone 9.0.7: the footer (was 9.0.5's STATUS `worker`), OUTCOME above it.
    assert!(lines[6].contains("│ cx default · M · tdd "), "{panel}");
    assert!(lines[5].contains("accept    loading…"), "{panel}");

    assert_eq!(
        footer_at(&mut app, MIN_INTERIOR_FOR_PANEL),
        INSPECTOR_HEIGHT
    );
    assert_eq!(footer_at(&mut app, 13), 1);
    let line = below_the_canvas(&mut app, 13);
    assert!(line.starts_with("◐ t2  map Gemini"), "{line}");
    assert!(!line.contains('╭'), "{line}");
    assert!(app.inspector_visible, "the collapse is not a toggle");
}

#[test]
fn i_toggles_the_tall_panel_too() {
    let mut app = gemini_view();
    let layout = laid_out(&mut app, WIDTH, 30 + CHROME);
    let tall = overview::view(&app, layout.main);
    let sized = tall.footer.height;
    assert!(sized > INSPECTOR_HEIGHT, "{sized}");

    press(&mut app, KeyCode::Char('i'), KeyModifiers::NONE);
    assert!(!app.inspector_visible);
    assert!(app.run_view.is_some(), "`i` leaves the run view open");
    assert_eq!(footer_at(&mut app, 30), 1);
    let line = overview::view(&app, layout.main);
    assert_eq!(line.canvas.height, tall.canvas.height + sized - 1);

    press(&mut app, KeyCode::Char('i'), KeyModifiers::NONE);
    assert!(app.inspector_visible);
    assert_eq!(footer_at(&mut app, 30), sized);
}

/// Opening and leaving the run view re-split the last frame's area at once, so keys
/// pressed before the next frame reveal into the canvas that frame will draw.
#[test]
fn opening_and_leaving_the_run_view_resplit_the_viewport_at_once() {
    let mut app = gemini_view();
    let layout = laid_out(&mut app, WIDTH, 30 + CHROME);
    let view = overview::view(&app, layout.main);
    let (tall, sized) = (view.canvas, view.footer.height);
    assert!(sized > INSPECTOR_HEIGHT, "{sized}");
    assert_eq!(app.graph_area, tall);

    press(&mut app, KeyCode::Esc, KeyModifiers::NONE);
    assert!(app.run_view.is_none());
    let project = overview::view(&app, layout.main).canvas;
    assert_eq!(project.height, tall.height + sized - INSPECTOR_HEIGHT);
    assert_eq!(app.graph_area, project, "no frame drawn in between");

    // Milestone 9.0.7 decision 17: the run view reopens on the run's own node, whose
    // panel is sized to it, not to `t2`'s.
    press(&mut app, KeyCode::Char('l'), KeyModifiers::NONE);
    assert!(app.run_view.is_some());
    assert_eq!(app.tree.selected, Some(NodeKey::Run("r1".into())));
    let reopened = overview::view(&app, layout.main);
    assert_eq!(app.graph_area, reopened.canvas, "no frame drawn in between");
    assert_ne!(
        reopened.footer.height, sized,
        "the run's node has its own rows"
    );
}

#[test]
fn the_single_line_for_a_task() {
    let mut app = gemini_view();
    app.inspector_visible = false;
    assert_eq!(
        below_the_canvas(&mut app, 30).trim_end(),
        "◐ t2  map Gemini hook events to status  in review · r2"
    );
}

/// Milestone 4.7's guard with the run view open: every degenerate size either side of
/// both steps, the inspector both ways, drawn, clicked, dragged and scrolled.
#[test]
fn no_panic_at_degenerate_sizes_in_the_run_view() {
    let t1 = NodeKey::Task {
        run: RUN_ID.into(),
        id: "t1".into(),
    };
    let reopen = |app: &mut App| {
        if app.keymap.conversation_mode() {
            press(app, KeyCode::Char('q'), KeyModifiers::NONE);
        }
        if app.run_view.is_none() {
            app.tree.collapsed.clear();
            open_run_view(app, RUN_ID);
        }
        select_nav(app, t1.clone());
    };
    let mut steps = [0u32; 3];
    for inspector_visible in [false, true] {
        let mut app = three();
        open_run_view(&mut app, RUN_ID);
        select_nav(&mut app, t1.clone());
        app.inspector_visible = inspector_visible;
        for width in [1, 2, 3, 4, 12, 40, 61] {
            for height in 0..=(MIN_INTERIOR_FOR_PANEL + 8) {
                let layout = laid_out(&mut app, width, height);
                assert!(app.run_view.is_some(), "{width}x{height}");
                let view = overview::view(&app, layout.main);
                let interior = layout.main_inner.height;
                assert_eq!(
                    view.canvas.height + view.footer.height,
                    interior,
                    "{width}x{height}: the split lost a row"
                );
                assert_eq!(app.graph_area, view.canvas, "{width}x{height}");
                // Milestone 9.0.7 decision 17: the panel takes its node's rows, from
                // the eight-row least to the interior less six canvas rows.
                match view.footer.height {
                    sized if sized > INSPECTOR_HEIGHT => {
                        assert!(inspector_visible);
                        assert!(view.canvas.height >= RUN_CANVAS_MIN, "{width}x{height}");
                        steps[0] += 1;
                    }
                    INSPECTOR_HEIGHT => {
                        assert!(inspector_visible);
                        assert!(view.canvas.height >= RUN_CANVAS_MIN, "{width}x{height}");
                        steps[1] += 1;
                    }
                    other => {
                        assert!(other <= 1, "{width}x{height}: footer {other}");
                        assert!(
                            !inspector_visible || interior < MIN_INTERIOR_FOR_PANEL,
                            "{width}x{height}"
                        );
                        steps[2] += 1;
                    }
                }
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
                    // A double click can leave the run view (the root focuses the
                    // orchestrator) or open a conversation over it.
                    app.on_click(x, y, &layout);
                    reopen(&mut app);
                    drawn(&app, width, height);
                }
            }
        }
    }
    assert!(
        steps.iter().all(|count| *count > 0),
        "every step was reached: {steps:?}"
    );
}

/// `t2`'s detail, landed: a five-line brief and two criteria.
fn with_t2_detail(app: &mut App) {
    use crate::app::task_detail::{DetailState, TaskDetailCache, detail_key};
    let task = app.runs.runs[0]
        .tasks
        .iter()
        .find(|t| t.id == "t2")
        .unwrap();
    app.task_detail = Some(TaskDetailCache {
        run_id: "r1".into(),
        task_id: "t2".into(),
        key: detail_key(task),
        state: DetailState::Ready(Box::new(proto::TaskDetailInfo {
            run_id: "r1".into(),
            task_id: "t2".into(),
            brief: "Map each Gemini hook event to a status.\nSubagentStop pairs with its start.\nStop marks the agent idle.\nNotification asks for input.\nKeep the table in one place.".into(),
            acceptance: vec!["every event maps".into(), "stop marks idle".into()],
            worker_summary: None,
            summary_source: None,
        })),
    });
}

/// The task panel's rows in a `width` x `height` terminal.
fn task_panel(width: u16, height: u16) -> Vec<String> {
    let mut app = gemini_view();
    with_t2_detail(&mut app);
    panel_rows(&mut app, width, height)
}

fn panel_rows(app: &mut App, width: u16, height: u16) -> Vec<String> {
    let layout = laid_out(app, width, height);
    let view = overview::view(app, layout.main);
    text_in(&drawn(app, width, height), view.footer)
        .lines()
        .map(str::to_owned)
        .collect()
}

/// Milestone 9.0.7 decision 12 in decision 17's panel at 80x24: the state word under the
/// title, OUTCOME first, the footer on the last row; the panel is cut at 15 rows (an
/// interior of 21 less six canvas rows).
#[test]
fn task_panel_renders_at_80x24() {
    assert_eq!(
        task_panel(80, 24),
        [
            "╭──────────────────────────────────────────╮",
            "│ ◐ t2  map Gemini hook events to status   │",
            "│   in review · r2                         │",
            "│ OUTCOME                                  │",
            "│ pipeline  done ✓ › proof ✓ › check ✓ ›   │",
            "│           review › merge ◌               │",
            "│ check     ✓ passed · test result: ok     │",
            "│ accept    ◌ every event maps             │",
            "│           ◌ stop marks idle              │",
            "│ EVIDENCE                                 │",
            "│ diff      +212 −31 · 4 files · test      │",
            "│           `status::gemini_stop_marks_idl │",
            "│           e` red a1b2c3d ✓               │",
            "│ cx default · M · tdd                     │",
            "╰────────────────────── ↓ PgDn · . actions ╯",
        ]
    );
}

/// Decision 17 at 120x40: the whole panel, 24 rows, nothing below it.
#[test]
fn task_panel_renders_at_120x40() {
    let rows = task_panel(120, 40);
    assert_eq!(rows.len(), 24, "{rows:#?}");
    let inner: Vec<&str> = rows[1..rows.len() - 1]
        .iter()
        .map(|row| row.trim_start_matches("│ ").trim_end_matches(['│', ' ']))
        .collect();
    assert_eq!(
        inner,
        [
            "◐ t2  map Gemini hook events to status                            in review · r2",
            "OUTCOME",
            "pipeline  done ✓ › proof ✓ › check ✓ › review › merge ◌",
            "check     ✓ passed · test result: ok",
            "accept    ◌ every event maps",
            "          ◌ stop marks idle",
            "EVIDENCE",
            "diff      +212 −31 · 4 files · test `status::gemini_stop_marks_idle` red a1b2c3d",
            "          ✓",
            "review    in review · r2",
            "INTENT",
            "brief     Map each Gemini hook event to a status.",
            "          … (b: more)",
            "DETAIL",
            "phase     in review",
            "worker    worker #1 · codex · 26m · 41 tool calls",
            "deps      after t0 ✓, t6 ✓ · unblocks t3, t7 · on critical path",
            "budget    ███████░░░ 104/150 tool calls · 38/60 min · 410k tokens",
            "tries     review 1/2 bounces · check 0/2 · escalation step 1",
            "route     codex · standard · high effort → reviewer claude · frontier",
            "history   12:31 review r1 changes · 12:20 check passed · 12:02 started",
            "cx default · M · tdd",
        ]
    );
    assert!(rows[0].starts_with('╭') && rows[23].starts_with('╰'));
    // Decision 16: nothing lies below, so the border names only the actions.
    assert!(rows[23].ends_with("─ . actions ╯"), "{}", rows[23]);
    assert!(!rows[0].contains("PgUp"), "{}", rows[0]);
}

/// The reducer's page size comes from the panel the frame draws: its interior is
/// the rows between the borders and the columns inside the padding.
#[test]
fn the_page_size_is_the_drawn_panels_interior() {
    for (width, height) in [(80, 24), (120, 40)] {
        let mut app = gemini_view();
        let layout = laid_out(&mut app, width, height);
        let view = overview::view(&app, layout.main);
        let rows: Vec<String> = text_in(&drawn(&app, width, height), view.footer)
            .lines()
            .map(str::to_owned)
            .collect();
        let top = rows.iter().position(|r| r.starts_with('╭')).unwrap();
        let bottom = rows.iter().position(|r| r.starts_with('╰')).unwrap();
        let inner_rows = u16::try_from(bottom - top - 1).unwrap();
        // `│ ` and ` │` on either side.
        let inner_cols = u16::try_from(rows[top].chars().count() - 4).unwrap();
        assert_eq!(
            app.task_panel_interior(),
            (inner_cols, inner_rows),
            "{width}x{height}"
        );
    }
}

/// Ruling D-2: a merged task's outcome shows without scrolling, at the end of OUTCOME
/// (milestone 9.0.7 decision 12), at 120x40 and, with the panel sized to its content
/// (decision 17), at 80x24 too: the panel there takes 15 rows (an interior of 21 less
/// six canvas rows), so its body is 10 rows under the two title rows and the result is
/// the eighth of them.
#[test]
fn a_merged_tasks_result_shows_without_scrolling() {
    for (width, height, result_row) in [(80, 24, 10), (120, 40, 7)] {
        let (mut snapshot, windows) = crate::tree::run_fixtures::gemini_fixture();
        let t2 = snapshot.runs[0]
            .tasks
            .iter_mut()
            .find(|t| t.id == "t2")
            .unwrap();
        t2.state = proto::TaskState::Merged;
        t2.merge_commit = Some("0123456789".into());
        let mut app = app_with_runs(windows, snapshot);
        open_run_view(&mut app, "r1");
        let key = NodeKey::Task {
            run: "r1".into(),
            id: "t2".into(),
        };
        let rows = tree::run_rows(&app.runs.runs[0], &app.windows, &app.tree, RunFilter::All);
        app.tree.select(&rows, key);
        with_t2_detail(&mut app);
        if let Some(crate::app::task_detail::DetailState::Ready(detail)) =
            app.task_detail.as_mut().map(|cache| &mut cache.state)
        {
            detail.worker_summary = Some("Mapped all nine hook events.\nAdded a test.".into());
            detail.summary_source = Some(proto::SummarySource::TaskDone);
        }
        let panel = panel_rows(&mut app, width, height);
        assert!(
            panel[result_row].starts_with("│ result    Mapped all nine hook events."),
            "{width}x{height}: {panel:#?}"
        );
        let above = &panel[result_row - 1];
        assert!(above.starts_with("│           ◌ stop"), "{panel:#?}");
        assert!(
            panel[0].starts_with('╭') && !panel[0].contains("PgUp"),
            "{panel:#?}"
        );
        let last = panel.last().unwrap();
        assert!(last.ends_with(" . actions ╯"), "{panel:#?}");
    }
}
