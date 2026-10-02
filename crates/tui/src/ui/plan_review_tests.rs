//! M9.0.5.7: the plan review screen (decision 12), rendered with `TestBackend`.

use crate::app::plan_review::BAR;
use crate::app::{App, ReviewTarget};
use crate::safe_text::tests::{first_hostile, hostile_text};
use crate::settings::UiSettings;
use crate::tree::orch_fixtures::hold;
use crate::tree::run_fixtures::{PROJECT, RUN_ID, pty, run, snapshot, task};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use proto::{
    HoldState, Route, RunState, RunsSnapshot, Size, Status, TaskInfo, TaskState, TestMode,
};
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use unicode_width::UnicodeWidthStr;

fn route(runtime: &str, model: &str, strength: &str, effort: &str) -> Route {
    serde_json::from_value(serde_json::json!({
        "runtime": runtime, "model": model, "strength": strength, "effort": effort,
    }))
    .expect("a Route")
}

/// The agent-written title of `t3`: an OSC title sequence, a bidi override and a CR.
const HOSTILE_TITLE: &str = "mail \x1b]0;x\x07tem\u{202E}plate\r";

fn plan_task(id: &str, title: &str, size: Size, wave: u32, deps: &[&str]) -> TaskInfo {
    let mut t = task(id, title, size, TaskState::Pending);
    t.wave = wave;
    t.deps = deps.iter().map(|d| (*d).to_string()).collect();
    t.route = route("claude", "opus", "standard", "medium");
    t.brief = format!("Build {title}.");
    t.owns = vec![format!("src/{id}.rs")];
    t.acceptance = vec![format!("{id} works")];
    t
}

/// Two epics, five tasks in three waves: `t1` (auth, a 40-line brief, notes, a
/// test-mode reason, a review route), `t2` and `t3` after `t1`, `t4` after `t3`
/// (implicitly after `t2`), `t5` after `t2` and `t4`. `t3`'s title is hostile.
pub(super) fn plan() -> RunsSnapshot {
    let mut gate = run(RUN_ID, PROJECT, RunState::AwaitingApproval);
    let mut t1 = plan_task("t1", "reset token model", Size::M, 0, &[]);
    t1.epic = Some("auth".into());
    t1.brief = (1..=40)
        .map(|n| format!("Brief line {n} of the token model."))
        .collect::<Vec<_>>()
        .join("\n");
    t1.owns = vec!["src/token.rs".into(), "src/lib.rs".into()];
    t1.acceptance = vec![
        "tokens expire after 1 h".into(),
        "a used token is refused".into(),
    ];
    t1.test_mode_reason = Some("new pure logic".into());
    t1.notes = vec!["raised to M: touches the schema".into()];
    t1.review_route = Some(route("codex", "gpt-5", "frontier", "high"));
    let mut t2 = plan_task("t2", "reset endpoint", Size::S, 1, &["t1"]);
    t2.epic = Some("auth".into());
    let mut t3 = plan_task("t3", HOSTILE_TITLE, Size::S, 1, &["t1"]);
    t3.epic = Some("mail".into());
    t3.test_mode = TestMode::Check;
    t3.test_mode_reason = Some("templates only".into());
    let mut t4 = plan_task("t4", "mail sender", Size::M, 2, &["t3"]);
    t4.implicit_deps = vec!["t2".into(), "t3".into()];
    t4.epic = Some("mail".into());
    let mut t5 = plan_task("t5", "docs", Size::L, 2, &["t2", "t4"]);
    t5.test_mode = TestMode::None;
    gate.tasks = vec![t1, t2, t3, t4, t5];
    snapshot(10_000, vec![gate])
}

pub(super) fn app_with(snap: RunsSnapshot, target: ReviewTarget) -> App {
    let mut app = App::new(
        vec![pty(1, "shell", PROJECT, Status::Idle)],
        "/tmp".into(),
        UiSettings::default(),
    );
    app.set_terminal_size(80, 24);
    app.on_daemon(proto::DaemonMsg::Run(proto::RunReply::Snapshot(snap)));
    assert!(
        app.open_plan_review(RUN_ID.into(), target).is_empty(),
        "opening sends nothing"
    );
    app
}

pub(super) fn press(app: &mut App, code: KeyCode) {
    assert!(
        app.on_key(KeyEvent::new(code, KeyModifiers::NONE))
            .is_empty()
    );
}

pub(super) fn draw(app: &mut App, width: u16, height: u16) -> Buffer {
    app.set_body_area(Rect::new(0, 0, width, height.saturating_sub(1)));
    let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
    terminal
        .draw(|f| {
            crate::ui::draw(f, app);
        })
        .unwrap();
    terminal.backend().buffer().clone()
}

pub(super) fn rows(buffer: &Buffer) -> Vec<String> {
    let area = buffer.area;
    (0..area.height)
        .map(|y| {
            let mut line = String::new();
            let mut x = 0;
            while x < area.width {
                let cell = &buffer[(x, y)];
                line.push_str(cell.symbol());
                x += 1;
            }
            line.trim_end().to_owned()
        })
        .collect()
}

#[test]
fn review_renders_at_80x24() {
    let mut app = app_with(plan(), ReviewTarget::Gate);
    let got = rows(&draw(&mut app, 80, 24));
    // Milestone 9.0.7 decisions 23–26: the frame, the header (no overlap: every task
    // owns its own files), the list in columns, the labelled detail; no waves line.
    let want = [
        "╭ plan · Add password reset · 3f9a ───────────────────────── awaiting approval ╮",
        "│ 5 tasks · 2 epics · M+S+S+M+L · ~500 calls                                   │",
        "├──────────────────────────────────────────────────────────────────────────────┤",
        "│▌t1 reset token model    cl opus  M tdd                                       │",
        "│ t2 reset endpoint       cl opus  S tdd    after t1                           │",
        "│ t3 mail  ]0;x template  cl opus  S check  after t1                           │",
        "│ t4 mail sender          cl opus  M tdd    after t3, t2 (implied)             │",
        "│ t5 docs                 cl opus  L none   after t2, t4                       │",
        "├──────────────────────────────────────────────────────────────────────────────┤",
        "│ t1  reset token model                                                        │",
        "│ brief      Brief line 1 of the token model.                                  │",
        "│            Brief line 2 of the token model.                                  │",
        "│            Brief line 3 of the token model.                                  │",
        "│            Brief line 4 of the token model.                                  │",
        "│            Brief line 5 of the token model.                                  │",
        "│            Brief line 6 of the token model.                                  │",
        "│            Brief line 7 of the token model.                                  │",
        "│            Brief line 8 of the token model.                                  │",
        "│            Brief line 9 of the token model.                                  │",
        "│            Brief line 10 of the token model.                                 │",
        "│            Brief line 11 of the token model.                                 │",
        "│            Brief line 12 of the token model.                                 │",
        "╰──────────────────────────────────────────────────────────────────────────────╯",
        " PLAN  a approve  x reject  e edit  d drop  j/k task  PgUp/PgDn scroll  esc back",
    ];
    assert_eq!(got, want, "{got:#?}");
}

#[test]
fn review_renders_at_120x40() {
    let mut app = app_with(plan(), ReviewTarget::Gate);
    press(&mut app, KeyCode::Char('j'));
    press(&mut app, KeyCode::Char('j'));
    let got = rows(&draw(&mut app, 120, 40));
    let blank = format!("│{}│", " ".repeat(118));
    let line = |text: &str| format!("│{text:<118}│");
    let mut want = vec![
        format!(
            "╭ plan · Add password reset · 3f9a {} awaiting approval ╮",
            "─".repeat(65)
        ),
        line(" 5 tasks · 2 epics · M+S+S+M+L · ~500 calls"),
        format!("├{}┤", "─".repeat(118)),
        line(" t1 reset token model    cl opus  M tdd"),
        line(" t2 reset endpoint       cl opus  S tdd    after t1"),
        format!(
            "│▌{:<117}│",
            "t3 mail  ]0;x template  cl opus  S check  after t1"
        ),
        line(" t4 mail sender          cl opus  M tdd    after t3, t2 (implied)"),
        line(" t5 docs                 cl opus  L none   after t2, t4"),
        format!("├{}┤", "─".repeat(118)),
        line(" t3  mail  ]0;x template"),
        // The title's CR breaks the brief's line, as `multi_line` reads it.
        line(" brief      Build mail ]0;x template"),
        line("            ."),
        line(" owns       src/t3.rs"),
        line(" done when  ◌ t3 works"),
        line(" test mode  check — templates only"),
        line(" deps       after t1 · unblocks t4"),
        line(" route      claude · opus · standard · medium effort"),
        line(" review     none"),
    ];
    while want.len() < 38 {
        want.push(blank.clone());
    }
    want.push(format!("╰{}╯", "─".repeat(118)));
    want.push(
        " PLAN  a approve  x reject  e edit  d drop  j/k task  PgUp/PgDn scroll  esc back".into(),
    );
    assert_eq!(got, want, "{got:#?}");
}

/// The detail's text on screen row `y`, from its first text column on (milestone
/// 9.0.7 decision 26: the detail is the stacked geometry's last area).
pub(super) fn right_of(app: &App, buffer: &Buffer, y: u16) -> String {
    let body = Rect::new(0, 0, buffer.area.width, buffer.area.height - 1);
    let detail = app.review_layout(body).detail;
    (detail.x + BAR..detail.right())
        .map(|x| buffer[(x, y)].symbol().to_owned())
        .collect::<String>()
        .trim_end()
        .to_owned()
}

#[test]
fn review_scrolls_the_brief() {
    let mut app = app_with(plan(), ReviewTarget::Gate);
    let before = rows(&draw(&mut app, 80, 24));
    press(&mut app, KeyCode::PageDown);
    let buffer = draw(&mut app, 80, 24);
    let after = rows(&buffer);
    assert_eq!(
        after[..9],
        before[..9],
        "the title, the header and the list stay"
    );
    // Thirteen detail rows (9–21), a page of twelve: line 12 of the detail is the brief's
    // twelfth, now at the top.
    assert_eq!(
        right_of(&app, &buffer, 9),
        "           Brief line 12 of the token model."
    );
    assert_eq!(
        right_of(&app, &buffer, 21),
        "           Brief line 24 of the token model."
    );
    // Scrolled to the end, the last section shows and nothing past it.
    for _ in 0..5 {
        press(&mut app, KeyCode::PageDown);
    }
    let buffer = draw(&mut app, 80, 24);
    assert_eq!(
        right_of(&app, &buffer, 21),
        "review     codex · gpt-5 · frontier · high effort"
    );
    assert_eq!(
        right_of(&app, &buffer, 20),
        "route      claude · opus · standard · medium effort"
    );
}

/// A running run whose hold `epic:mail` awaits approval over `t3` and `t4`.
pub(super) fn hold_plan() -> RunsSnapshot {
    let mut snap = plan();
    let run = &mut snap.runs[0];
    run.state = RunState::Running;
    run.tasks[0].state = TaskState::Merged;
    for task in &mut run.tasks[2..4] {
        task.hold = Some("epic:mail".into());
    }
    run.holds = vec![hold("epic:mail", HoldState::Awaiting, &["t3", "t4"])];
    snap
}

#[test]
fn hold_review_lists_only_the_hold_tasks() {
    let mut app = app_with(hold_plan(), ReviewTarget::Hold("epic:mail".into()));
    let got = rows(&draw(&mut app, 120, 40));
    let title = "╭ hold epic:mail · Add password reset · 3f9a ";
    assert_eq!(
        got[0],
        format!("{title}{} awaiting approval ╮", "─".repeat(55))
    );
    let line = |text: &str| format!("│{text:<118}│");
    // The hold's own tasks, sizes and budget; both in epic `mail`.
    assert_eq!(got[1], line(" 2 tasks · 1 epic · S+M · ~200 calls"));
    assert_eq!(
        got[3],
        format!(
            "│▌{:<117}│",
            "t3 mail  ]0;x template  cl opus  S check  after t1"
        )
    );
    assert_eq!(
        got[4],
        line(" t4 mail sender          cl opus  M tdd    after t3, t2 (implied)")
    );
    assert_eq!(got[5], format!("├{}┤", "─".repeat(118)));
    assert_eq!(got[6], line(" t3  mail  ]0;x template"));
    assert_eq!(
        got[39],
        " PLAN  a approve hold  x reject hold  j/k task  PgUp/PgDn scroll  esc back"
    );
}

#[test]
fn review_hints() {
    let mut gate = app_with(plan(), ReviewTarget::Gate);
    let rows_gate = rows(&draw(&mut gate, 160, 30));
    assert_eq!(
        rows_gate[29],
        " PLAN  a approve  x reject  e edit  d drop  j/k task  PgUp/PgDn scroll  esc back"
    );
    let mut held = app_with(hold_plan(), ReviewTarget::Hold("epic:mail".into()));
    let rows_held = rows(&draw(&mut held, 160, 30));
    assert_eq!(
        rows_held[29],
        " PLAN  a approve hold  x reject hold  j/k task  PgUp/PgDn scroll  esc back"
    );
    // Esc: the status bar is the terminal's again.
    press(&mut held, KeyCode::Esc);
    let rows_after = rows(&draw(&mut held, 160, 30));
    assert!(!rows_after[29].contains("PLAN"), "{:?}", rows_after[29]);
}

/// Every hostile character in every agent-written field the review shows.
fn hostile_plan() -> RunsSnapshot {
    let bad = hostile_text();
    let mut snap = plan();
    let run = &mut snap.runs[0];
    run.goal = format!("goal {bad}");
    for task in &mut run.tasks {
        task.title = format!("{bad}{}", task.title);
        task.brief = format!("{bad}\n{bad}\r\n{}", task.brief);
        task.owns.push(format!("src/{bad}.rs"));
        task.acceptance.push(bad.clone());
        task.notes.push(bad.clone());
        task.test_mode_reason = Some(bad.clone());
        task.route.model = format!("m{bad}");
        if let Some(review) = &mut task.review_route {
            review.model = format!("r{bad}");
        }
        // Review finding 2: ids and deps are agent-written too; short, so a row shows
        // them before it is cut.
        task.id = hostile_id(&task.id);
        task.deps = task.deps.iter().map(|d| hostile_id(d)).collect();
        task.implicit_deps = task.implicit_deps.iter().map(|d| hostile_id(d)).collect();
    }
    snap
}

/// `id` with a CSI sequence, a bidi override, a CR and a NUL in it.
pub(super) fn hostile_id(id: &str) -> String {
    format!("{id}\x1b[2J\u{202E}\r\0x")
}

/// Every span the review builds, before ratatui sees it (ratatui itself skips some
/// zero-width characters, so the buffer alone would hide a missing sanitiser), holds
/// no hostile character and fits its row.
pub(super) fn assert_spans_clean(app: &App, body: Rect, what: &str) {
    for placed in super::placed(app, body) {
        let text: String = placed
            .line
            .spans
            .iter()
            .map(|s| s.content.as_ref())
            .collect();
        assert_eq!(first_hostile(&text), None, "{what}: {text:?}");
        assert!(
            text.width() <= usize::from(placed.area.width),
            "{what}: {text:?} is wider than {}",
            placed.area.width
        );
    }
}

pub(super) fn assert_clean(buffer: &Buffer, what: &str) {
    for y in 0..buffer.area.height {
        for x in 0..buffer.area.width {
            let symbol = buffer[(x, y)].symbol();
            assert_eq!(
                first_hostile(symbol),
                None,
                "{what}: a hostile character at ({x}, {y})"
            );
        }
    }
}

#[test]
fn review_text_is_sanitised() {
    for (width, height) in [(80, 24), (120, 40)] {
        let mut app = app_with(hostile_plan(), ReviewTarget::Gate);
        for task in 0..5 {
            for page in 0..12 {
                let buffer = draw(&mut app, width, height);
                assert!(
                    rows(&buffer)[0].starts_with("╭ plan · "),
                    "the review shows"
                );
                let what = format!("{width}x{height} task {task} page {page}");
                assert_clean(&buffer, &what);
                assert_spans_clean(&app, Rect::new(0, 0, width, height - 1), &what);
                press(&mut app, KeyCode::PageDown);
            }
            press(&mut app, KeyCode::Char('j'));
        }
    }
    // A hold's id is agent-written too.
    let mut snap = hostile_plan();
    let run = &mut snap.runs[0];
    run.state = RunState::Running;
    let id = format!("epic:{}", hostile_text());
    run.tasks[2].hold = Some(id.clone());
    let t3 = run.tasks[2].id.clone();
    run.holds = vec![hold(&id, HoldState::Awaiting, &[t3.as_str()])];
    let mut app = app_with(snap, ReviewTarget::Hold(id));
    let buffer = draw(&mut app, 120, 40);
    assert!(
        rows(&buffer)[0].starts_with("╭ hold epic:"),
        "the hold review shows"
    );
    assert_clean(&buffer, "hold review");
    assert_spans_clean(&app, Rect::new(0, 0, 120, 39), "hold review");
}

/// Long unbroken words, wide characters and many tasks: nothing crosses the frame, and
/// the selection stays in view.
#[test]
fn nothing_overflows_a_pane() {
    let mut snap = plan();
    let run = &mut snap.runs[0];
    run.goal = "G".repeat(300);
    let t1 = &mut run.tasks[0];
    t1.title = format!("{}{}", "W".repeat(200), "界".repeat(100));
    t1.brief = format!("{} {}", "x".repeat(500), "界".repeat(300));
    t1.owns = vec![format!("src/{}.rs", "deep/".repeat(60))];
    t1.route.model = "界".repeat(80);
    for n in 6..40 {
        run.tasks
            .push(plan_task(&format!("t{n}"), "more", Size::S, 3, &["t5"]));
    }
    for (width, height) in [(80, 24), (120, 40), (61, 12)] {
        let mut app = app_with(snap.clone(), ReviewTarget::Gate);
        let body = Rect::new(0, 0, width, height - 1);
        for step in 0..40 {
            let buffer = draw(&mut app, width, height);
            assert_spans_clean(&app, body, &format!("{width}x{height} step {step}"));
            let selected = app.plan_review.as_ref().unwrap().selected.clone().unwrap();
            let shown = rows(&buffer)
                .iter()
                .any(|row| row.starts_with(&format!("│▌{selected} ")));
            assert!(shown, "{width}x{height}: {selected} is in view");
            // Nothing crosses the frame: both sides whole on every interior row.
            for y in 1..height - 2 {
                for x in [0, width - 1] {
                    let side = buffer[(x, y)].symbol();
                    assert!(
                        ["│", "├", "┤"].contains(&side),
                        "{width}x{height} step {step}: {side:?} at ({x}, {y})"
                    );
                }
            }
            press(&mut app, KeyCode::Char('j'));
            press(&mut app, KeyCode::PageDown);
        }
        // The header row is the frame's first, whatever the list holds.
        let buffer = draw(&mut app, width, height);
        let header = &rows(&buffer)[1];
        assert!(header.starts_with("│ 39 tasks · 2 epics · "), "{header:?}");
    }
}

#[test]
fn an_empty_acceptance_and_owns_read_none() {
    let mut snap = plan();
    snap.runs[0].tasks[1].acceptance.clear();
    snap.runs[0].tasks[1].owns.clear();
    let mut app = app_with(snap, ReviewTarget::Gate);
    press(&mut app, KeyCode::Char('j'));
    let buffer = draw(&mut app, 120, 40);
    let right: Vec<String> = (1..39).map(|y| right_of(&app, &buffer, y)).collect();
    assert!(right.contains(&"owns       none".to_owned()), "{right:#?}");
    assert!(right.contains(&"done when  none".to_owned()), "{right:#?}");
}

#[test]
fn no_panic_at_tiny_sizes() {
    for (width, height) in [(20, 5), (1, 1), (1, 2), (2, 3), (31, 2), (40, 3)] {
        for target in [ReviewTarget::Gate, ReviewTarget::Hold("epic:mail".into())] {
            let snap = if target == ReviewTarget::Gate {
                plan()
            } else {
                hold_plan()
            };
            let mut app = app_with(snap, target);
            draw(&mut app, width, height);
            press(&mut app, KeyCode::PageDown);
            press(&mut app, KeyCode::Char('j'));
            draw(&mut app, width, height);
        }
    }
}

/// Review finding 3: a detail whose scroll is past its end at draw time (the body grew
/// since the last key) still shows its last rows at the bottom, not blank rows.
#[test]
fn a_scroll_past_the_end_draws_the_end() {
    let mut app = app_with(plan(), ReviewTarget::Gate);
    draw(&mut app, 80, 24);
    for _ in 0..10 {
        press(&mut app, KeyCode::PageDown);
    }
    // The 80×24 end, drawn at 120×40 with no key or snapshot in between.
    let mut terminal = Terminal::new(TestBackend::new(120, 40)).unwrap();
    terminal
        .draw(|f| {
            crate::ui::draw(f, &app);
        })
        .unwrap();
    let buffer = terminal.backend().buffer().clone();
    assert_eq!(
        right_of(&app, &buffer, 37),
        "review     codex · gpt-5 · frontier · high effort"
    );
    assert_eq!(
        right_of(&app, &buffer, 36),
        "route      claude · opus · standard · medium effort"
    );
}

/// Review finding 3: the brief shrinks while it is scrolled to its end; the next frame
/// shows the new end.
#[test]
fn a_shrunk_detail_draws_its_new_end() {
    let mut app = app_with(plan(), ReviewTarget::Gate);
    draw(&mut app, 80, 24);
    for _ in 0..10 {
        press(&mut app, KeyCode::PageDown);
    }
    // Five brief lines: fifteen detail rows against the thirteen shown.
    let mut snap = plan();
    snap.runs[0].tasks[0].brief = (1..=5)
        .map(|n| format!("short {n}"))
        .collect::<Vec<_>>()
        .join("\n");
    app.on_daemon(proto::DaemonMsg::Run(proto::RunReply::Snapshot(snap)));
    let buffer = draw(&mut app, 80, 24);
    assert_eq!(
        right_of(&app, &buffer, 21),
        "review     codex · gpt-5 · frontier · high effort"
    );
    assert_eq!(right_of(&app, &buffer, 10), "           short 3");
    // Two rows past the detail now, so one page up shows the brief's first line.
    press(&mut app, KeyCode::PageUp);
    let buffer = draw(&mut app, 80, 24);
    assert_eq!(right_of(&app, &buffer, 10), "brief      short 1");
}

#[path = "plan_review_stages_tests.rs"]
mod stages;
