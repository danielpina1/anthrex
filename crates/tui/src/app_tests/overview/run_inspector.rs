//! M8c.8: the run view's tall panel (decision 28) wired into the overview — its height
//! steps, the `i` toggle, the single line, and the degenerate-size guard with the run
//! view open.

use super::run_view::{select_nav, three};
use super::*;
use crate::inspector::{
    INSPECTOR_HEIGHT, MIN_INTERIOR_FOR_PANEL, MIN_INTERIOR_FOR_RUN_PANEL, RUN_INSPECTOR_HEIGHT,
    RUN_INSPECTOR_TALL_HEIGHT,
};
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

#[test]
fn the_run_view_gets_the_tall_panel() {
    let mut app = gemini_view();
    assert_eq!(footer_at(&mut app, 30), RUN_INSPECTOR_HEIGHT);
    let panel = below_the_canvas(&mut app, 30);
    let lines: Vec<&str> = panel.lines().collect();
    assert_eq!(lines.len(), usize::from(RUN_INSPECTOR_HEIGHT));
    assert!(lines[0].starts_with('╭'), "{panel}");
    assert!(
        lines[1].contains("◐ t2  map Gemini hook events to status")
            && lines[1]
                .trim_end_matches(['│', ' '])
                .ends_with("M · tdd · review round 2"),
        "{panel}"
    );
    // Milestone 9.0.5: the task is GOAL, STATUS and RESULT; the ninth body row of
    // the twelve-row panel is STATUS's `route` (was M8c's flat `history`).
    assert!(
        lines[2].contains("GOAL") && lines[4].contains("STATUS"),
        "{panel}"
    );
    assert!(lines[10].contains("route     codex · standard"), "{panel}");
    assert!(lines[11].starts_with('╰'), "{panel}");

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
    assert_eq!(
        footer_at(&mut app, MIN_INTERIOR_FOR_RUN_PANEL),
        RUN_INSPECTOR_HEIGHT
    );
    assert_eq!(footer_at(&mut app, 17), INSPECTOR_HEIGHT);
    // The eight-row step is still the panel, not the single line.
    let panel = below_the_canvas(&mut app, 17);
    let lines: Vec<&str> = panel.lines().collect();
    assert_eq!(lines.len(), usize::from(INSPECTOR_HEIGHT));
    assert!(
        lines[0].starts_with('╭') && lines[7].starts_with('╰'),
        "{panel}"
    );
    assert!(lines[1].contains("◐ t2  map Gemini hook events"), "{panel}");
    // Milestone 9.0.5: STATUS's `worker` (was M8c's flat `tries`).
    assert!(lines[6].contains("worker    worker #1 · codex"), "{panel}");

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
    assert_eq!(tall.footer.height, RUN_INSPECTOR_HEIGHT);

    press(&mut app, KeyCode::Char('i'), KeyModifiers::NONE);
    assert!(!app.inspector_visible);
    assert!(app.run_view.is_some(), "`i` leaves the run view open");
    assert_eq!(footer_at(&mut app, 30), 1);
    let line = overview::view(&app, layout.main);
    assert_eq!(
        line.canvas.height,
        tall.canvas.height + RUN_INSPECTOR_HEIGHT - 1
    );

    press(&mut app, KeyCode::Char('i'), KeyModifiers::NONE);
    assert!(app.inspector_visible);
    assert_eq!(footer_at(&mut app, 30), RUN_INSPECTOR_HEIGHT);
}

/// Opening and leaving the run view re-split the last frame's area at once, so keys
/// pressed before the next frame reveal into the canvas that frame will draw.
#[test]
fn opening_and_leaving_the_run_view_resplit_the_viewport_at_once() {
    let mut app = gemini_view();
    let layout = laid_out(&mut app, WIDTH, 30 + CHROME);
    let tall = overview::view(&app, layout.main).canvas;
    assert_eq!(app.graph_area, tall);

    press(&mut app, KeyCode::Esc, KeyModifiers::NONE);
    assert!(app.run_view.is_none());
    let project = overview::view(&app, layout.main).canvas;
    assert_eq!(
        project.height,
        tall.height + RUN_INSPECTOR_HEIGHT - INSPECTOR_HEIGHT
    );
    assert_eq!(app.graph_area, project, "no frame drawn in between");

    press(&mut app, KeyCode::Char('l'), KeyModifiers::NONE);
    assert!(app.run_view.is_some());
    assert_eq!(app.graph_area, tall, "no frame drawn in between");
}

#[test]
fn the_single_line_for_a_task() {
    let mut app = gemini_view();
    app.inspector_visible = false;
    assert_eq!(
        below_the_canvas(&mut app, 30).trim_end(),
        "◐ t2  map Gemini hook events to status  M · tdd · review round 2"
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
            for height in 0..=(MIN_INTERIOR_FOR_RUN_PANEL + 4) {
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
                match view.footer.height {
                    RUN_INSPECTOR_HEIGHT => {
                        assert!(inspector_visible);
                        assert!(view.canvas.height >= 6, "{width}x{height}");
                        steps[0] += 1;
                    }
                    INSPECTOR_HEIGHT => {
                        assert!(inspector_visible);
                        assert!(interior < MIN_INTERIOR_FOR_RUN_PANEL, "{width}x{height}");
                        assert!(view.canvas.height >= 6, "{width}x{height}");
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
    let layout = laid_out(&mut app, width, height);
    let view = overview::view(&app, layout.main);
    text_in(&drawn(&app, width, height), view.footer)
        .lines()
        .map(str::to_owned)
        .collect()
}

/// Milestone 9.0.5 decision 22 at M8c's twelve rows: GOAL first, the brief collapsed.
#[test]
fn task_panel_renders_at_80x24() {
    assert_eq!(
        task_panel(80, 24),
        [
            "╭──────────────────────────────────────────╮",
            "│ ◐ t2  map Gemini hook events to status   │",
            "│ GOAL                                     │",
            "│ brief     Map each Gemini hook event to  │",
            "│           a status.                      │",
            "│           SubagentStop pairs with its    │",
            "│           … (b: more)                    │",
            "│ done when ☐ every event maps             │",
            "│           ☐ stop marks idle              │",
            "│ STATUS                                   │",
            "│ stage     in review                      │",
            "╰──────────────────────────────────────────╯",
        ]
    );
}

/// Decision 25's tall step: eighteen rows at 40.
#[test]
fn task_panel_renders_at_120x40() {
    let rows = task_panel(120, 40);
    assert_eq!(rows.len(), usize::from(RUN_INSPECTOR_TALL_HEIGHT));
    let inner: Vec<&str> = rows[1..rows.len() - 1]
        .iter()
        .map(|row| row.trim_start_matches("│ ").trim_end_matches(['│', ' ']))
        .collect();
    assert_eq!(
        inner,
        [
            "◐ t2  map Gemini hook events to status                  M · tdd · review round 2",
            "GOAL",
            "brief     Map each Gemini hook event to a status.",
            "          SubagentStop pairs with its start.",
            "          Stop marks the agent idle.",
            "          … (b: more)",
            "done when ☐ every event maps",
            "          ☐ stop marks idle",
            "STATUS",
            "stage     in review",
            "worker    worker #1 · codex · 26m · 41 tool calls",
            "check     ✓ passed · test result: ok",
            "review    in review · round 2",
            "stages    done ✓ → proof ✓ → check ✓ → review ● → merge ·",
            "route     codex · standard · high effort → reviewer claude · frontier",
            "deps      waits on t0 ✓ t6 ✓ · unblocks t3, t7 · on critical path",
        ]
    );
    assert!(rows[0].starts_with('╭') && rows[17].starts_with('╰'));
}
