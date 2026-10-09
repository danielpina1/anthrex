//! Milestone 9.9.9 (OFA §4.6) and the final review's M-2: the Alerts view's quiet
//! footer, one dim line per shown run whose orchestrator handled something.

use super::super::alerts::fixture::{app_of, named, three_runs, three_runs_snapshot};
use super::tests::{chord, draw_at, press, rows_of};
use crate::app::App;
use crate::theme::{self, Role};
use crate::tree::run_fixtures::pty;
use crate::ui::audit;
use crossterm::event::{KeyCode, KeyModifiers};
use proto::{RunState, Status};

/// Milestone 9.9.9 (OFA §4.6): a run whose orchestrator handled something is named in a
/// quiet footer below the list; it is no alert and never selectable.
#[test]
fn the_footer_names_each_run_that_was_handled() {
    let mut runs = three_runs_snapshot();
    let mut orch = crate::tree::orch_fixtures::orchestrator_info(None);
    orch.live = false;
    orch.handled_total = 1;
    runs[1].orchestrator = Some(orch);
    let mut app = app_of(vec![pty(1, "shell", "/tmp/repo", Status::Idle)], runs);
    let before = crate::app::alerts(&app).len();
    chord(&mut app, 'a');
    let footer = "Add mul(): orchestrator handled 1 · o on its alerts opens the run";
    let rows = rows_of(&app, 120, 40);
    assert_eq!(rows.last().map(String::as_str), Some(footer), "{rows:#?}");
    let (buffer, _) = draw_at(&app, 120, 40);
    let (x, y) = audit::find(&buffer, "Add mul(): orchestrator")[0];
    let muted = theme::role(Role::Muted, app.palette()).fg;
    assert_eq!(Some(buffer[(x, y)].fg), muted);

    // Never selectable: `j` past the last alert stays on it.
    for _ in 0..before + 3 {
        press(&mut app, KeyCode::Char('j'), KeyModifiers::NONE);
    }
    assert_eq!(crate::app::alerts(&app).len(), before);
    assert_eq!(app.alerts_focus.as_ref().unwrap().at, before - 1);

    // None for a run with 0 handled.
    let app = three_runs();
    let mut app = app;
    chord(&mut app, 'a');
    let rows = rows_of(&app, 120, 40);
    assert!(
        !rows.iter().any(|l| l.contains("orchestrator handled")),
        "{rows:#?}"
    );
}

/// `three_runs` plus `count` running runs, each with `handled` things handled, created
/// after the fixture's and titled `Handled <i>`.
fn app_with_handled(count: usize, state: RunState) -> App {
    let mut runs = three_runs_snapshot();
    for i in 0..count {
        let mut run = named(
            &format!("h-{i}"),
            &format!("Handled {i}"),
            state,
            100 + i as u64,
        );
        let mut orch = crate::tree::orch_fixtures::orchestrator_info(None);
        orch.live = false;
        orch.handled_total = 2;
        run.orchestrator = Some(orch);
        runs.push(run);
    }
    let mut app = app_of(vec![pty(1, "shell", "/tmp/repo", Status::Idle)], runs);
    chord(&mut app, 'a');
    app
}

/// Final review M-2: the footer is one line per *shown* run, in `shown_runs` order; a
/// terminal run the daemon still lists, handled or not, has none.
#[test]
fn the_footer_follows_shown_runs() {
    let mut app = app_with_handled(0, RunState::Running);
    let mut runs = app.runs.runs.clone();
    for (id, goal, state, at) in [
        ("late", "Late", RunState::Running, 50),
        ("done", "Accepted one", RunState::Accepted, 40),
        ("gone", "Discarded one", RunState::Discarded, 41),
        ("early", "Early", RunState::Running, 10),
    ] {
        let mut run = named(id, goal, state, at);
        let mut orch = crate::tree::orch_fixtures::orchestrator_info(None);
        orch.live = false;
        orch.handled_total = 3;
        run.orchestrator = Some(orch);
        runs.push(run);
    }
    let mut app2 = app_of(vec![pty(1, "shell", "/tmp/repo", Status::Idle)], runs);
    chord(&mut app2, 'a');
    std::mem::swap(&mut app, &mut app2);
    let rows = rows_of(&app, 120, 40);
    let lines: Vec<&String> = rows
        .iter()
        .filter(|l| l.contains("orchestrator handled"))
        .collect();
    assert_eq!(lines.len(), 2, "{rows:#?}");
    assert!(lines[0].starts_with("Early:"), "{lines:?}");
    assert!(lines[1].starts_with("Late:"), "{lines:?}");
    assert!(
        !rows
            .iter()
            .any(|l| l.contains("Accepted one") || l.contains("Discarded one"))
    );
}

/// Task 9 M4: more footer lines than rows end in `+N more`, the last visible row.
#[test]
fn an_overflowing_footer_ends_in_plus_n_more() {
    let app = app_with_handled(30, RunState::Running);
    let rows = rows_of(&app, 80, 24);
    let shown = rows
        .iter()
        .filter(|l| l.contains("orchestrator handled"))
        .count();
    let last = rows.last().expect("rows");
    let more = last.strip_prefix("+").and_then(|r| r.strip_suffix(" more"));
    let n: usize = more.and_then(|n| n.parse().ok()).unwrap_or_else(|| {
        panic!("last row is {last:?}: {rows:#?}");
    });
    assert_eq!(shown + n, 30, "{rows:#?}");
}

/// Task 9 M3: at 80x24 with several handled runs the footer stays inside the frame,
/// the list and detail keep their rows, and the footer never overlaps either.
#[test]
fn several_handled_runs_leave_the_list_and_detail_usable_at_80x24() {
    let mut app = app_with_handled(4, RunState::Running);
    press(&mut app, KeyCode::Char('j'), KeyModifiers::NONE);
    let (buffer, layout) = draw_at(&app, 80, 24);
    let rows = rows_of(&app, 80, 24);
    let foot: Vec<usize> = (0..rows.len())
        .filter(|&i| rows[i].contains("orchestrator handled"))
        .collect();
    assert_eq!(foot.len(), 4, "{rows:#?}");
    assert_eq!(
        foot,
        (foot[0]..foot[0] + 4).collect::<Vec<_>>(),
        "{rows:#?}"
    );
    assert_eq!(foot[3], rows.len() - 1, "{rows:#?}");
    let body: Vec<&String> = rows[..foot[0]].iter().collect();
    assert!(body.len() >= 3, "{rows:#?}");
    for what in [
        "Docs · 77aa",
        "Add mul() · 0723",
        "Fix CI · 9b1e",
        "which crate owns",
    ] {
        assert!(body.iter().any(|l| l.contains(what)), "{what}: {rows:#?}");
    }
    // The bottom border and the frame's sides survive.
    let m = layout.main;
    let bottom: String = (m.x..m.x + m.width)
        .map(|x| buffer[(x, m.y + m.height - 1)].symbol())
        .collect();
    assert!(
        bottom.starts_with('╰') && bottom.ends_with('╯'),
        "{bottom:?}"
    );
    for y in m.y..m.y + m.height {
        assert!(buffer[(m.x + m.width - 1, y)].symbol() != " ", "{y}");
    }
}

/// Task 9 M3: `detail_scroll` agrees with the drawn detail with a footer present: at
/// its `max` the last detail line shows and no `↓` remains; one row less, `↓` shows.
#[test]
fn detail_scroll_matches_the_drawn_detail_with_a_footer() {
    let mut app = app_with_handled(14, RunState::Running);
    press(&mut app, KeyCode::Char('j'), KeyModifiers::NONE);
    let (_, layout) = draw_at(&app, 80, 24);
    let (max, page) = super::detail_scroll(&app, layout.main).expect("selected");
    assert!(max > 0, "the detail must overflow here to pin anything");
    assert!(page >= 1);
    let mut at = |scroll: u16| {
        app.alerts_focus.as_mut().unwrap().scroll = scroll;
        rows_of(&app, 80, 24)
    };
    let end = at(max);
    assert!(!end.iter().any(|l| l.contains('↓')), "{end:#?}");
    assert!(end.iter().any(|l| l.contains('↑')), "{end:#?}");
    let before = at(max - 1);
    assert!(before.iter().any(|l| l.contains('↓')), "{before:#?}");
    // PgDn from the top lands within [0, max], by the page.
    app.alerts_focus.as_mut().unwrap().scroll = 0;
    app.set_graph_viewport(layout.main);
    press(&mut app, KeyCode::PageDown, KeyModifiers::NONE);
    assert_eq!(app.alerts_focus.as_ref().unwrap().scroll, page.min(max));
}
