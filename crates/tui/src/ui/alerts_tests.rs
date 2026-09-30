//! M9.0.5.9: the Alerts box (decisions 19 and 20) and the status bar's flag, rendered
//! with `TestBackend`. The fixture is task 8's (`tree::alert_fixtures::every_source`):
//! ten alerts, three of priority 1.

use super::*;
use crate::safe_text::tests::{first_hostile, hostile_text};
use crate::settings::UiSettings;
use crate::tree::alert_fixtures::{at, blocked, every_source};
use crate::tree::run_fixtures::{pty, snapshot};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use proto::{BlockReason, ProposalAlertInfo, RunState, RunsSnapshot, Status, WindowInfo};
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Color;

fn app_with(snap: RunsSnapshot, windows: Vec<WindowInfo>) -> App {
    let mut app = App::new(windows, "/tmp".into(), UiSettings::default());
    app.set_terminal_size(80, 24);
    app.on_daemon(proto::DaemonMsg::Run(proto::RunReply::Snapshot(snap)));
    app
}

fn every_app() -> App {
    let (snap, windows) = every_source();
    app_with(snap, windows)
}

fn empty_app() -> App {
    app_with(
        snapshot(1, vec![]),
        vec![pty(1, "shell", "/r/demo", Status::Idle)],
    )
}

fn key(app: &mut App, code: KeyCode, mods: KeyModifiers) {
    let _ = app.on_key(KeyEvent::new(code, mods));
}

fn focus(app: &mut App) {
    key(app, KeyCode::Char('b'), KeyModifiers::CONTROL);
    key(app, KeyCode::Char('a'), KeyModifiers::NONE);
    assert!(app.alerts_focus.is_some());
}

fn draw_at(app: &App, width: u16, height: u16) -> (Buffer, crate::ui::Layout) {
    let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
    let mut layout = None;
    terminal
        .draw(|f| layout = Some(crate::ui::draw(f, app)))
        .unwrap();
    (terminal.backend().buffer().clone(), layout.unwrap())
}

/// The cells of `rect` on row `y`, trailing spaces kept.
fn text_in(buffer: &Buffer, rect: Rect, y: u16) -> String {
    (rect.x..rect.x + rect.width)
        .map(|x| buffer[(x, y)].symbol())
        .collect()
}

/// The Alerts box's rows, as drawn.
fn box_rows(buffer: &Buffer, layout: &crate::ui::Layout) -> Vec<String> {
    let area = layout.alerts;
    (area.y..area.y + area.height)
        .map(|y| text_in(buffer, area, y))
        .collect()
}

fn row(buffer: &Buffer, y: u16) -> String {
    let area = buffer.area;
    text_in(buffer, Rect::new(0, 0, area.width, area.height), y)
        .trim_end()
        .to_owned()
}

#[test]
fn empty_alerts_collapse_to_one_line_at_80x24() {
    let app = empty_app();
    let (buffer, layout) = draw_at(&app, 80, 24);
    assert_eq!(layout.alerts, Rect::new(0, 20, 34, 3));
    assert_eq!(
        box_rows(&buffer, &layout),
        [
            "╭ Alerts (0) ────────────────────╮",
            "│no alerts                       │",
            "╰────────────────────────────────╯",
        ]
    );
    assert_eq!(buffer[(1, 21)].style().fg, Some(theme::DIM));
}

#[test]
fn empty_alerts_collapse_to_one_line_at_120x40() {
    let app = empty_app();
    let (buffer, layout) = draw_at(&app, 120, 40);
    assert_eq!(layout.alerts, Rect::new(0, 36, 34, 3));
    assert_eq!(
        box_rows(&buffer, &layout),
        [
            "╭ Alerts (0) ────────────────────╮",
            "│no alerts                       │",
            "╰────────────────────────────────╯",
        ]
    );
}

/// Each alert row's `●` colour, top to bottom.
fn dot_colours(buffer: &Buffer, layout: &crate::ui::Layout) -> Vec<Option<Color>> {
    let inner = layout.alerts_inner;
    (inner.y..inner.y + inner.height)
        .filter(|y| buffer[(inner.x, *y)].symbol() == "●")
        .map(|y| buffer[(inner.x, y)].style().fg)
        .collect()
}

#[test]
fn four_priorities_render_at_80x24() {
    let app = every_app();
    let (buffer, layout) = draw_at(&app, 80, 24);
    // 23 rows of column: max(1, 23 / 3 − 2) = 5 rows, the last `+6 more`.
    assert_eq!(
        box_rows(&buffer, &layout),
        [
            "╭ Alerts (10) ───────────────────╮",
            "│● a-attn  orchestrator asks for…│",
            "│● b-gate  orchestrator waits at…│",
            "│● c-held  orchestrator wake-up …│",
            "│● b-gate  plan awaits approval …│",
            "│+6 more                         │",
            "╰────────────────────────────────╯",
        ]
    );
    let red = Some(Color::Red);
    assert_eq!(
        dot_colours(&buffer, &layout),
        [red, red, red, Some(Color::Yellow)]
    );
    // The label shares the dot's colour; the text is the default colour.
    let y = layout.alerts_inner.y;
    assert_eq!(buffer[(layout.alerts_inner.x + 2, y)].style().fg, red);
    assert_eq!(
        buffer[(layout.alerts_inner.x + 10, y)].style().fg,
        Some(Color::Reset)
    );
}

#[test]
fn four_priorities_render_at_120x40() {
    let app = every_app();
    let (buffer, layout) = draw_at(&app, 120, 40);
    // 39 rows of column: min(10, 6, 11) = 6 rows, the last `+5 more`.
    assert_eq!(
        box_rows(&buffer, &layout),
        [
            "╭ Alerts (10) ───────────────────╮",
            "│● a-attn  orchestrator asks for…│",
            "│● b-gate  orchestrator waits at…│",
            "│● c-held  orchestrator wake-up …│",
            "│● b-gate  plan awaits approval …│",
            "│● c-held  hold epic:ui awaits a…│",
            "│+5 more                         │",
            "╰────────────────────────────────╯",
        ]
    );
    let (red, yellow) = (Some(Color::Red), Some(Color::Yellow));
    assert_eq!(
        dot_colours(&buffer, &layout),
        [red, red, red, yellow, yellow]
    );
    // Focused at the last alert, the window shows the last six: P2, P3 ×3, P4 ×2.
    let mut app = every_app();
    focus(&mut app);
    for _ in 0..9 {
        key(&mut app, KeyCode::Char('j'), KeyModifiers::NONE);
    }
    let (buffer, layout) = draw_at(&app, 120, 40);
    let (magenta, green) = (Some(Color::Magenta), Some(Color::Green));
    assert_eq!(
        dot_colours(&buffer, &layout),
        [yellow, magenta, magenta, magenta, green, green]
    );
}

#[test]
fn more_alerts_than_rows_shows_more_and_scrolls_when_focused() {
    let mut app = every_app();
    focus(&mut app);
    // Focused, the first five rows are alerts and no `+k more` shows.
    let (buffer, layout) = draw_at(&app, 80, 24);
    let rows = box_rows(&buffer, &layout);
    assert!(rows.iter().all(|r| !r.contains("more")), "{rows:#?}");
    assert_eq!(rows[1], "│● a-attn  orchestrator asks for…│");
    // The selection walks down; the rows scroll to keep it in view, on the last row.
    for step in 1..10 {
        key(&mut app, KeyCode::Char('j'), KeyModifiers::NONE);
        let (buffer, layout) = draw_at(&app, 80, 24);
        let inner = layout.alerts_inner;
        let reversed: Vec<u16> = (inner.y..inner.y + inner.height)
            .filter(|y| {
                buffer[(inner.x, *y)]
                    .modifier
                    .contains(ratatui::style::Modifier::REVERSED)
            })
            .collect();
        let expected = inner.y + (step as u16).min(inner.height - 1);
        assert_eq!(reversed, [expected], "step {step}");
    }
    let (buffer, layout) = draw_at(&app, 80, 24);
    assert_eq!(
        box_rows(&buffer, &layout),
        [
            "╭ Alerts (10) ───────────────────╮",
            "│● c-held  t1 blocked (human): n…│",
            "│● d-bare  t1 blocked (question)…│",
            "│● e-halt  run halted: disk full │",
            "│● f-done  ready to accept · 2/2…│",
            "│● shop  profile proposal ready  │",
            "╰────────────────────────────────╯",
        ]
    );
}

#[test]
fn the_focused_box_has_the_focused_border_and_reversed_selection() {
    let mut app = every_app();
    let (buffer, layout) = draw_at(&app, 80, 24);
    let corner = (layout.alerts.x, layout.alerts.y);
    assert_eq!(buffer[corner].style().fg, theme::border().fg);
    focus(&mut app);
    key(&mut app, KeyCode::Char('j'), KeyModifiers::NONE);
    let (buffer, layout) = draw_at(&app, 80, 24);
    assert_eq!(buffer[corner].style().fg, Some(app.settings.accent));
    let inner = layout.alerts_inner;
    for x in inner.x..inner.x + inner.width - 1 {
        assert!(
            buffer[(x, inner.y + 1)]
                .modifier
                .contains(ratatui::style::Modifier::REVERSED),
            "cell ({x}, {}) of the selected row",
            inner.y + 1
        );
        assert!(
            !buffer[(x, inner.y)]
                .modifier
                .contains(ratatui::style::Modifier::REVERSED)
        );
    }
    // The status bar says so.
    assert_eq!(row(&buffer, 23), " ALERTS  j/k move  ⏎ go  esc back");
}

#[test]
fn hidden_sidebar_shows_the_flag() {
    let mut app = every_app();
    app.sidebar_visible = false;
    let (buffer, layout) = draw_at(&app, 80, 24);
    assert_eq!(layout.alerts.height, 0);
    assert!(
        row(&buffer, 23).starts_with(" ⚑ 10 "),
        "{:?}",
        row(&buffer, 23)
    );
    assert_eq!(buffer[(1, 23)].style().fg, Some(Color::Red));
    // A lower top priority takes its colour.
    let mut snap = snapshot(1, vec![at("r", RunState::Halted, 1)]);
    snap.proposals = vec![];
    let mut app = app_with(snap, vec![pty(1, "shell", "/r/demo", Status::Idle)]);
    app.sidebar_visible = false;
    let (buffer, _) = draw_at(&app, 80, 24);
    assert!(
        row(&buffer, 23).starts_with(" ⚑ 1 "),
        "{:?}",
        row(&buffer, 23)
    );
    assert_eq!(buffer[(1, 23)].style().fg, Some(Color::Magenta));
    // No flag at zero, nor while the sidebar shows.
    let mut app = empty_app();
    app.sidebar_visible = false;
    let (buffer, _) = draw_at(&app, 80, 24);
    assert!(!row(&buffer, 23).contains('⚑'), "{:?}", row(&buffer, 23));
    let (buffer, _) = draw_at(&every_app(), 80, 24);
    assert!(!row(&buffer, 23).contains('⚑'), "{:?}", row(&buffer, 23));
}

#[test]
fn the_agent_list_keeps_its_rows_above_the_box() {
    for (width, height) in [(80, 24), (120, 40), (80, 12)] {
        let app = every_app();
        let (buffer, layout) = draw_at(&app, width, height);
        // The agents block ends on the row above the box's top border.
        assert_eq!(layout.sidebar.y + layout.sidebar.height, layout.alerts.y);
        let bottom = layout.sidebar.y + layout.sidebar.height - 1;
        assert!(row(&buffer, bottom).starts_with('╰'), "{width}x{height}");
        assert!(row(&buffer, layout.alerts.y).starts_with("╭ Alerts"));
        // The list and its footer sit inside the agents block.
        let list = layout.sidebar_list;
        assert!(list.y + list.height <= layout.sidebar_footer.y);
        assert!(layout.sidebar_footer.y < bottom);
        assert!(row(&buffer, layout.sidebar_footer.y).contains("4 agents"));
    }
}

/// Hostile text in every run id, block text, halted reason and proposal path.
fn hostile_app() -> App {
    let bad = hostile_text();
    let mut halted = at(&format!("h{bad}"), RunState::Halted, 1);
    halted.halted_reason = Some(bad.clone());
    let mut bare = at(&format!("b{bad}"), RunState::Running, 2);
    bare.tasks = vec![blocked(&format!("t{bad}"), BlockReason::Human, &bad)];
    let mut snap = snapshot(1, vec![halted, bare]);
    snap.proposals = vec![ProposalAlertInfo {
        project: format!("/r/p{bad}").into(),
        updated_at: 1,
    }];
    app_with(snap, vec![pty(1, "shell", "/r/demo", Status::Idle)])
}

/// Every span of the box's lines is clean and fits the interior.
fn assert_lines_clean(app: &App, width: u16, rows: u16, what: &str) {
    for line in super::lines(app, width, rows) {
        let text: String = line.spans.iter().map(|s| s.content.as_ref()).collect();
        assert_eq!(first_hostile(&text), None, "{what}: {text:?}");
        assert!(
            unicode_width::UnicodeWidthStr::width(text.as_str()) <= usize::from(width),
            "{what}: {text:?} is wider than {width}"
        );
    }
    assert!(super::lines(app, width, rows).len() <= usize::from(rows));
}

#[test]
fn alert_text_is_sanitised() {
    let mut app = hostile_app();
    for focused in [false, true] {
        if focused {
            focus(&mut app);
        }
        for (width, height) in [(80, 24), (120, 40)] {
            let (buffer, layout) = draw_at(&app, width, height);
            for y in layout.alerts.y..layout.alerts.y + layout.alerts.height {
                let text = text_in(&buffer, layout.alerts, y);
                assert_eq!(first_hostile(&text), None, "{text:?}");
            }
            let inner = layout.alerts_inner;
            assert_lines_clean(&app, inner.width, inner.height, "hostile");
            assert!(box_rows(&buffer, &layout)[1].starts_with("│● "), "drawn");
        }
    }
}

#[test]
fn nothing_overflows_the_box() {
    let mut app = every_app();
    for focused in [false, true] {
        if focused {
            focus(&mut app);
        }
        for width in 0..40 {
            for rows in 0..8 {
                assert_lines_clean(&app, width, rows, &format!("{width}x{rows}"));
            }
        }
    }
}

#[test]
fn no_panic_at_tiny_sizes() {
    for sidebar in [0, 1, 2, 3, 24] {
        for (width, height) in [(1, 1), (2, 2), (20, 5), (40, 3), (12, 4)] {
            let mut app = every_app();
            app.sidebar_width = sidebar;
            draw_at(&app, width, height);
            focus(&mut app);
            draw_at(&app, width, height);
            let mut app = empty_app();
            app.sidebar_width = sidebar;
            draw_at(&app, width, height);
        }
    }
}

/// Decision 27: the box sanitises again when it draws, whatever `app::alerts` did.
#[test]
fn a_raw_alert_line_is_sanitised_by_the_box_itself() {
    let bad = hostile_text();
    let alert = crate::app::Alert {
        priority: 3,
        key: crate::app::AlertKey::Halted("r".into()),
        label: format!("r{bad}"),
        text: format!("t{bad}"),
    };
    for width in [4, 20, 400] {
        for selected in [false, true] {
            let line = super::alert_line(&alert, width, selected);
            let text: String = line.spans.iter().map(|s| s.content.as_ref()).collect();
            assert_eq!(first_hostile(&text), None, "{width}: {text:?}");
        }
    }
}

/// Review: the status bar's badge and hint precedence, PREFIX > REVIEW > ALERTS >
/// TREE. `C-b a` from tree mode (the overview, the run view) keeps `tree_input` set.
#[test]
fn the_status_bar_precedence_is_prefix_review_alerts_tree() {
    let mut app = every_app();
    key(&mut app, KeyCode::Char('b'), KeyModifiers::CONTROL);
    key(&mut app, KeyCode::Char('t'), KeyModifiers::NONE);
    assert!(app.tree_input.is_some());
    focus(&mut app);
    assert!(app.tree_input.is_some(), "tree mode stays under the box");
    let (buffer, _) = draw_at(&app, 120, 24);
    assert_eq!(row(&buffer, 23), " ALERTS  j/k move  ⏎ go  esc back");
    // The prefix pending wins over the box.
    key(&mut app, KeyCode::Char('b'), KeyModifiers::CONTROL);
    let (buffer, _) = draw_at(&app, 120, 24);
    assert!(
        row(&buffer, 23).starts_with(" PREFIX "),
        "{:?}",
        row(&buffer, 23)
    );
    key(&mut app, KeyCode::Esc, KeyModifiers::NONE);
    // The review wins over the box (both set only by a constructed state: the box
    // refuses `C-b a` under the review, and its Enter leaves before opening one).
    app.open_plan_review("b-gate".into(), crate::app::ReviewTarget::Gate);
    assert!(app.alerts_focus.is_some());
    let (buffer, _) = draw_at(&app, 120, 24);
    assert!(
        row(&buffer, 23).starts_with(" REVIEW  a approve  x reject"),
        "{:?}",
        row(&buffer, 23)
    );
}

/// Review: exactly as many alerts as rows show them all, with no `+<k> more`.
#[test]
fn as_many_alerts_as_rows_show_them_all() {
    let runs = (0..6)
        .map(|n| at(&format!("h{n}"), RunState::Halted, n))
        .collect();
    let app = app_with(
        snapshot(1, runs),
        vec![pty(1, "shell", "/r/demo", Status::Idle)],
    );
    let (buffer, layout) = draw_at(&app, 120, 40);
    let rows = box_rows(&buffer, &layout);
    assert_eq!(rows.len(), 8, "{rows:#?}");
    assert!(
        rows[1..7].iter().all(|r| r.starts_with("│● h")),
        "{rows:#?}"
    );
    assert!(rows.iter().all(|r| !r.contains("more")), "{rows:#?}");
}

/// Review: a short terminal still shows one alert row (the formula's `max(1, …)`).
#[test]
fn a_short_terminal_still_shows_one_alert() {
    for height in 5..=9 {
        let app = every_app();
        let (buffer, layout) = draw_at(&app, 80, height);
        let rows = box_rows(&buffer, &layout);
        assert_eq!(rows.len(), 3, "{height}: {rows:#?}");
        assert!(rows[1].starts_with("│● a-attn"), "{height}: {rows:#?}");
    }
}
