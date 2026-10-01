//! M9.0.7.8: the task panel drawn outcome first (decisions 12 and 16): the state word
//! flush right or on its own row, OUTCOME then EVIDENCE then INTENT then DETAIL, the
//! footer pinned outside the scroll, and the borders' scroll and action marks.

use crate::app::App;
use crate::app::task_detail::DetailState;
use crate::inspector::run_task_outcome_tests::{
    in_review, in_review_snapshot, land_detail, t2_key,
};
use crate::inspector::run_tests::{app_of, inspect_node};
use crate::inspector::{Inspection, render};
use crate::safe_text::tests::{first_hostile, hostile_text};
use crate::theme::{self, Palette, Role};
use crate::tree::{self, NodeKey, RunFilter};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use proto::TaskState;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::{Terminal, backend::TestBackend};

fn key(app: &mut App, code: KeyCode, modifiers: KeyModifiers) {
    let _ = app.on_key(KeyEvent::new(code, modifiers));
}

/// `app` in the run view of `r1` with `t2` selected, at `w`×`h`, as `lib::draw` lays
/// it out.
fn run_view(mut app: App, w: u16, h: u16) -> App {
    let _ = app.set_terminal_size(w, h);
    key(&mut app, KeyCode::Char('b'), KeyModifiers::CONTROL);
    key(&mut app, KeyCode::Char('T'), KeyModifiers::NONE);
    let rows = tree::build_with_runs(&app.windows, &app.runs.runs, &app.tree);
    app.tree.select(&rows, NodeKey::Run("r1".into()));
    key(&mut app, KeyCode::Char('l'), KeyModifiers::NONE);
    assert!(app.run_view.is_some(), "the run view opened");
    let rows = tree::run_rows(&app.runs.runs[0], &app.windows, &app.tree, RunFilter::All);
    app.tree.select(&rows, t2_key());
    assert_eq!(app.tree.selected, Some(t2_key()), "t2 is a run-view row");
    lay_out(&mut app, w, h);
    app
}

fn lay_out(app: &mut App, w: u16, h: u16) -> Rect {
    let layout = crate::ui::layout_for(app, Rect::new(0, 0, w, h));
    app.set_tree_viewports(layout.sidebar_list.height, layout.main_inner.height);
    app.set_graph_viewport(layout.main);
    crate::ui::overview::view(app, layout.main).footer
}

/// The panel's rows as drawn in the whole frame, untrimmed.
fn panel(app: &mut App, w: u16, h: u16) -> Vec<String> {
    let area = lay_out(app, w, h);
    let buffer = crate::ui::audit::draw(app, w, h);
    (area.y..area.bottom())
        .map(|y| {
            (area.x..area.right())
                .map(|x| buffer[(x, y)].symbol())
                .collect()
        })
        .collect()
}

/// The panel's interior rows, the border and padding taken off and trimmed.
fn interior(rows: &[String]) -> Vec<String> {
    rows[1..rows.len() - 1]
        .iter()
        .map(|row| {
            row.trim_start_matches("│ ")
                .trim_end_matches(['│', ' '])
                .to_owned()
        })
        .collect()
}

#[test]
fn the_task_panel_renders_outcome_first_at_120x40() {
    let mut app = run_view(in_review(), 120, 40);
    let rows = panel(&mut app, 120, 40);
    assert_eq!(
        interior(&rows),
        [
            "◐ t2  map Gemini hook events to status                            in review · r2",
            "OUTCOME",
            "pipeline  done ✓ › proof ✓ › check ✓ › review › merge ◌",
            "check     ✓ passed · cargo test -p gemini 4.1s",
            "accept    ◌ Stop marks the window idle",
            "          ◌ SubagentStop pairs with Start",
            "EVIDENCE",
            "diff      +142 −18 · 3 files",
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
            "stage     1 of 2",
            "route     codex · standard · high effort → reviewer claude · frontier",
            "history   12:31 review r1 changes · 12:20 check passed · 12:02 started",
            "stage 1 of 2 · cx gpt-6-sol · M · tdd",
        ]
    );
    // Milestone 9.0.7 decision 17: the panel takes all 22 rows it needs, so nothing
    // lies below and the border names only the actions.
    assert_eq!(rows.len(), 24);
    assert!(rows[0].starts_with('╭'), "{}", rows[0]);
    assert!(rows[23].ends_with("─ . actions ╯"), "{}", rows[23]);
}

/// Decision 13: each step's mark in its role, the current step's word bold `Working`.
#[test]
fn the_pipeline_and_the_ticks_are_coloured() {
    let mut app = run_view(in_review(), 120, 40);
    let area = lay_out(&mut app, 120, 40);
    let buffer = crate::ui::audit::draw(&app, 120, 40);
    let p = app.palette();
    let at = |text: &str| {
        let hits = crate::ui::audit::find(&buffer, text);
        let (x, y) = *hits
            .iter()
            .find(|(x, y)| area.contains((*x, *y).into()))
            .unwrap_or_else(|| panic!("{text:?} in the panel"));
        &buffer[(x, y)]
    };
    assert_eq!(at("✓ › proof").fg, theme::role(Role::Done, p).fg.unwrap());
    assert_eq!(at("◌ Stop").fg, theme::role(Role::Muted, p).fg.unwrap());
    let current = at("review › merge");
    assert_eq!(current.fg, theme::role(Role::Working, p).fg.unwrap());
    assert!(current.modifier.contains(ratatui::style::Modifier::BOLD));
    assert_eq!(at("✓ passed").fg, theme::role(Role::Done, p).fg.unwrap());
    // The criterion's own text is the default colour.
    assert_ne!(at("Stop marks").fg, theme::role(Role::Muted, p).fg.unwrap());
}

#[test]
fn at_80_columns_the_state_word_moves_under_the_title() {
    let mut app = run_view(in_review(), 80, 24);
    let rows = interior(&panel(&mut app, 80, 24));
    assert_eq!(rows[0], "◐ t2  map Gemini hook events to status");
    assert_eq!(rows[1], "  in review · r2");
    assert_eq!(rows[2], "OUTCOME");
    assert_eq!(
        rows.last().map(String::as_str),
        Some("stage 1 of 2 · cx gpt-6-sol · M · tdd")
    );
    // Narrower still: the name is cut, the state word kept whole on its row.
    let inspection = inspect_node(&in_review(), &t2_key());
    let drawn = draw(&inspection, 28, 8);
    assert_eq!(drawn[1], "│ ◐ t2  map Gemini hook e… │");
    assert_eq!(drawn[2], "│   in review · r2         │");
}

/// At 80x24, where decision 17's panel is cut at 15 rows (the title on two rows).
#[test]
fn the_footer_stays_while_the_body_scrolls() {
    let mut app = run_view(in_review(), 80, 24);
    let before = panel(&mut app, 80, 24);
    assert_eq!(before.len(), 15);
    let last = before.len() - 2;
    for _ in 0..2 {
        key(&mut app, KeyCode::PageDown, KeyModifiers::NONE);
    }
    assert!(app.inspector_scroll_for(&t2_key()) > 0, "PgDn scrolled");
    let after = panel(&mut app, 80, 24);
    assert_eq!(after[last], before[last], "the footer row is unchanged");
    assert!(after[last].contains("stage 1 of 2 · cx gpt-6-sol · M · tdd"));
    assert_eq!(after[1..3], before[1..3], "the title rows stay");
    assert_ne!(after[3], before[3], "the body moved");
    assert!(after[0].ends_with(" ↑ PgUp ╮"), "{}", after[0]);
    // A page is the body's rows: the title rows and the footer are not scrolled past.
    let footer = lay_out(&mut app, 80, 24);
    let (width, height) = (footer.width - 4, footer.height - 2);
    assert_eq!(
        crate::inspector::task_panel_room(&app, width, height),
        height - 3
    );
}

fn draw_buffer(inspection: &Inspection, w: u16, h: u16) -> Buffer {
    let mut terminal = Terminal::new(TestBackend::new(w, h)).unwrap();
    terminal
        .draw(|frame| render(frame, inspection, Rect::new(0, 0, w, h)))
        .unwrap();
    terminal.backend().buffer().clone()
}

fn draw(inspection: &Inspection, w: u16, h: u16) -> Vec<String> {
    crate::ui::audit::rows(&draw_buffer(inspection, w, h))
}

#[test]
fn borders_show_scroll_and_actions() {
    let mut inspection = inspect_node(&in_review(), &t2_key());
    assert!(inspection.actions, "t2 lists open conversation");
    let rows = draw(&inspection, 84, 12);
    assert!(rows[11].ends_with(" ↓ PgDn · . actions ╯"), "{}", rows[11]);
    assert!(!rows[0].contains("PgUp"), "{}", rows[0]);
    // Keys in the accent, words muted.
    let buffer = draw_buffer(&inspection, 84, 12);
    let cell = |text: &str| {
        let (x, y) = crate::ui::audit::find(&buffer, text)[0];
        buffer[(x, y)].fg
    };
    let p = Palette::PLAIN;
    assert_eq!(cell("↓ PgDn"), theme::role(Role::Accent, p).fg.unwrap());
    assert_eq!(cell(". actions"), theme::role(Role::Accent, p).fg.unwrap());
    assert_eq!(cell("actions ╯"), theme::role(Role::Muted, p).fg.unwrap());
    // Scrolled to the end: the top mark, and the actions alone below.
    inspection.scroll = u16::MAX;
    let rows = draw(&inspection, 84, 12);
    assert!(rows[0].ends_with(" ↑ PgUp ╮"), "{}", rows[0]);
    assert!(rows[11].ends_with("─ . actions ╯"), "{}", rows[11]);
    // Everything fits: ` . actions ` alone.
    inspection.scroll = 0;
    let rows = draw(&inspection, 84, 40);
    assert!(rows[39].ends_with("─ . actions ╯"), "{}", rows[39]);
    assert!(!rows[0].contains("PgUp"));
    // No action: no mark.
    inspection.actions = false;
    let rows = draw(&inspection, 84, 40);
    assert!(rows[39].ends_with("──╯"), "{}", rows[39]);
    // Too narrow for the whole mark: `↓ PgDn` alone, then nothing.
    inspection.actions = true;
    let rows = draw(&inspection, 16, 12);
    assert!(rows[11].ends_with(" ↓ PgDn ╯"), "{}", rows[11]);
    let rows = draw(&inspection, 9, 12);
    assert!(
        rows[11].ends_with("───╯") && !rows[11].contains('↓'),
        "{}",
        rows[11]
    );
    // ASCII: folded marks.
    let mut ascii = in_review();
    ascii.settings.badges.ascii = true;
    let inspection = inspect_node(&ascii, &t2_key());
    let mut terminal = Terminal::new(TestBackend::new(84, 12)).unwrap();
    terminal
        .draw(|frame| {
            crate::inspector::render_in(
                frame,
                &inspection,
                Rect::new(0, 0, 84, 12),
                ascii.palette(),
            )
        })
        .unwrap();
    let rows = crate::ui::audit::rows(terminal.backend().buffer());
    assert!(rows[11].ends_with(" v PgDn - . actions +"), "{}", rows[11]);
    assert_eq!(
        crate::ui::audit::first_non_ascii(terminal.backend().buffer()),
        None
    );
}

/// Review focus 5: criteria, block text, the review's summary and the worker's summary
/// are agent text, asserted on the drawn cells.
#[test]
fn outcome_text_is_sanitised() {
    let hostile = hostile_text();
    let plant = |app: &mut App| {
        if let Some(DetailState::Ready(detail)) = app.task_detail.as_mut().map(|c| &mut c.state) {
            detail.acceptance = vec![format!("CRIT {hostile}"), format!("{hostile}\nCRIT2")];
            detail.worker_summary = Some(format!("SUMMARY {hostile}\n{hostile}"));
        }
    };
    // Blocked: the block's every line in OUTCOME.
    let (mut snapshot, windows) = in_review_snapshot();
    let task = snapshot.runs[0]
        .tasks
        .iter_mut()
        .find(|t| t.id == "t2")
        .unwrap();
    task.state = TaskState::Blocked;
    task.block = Some(proto::BlockInfo {
        reason: proto::BlockReason::Question,
        text: format!("BLOCK {hostile}\n{hostile}"),
    });
    let mut blocked = app_of((snapshot, windows));
    land_detail(&mut blocked, None);
    plant(&mut blocked);
    // Merged by an approval whose summary is hostile: the review row and the result.
    let (mut snapshot, windows) = in_review_snapshot();
    let task = snapshot.runs[0]
        .tasks
        .iter_mut()
        .find(|t| t.id == "t2")
        .unwrap();
    task.state = TaskState::Merged;
    task.merge_commit = Some(hostile.clone());
    task.reviews[1].verdict = Some(proto::Verdict::Approve);
    task.reviews[1].summary = format!("VERDICT {hostile}");
    let mut merged = app_of((snapshot, windows));
    land_detail(&mut merged, None);
    plant(&mut merged);
    for (app, planted) in [
        (&blocked, &["CRIT", "CRIT2", "BLOCK"][..]),
        (&merged, &["CRIT", "VERDICT", "SUMMARY"][..]),
    ] {
        let inspection = inspect_node(app, &t2_key());
        for (w, h) in [(84, 60), (44, 60), (120, 80)] {
            let rows = draw(&inspection, w, h);
            for row in &rows {
                assert_eq!(first_hostile(row), None, "{w}x{h}: {row:?}");
            }
            for text in planted {
                assert!(
                    rows.iter().any(|row| row.contains(text)),
                    "{text} at {w}x{h}: {rows:#?}"
                );
            }
        }
        let footer = inspection.footer.as_deref().unwrap_or_default();
        assert_eq!(first_hostile(footer), None);
    }
}

#[test]
fn no_panic_at_tiny_sizes() {
    let inspection = inspect_node(&in_review(), &t2_key());
    for (w, h) in [
        (0, 0),
        (1, 1),
        (5, 40),
        (20, 5),
        (3, 3),
        (4, 2),
        (40, 3),
        (40, 4),
    ] {
        let rows = draw(&inspection, w, h);
        assert!(rows.len() <= usize::from(h), "{w}x{h}");
    }
    for (w, h) in [(20, 5), (1, 1), (5, 40)] {
        let mut app = run_view(in_review(), 120, 40);
        lay_out(&mut app, w, h);
        let _ = crate::ui::audit::draw(&app, w, h);
        key(&mut app, KeyCode::PageDown, KeyModifiers::NONE);
        key(&mut app, KeyCode::PageUp, KeyModifiers::NONE);
        let _ = crate::ui::audit::draw(&app, w, h);
    }
}
