//! Milestone 9.0.7 task 9, fix round 1: the run view's title against hostile, wide and
//! narrow input (decision 21, Global Constraint 4), the right-hand titles muted, and the
//! reveal's re-split as the selection moves (decision 17, Review focus 3).

use super::polish_tests::{add_mul, app_of, frame, key, run_view, select, staged};
use crate::app::App;
use crate::safe_text::tests::{first_hostile, hostile_text};
use crate::theme::{Role, role};
use crate::tree::NodeKey;
use crate::tree::run_fixtures::RUN_ID;
use crate::ui::audit;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use proto::RunState;
use ratatui::buffer::Buffer;
use ratatui::{Terminal, backend::TestBackend};
use unicode_width::UnicodeWidthStr;

/// The run view on `add-mul-0723` (2/3 merged, approved 14m ago) with `goal`.
fn titled(goal: &str) -> App {
    let (mut snap, windows) = add_mul(RunState::Running);
    snap.runs[0].goal = goal.into();
    run_view(app_of((snap, windows), false), "add-mul-0723")
}

/// The overview alone drawn into a `width`-column area: its top border row.
fn top_at(app: &App, width: u16) -> (Buffer, String) {
    let mut terminal = Terminal::new(TestBackend::new(width, 20)).unwrap();
    terminal
        .draw(|f| crate::ui::overview::render(f, app, f.area()))
        .unwrap();
    let buffer = terminal.backend().buffer().clone();
    // As a reader sees it: the cell a wide character spills into is skipped.
    let mut top = String::new();
    let mut x = 0;
    while x < width {
        let symbol = buffer[(x, 0)].symbol();
        top.push_str(symbol);
        x += u16::try_from(UnicodeWidthStr::width(symbol).max(1)).unwrap();
    }
    (buffer, top.trim_end().to_owned())
}

#[test]
fn a_hostile_goal_is_sanitised_in_the_title() {
    let app = titled(&hostile_text());
    let (_, top) = top_at(&app, 120);
    assert_eq!(first_hostile(&top), None, "{top}");
    assert!(top.contains(" · 0723 "), "{top}");
    assert!(top.ends_with(" 2/3 merged · 14m ╮"), "{top}");
}

/// A wide goal is cut by display width: 120 columns leave the name 91 (the corners,
/// ` run · `, the title's spaces, the right text and one column), so `a` and 41
/// two-column characters, `…`, ` · 0723`, then one border column before the right text.
#[test]
fn a_long_wide_goal_is_cut_by_display_width() {
    let app = titled(&format!("a{}", "漢字".repeat(60)));
    let (_, top) = top_at(&app, 120);
    let name = format!("a{}…", "漢字".repeat(20) + "漢");
    assert!(
        top.ends_with(&format!(" run · {name} · 0723 ─ 2/3 merged · 14m ╮")),
        "{top}"
    );
    assert!(top.starts_with("╭ run · a"), "{top}");
}

/// Below 16 columns of name room the right-hand text goes and the name is cut to the
/// rest: 40 columns leave 29.
#[test]
fn a_narrow_title_drops_the_right_text_first() {
    let app = titled("Add a multiply function to crate a");
    let (_, top) = top_at(&app, 40);
    assert!(!top.contains("merged"), "{top}");
    assert!(
        top.contains(" run · Add a multiply functi… · 0723 "),
        "{top}"
    );
}

/// The cells of the first `text` on `row`, each muted.
fn assert_muted(buffer: &Buffer, row: u16, text: &str, p: crate::theme::Palette) {
    let at = audit::find(buffer, text);
    let &(x, _) = at
        .iter()
        .find(|(_, y)| *y == row)
        .unwrap_or_else(|| panic!("{text:?} on row {row}"));
    let muted = role(Role::Muted, p).fg;
    for cx in x..x + u16::try_from(text.chars().count()).unwrap() {
        assert_eq!(Some(buffer[(cx, row)].fg), muted, "{text:?} cell {cx}");
    }
}

/// The run view's and the Alerts view's right-hand titles are muted, never the accent of
/// the frame they sit on.
#[test]
fn right_hand_titles_are_muted() {
    let mut app = titled("Add mul()");
    let p = app.palette();
    let (buffer, layout) = frame(&mut app, 120, 40);
    assert_muted(&buffer, layout.main.y, " 2/3 merged · 14m ", p);

    let mut alerts = crate::ui::alerts::fixture::three_runs();
    alerts.on_key(KeyEvent::new(KeyCode::Char('b'), KeyModifiers::CONTROL));
    key(&mut alerts, KeyCode::Char('a'));
    let (buffer, layout) = frame(&mut alerts, 120, 40);
    let n = crate::app::alerts(&alerts).len();
    assert_muted(
        &buffer,
        layout.main.y,
        &format!(" 1/{n} "),
        alerts.palette(),
    );
}

/// Review focus 3: a key that moves the selection from a task to a stage re-splits the
/// reducer's canvas at once, before any frame, to the one the next frame draws.
#[test]
fn moving_to_a_stage_resplits_the_canvas_before_the_next_frame() {
    let mut app = staged(|_| {});
    select(
        &mut app,
        NodeKey::Task {
            run: RUN_ID.into(),
            id: "t3".into(),
        },
    );
    let (_, layout) = frame(&mut app, 120, 40);
    let before = app.graph_area;
    key(&mut app, KeyCode::Char('k'));
    assert_eq!(
        app.tree.selected,
        Some(NodeKey::Stage {
            run: RUN_ID.into(),
            n: 2,
        })
    );
    let canvas = crate::ui::overview::view(&app, layout.main).canvas;
    assert_ne!(canvas, before, "the stage's panel is not the task's");
    assert_eq!(app.graph_area, canvas, "no frame drawn in between");
}

/// `add-mul-0723` as round `n` of `n` rounds (2 and over), else as it is.
fn rounds_of(n: u32) -> App {
    let (mut snap, windows) = add_mul(RunState::Running);
    if n > 1 {
        let run = &mut snap.runs[0];
        run.round = n;
        run.rounds = (1..=n)
            .map(|k| proto::RoundInfo {
                n: k,
                goal_head: format!("request {k}"),
                origin: proto::RoundOrigin::User,
                outcome: None,
                summary_head: None,
            })
            .collect();
    }
    run_view(app_of((snap, windows), false), "add-mul-0723")
}

/// Milestone 9.3 decision 32: ` · round <n>` after the name for a run of several
/// rounds; a one-round run's title is unchanged (pinning, with the tests above). At 40
/// columns the round is kept and the goal is cut.
#[test]
fn the_run_title_adds_the_round() {
    let (_, top) = top_at(&rounds_of(1), 120);
    assert!(top.starts_with("╭ run · Add mul() · 0723 ─"), "{top}");
    assert!(top.ends_with(" 2/3 merged · 14m ╮"), "{top}");
    let (_, top) = top_at(&rounds_of(2), 120);
    assert!(
        top.starts_with("╭ run · Add mul() · 0723 · round 2 ─"),
        "{top}"
    );
    assert!(top.ends_with(" 2/3 merged · 14m ╮"), "{top}");
    let mut app = rounds_of(12);
    app.runs.runs[0].goal = "Add a multiply function to crate a".into();
    let (_, top) = top_at(&app, 40);
    assert!(
        top.starts_with("╭ run · Add a mult… · 0723 · round 12 ─╮"),
        "{top}"
    );
}
