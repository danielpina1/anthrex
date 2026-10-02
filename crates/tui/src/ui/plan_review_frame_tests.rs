//! M9.0.7.11: the plan review framed, with its summary header, aligned task columns
//! and labelled detail (decisions 23–26), rendered with `TestBackend`.

use super::tests::{assert_clean, assert_spans_clean, draw, hostile_id, press, rows};
use crate::app::{App, ReviewTarget};
use crate::safe_text::tests::hostile_text;
use crate::settings::UiSettings;
use crate::theme::{self, Role};
use crate::tree::orch_fixtures::hold;
use crate::tree::plan_fixtures::{PLAN_RUN, overlapping_plan, plan_task, route};
use crate::tree::run_fixtures::{PROJECT, pty};
use crate::ui::audit;
use crossterm::event::KeyCode;
use proto::{HoldState, RunState, RunsSnapshot, Runtime, Size, Status};
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Modifier;

fn app_with(snap: RunsSnapshot, target: ReviewTarget) -> App {
    let mut app = App::new(
        vec![pty(1, "shell", PROJECT, Status::Idle)],
        "/tmp".into(),
        UiSettings::default(),
    );
    app.set_terminal_size(80, 24);
    app.on_daemon(proto::DaemonMsg::Run(proto::RunReply::Snapshot(snap)));
    assert!(app.open_plan_review(PLAN_RUN.into(), target).is_empty());
    app
}

/// `inner` between the frame's sides, padded to the interior's width.
fn framed(inner: &str, width: usize) -> String {
    let pad = (width - 2).saturating_sub(unicode_width::UnicodeWidthStr::width(inner));
    format!("│{inner}{}│", " ".repeat(pad))
}

/// The top border: `left` after the corner, ` awaiting approval ` before the other.
fn top(left: &str, width: usize) -> String {
    let right = " awaiting approval ";
    let used = 2 + unicode_width::UnicodeWidthStr::width(left) + right.len();
    format!("╭{left}{}{right}╮", "─".repeat(width - used))
}

fn rule(width: usize) -> String {
    format!("├{}┤", "─".repeat(width - 2))
}

/// The rows decision 23–26 draw for the overlapping plan, `t1` selected, at `width`
/// columns and `height` rows: the frame's interior rows below the detail are blank.
fn want(width: usize, height: usize) -> Vec<String> {
    let mut want = vec![
        top(" plan · Add mul() · 0723 ", width),
        framed(
            " 3 tasks · 2 stages · S+M+S · ~190 calls · critical t1 › t2 › t3",
            width,
        ),
        framed(" ⚠ t2 and t3 both own crates/c/src/lib.rs", width),
        rule(width),
        framed(
            "▌t1 add mul() to a       cx gpt-6-sol  S tdd   stage 1",
            width,
        ),
        framed(
            " t2 report_product in c  cl opus       M tdd   stage 2  after t1",
            width,
        ),
        framed(
            " t3 docs                 cl haiku      S none  stage 2  after t1 (implied)",
            width,
        ),
        rule(width),
        framed(" t1  add mul() to a", width),
        framed(" brief      Build add mul() to a.", width),
        framed(" owns       crates/t1/src/lib.rs", width),
        framed(" done when  ◌ t1 works", width),
        framed(" test mode  tdd", width),
        framed(" stage      1 of 2", width),
        framed(" deps       unblocks t2, t3", width),
        framed(
            " route      codex · gpt-6-sol · standard · medium effort",
            width,
        ),
        framed(" review     none", width),
    ];
    while want.len() < height - 1 {
        want.push(framed("", width));
    }
    want.push(format!("╰{}╯", "─".repeat(width - 2)));
    want
}

#[test]
fn the_review_is_framed_with_its_header_at_80x24() {
    let mut app = app_with(overlapping_plan(), ReviewTarget::Gate);
    let buffer = draw(&mut app, 80, 24);
    let got = rows(&buffer);
    let mut expected = want(80, 23);
    expected.push(
        " PLAN  a approve  x reject  e edit  d drop  j/k task  PgUp/PgDn scroll  esc back".into(),
    );
    assert_eq!(got, expected, "{got:#?}");
    // `t1`'s row: the bar in the accent, the rest of the row reversed to the frame.
    let accent = theme::fg(Role::Accent);
    assert_eq!(buffer[(1, 4)].fg, accent);
    assert!(!buffer[(1, 4)].modifier.contains(Modifier::REVERSED));
    assert!((2..79).all(|x| buffer[(x, 4)].modifier.contains(Modifier::REVERSED)));
    assert!(!buffer[(2, 5)].modifier.contains(Modifier::REVERSED));
    // The warning in `Attention`; the detail's labels muted.
    assert_eq!(buffer[(2, 2)].fg, theme::fg(Role::Attention));
    assert_eq!(buffer[(2, 9)].fg, theme::fg(Role::Muted));
    // The header's first row is cut with `…` to the width.
    let mut narrow = app_with(overlapping_plan(), ReviewTarget::Gate);
    let got = rows(&draw(&mut narrow, 50, 24));
    assert_eq!(got[1], "│ 3 tasks · 2 stages · S+M+S · ~190 calls · crit…│");
}

#[test]
fn the_review_is_framed_with_its_header_at_120x40() {
    let mut app = app_with(overlapping_plan(), ReviewTarget::Gate);
    let got = rows(&draw(&mut app, 120, 40));
    let mut expected = want(120, 39);
    expected.push(
        " PLAN  a approve  x reject  e edit  d drop  j/k task  PgUp/PgDn scroll  esc back".into(),
    );
    assert_eq!(got, expected, "{got:#?}");
}

#[test]
fn the_hold_variant_names_its_hold() {
    let mut snap = overlapping_plan();
    let run = &mut snap.runs[0];
    run.state = RunState::Running;
    for task in &mut run.tasks[1..] {
        task.hold = Some("epic:1".into());
    }
    run.holds = vec![hold("epic:1", HoldState::Awaiting, &["t2", "t3"])];
    let mut app = app_with(snap, ReviewTarget::Hold("epic:1".into()));
    let got = rows(&draw(&mut app, 80, 24));
    assert_eq!(got[0], top(" hold epic:1 · Add mul() · 0723 ", 80));
    // The hold's own tasks, all in stage 2: no stage count; the overlap is theirs.
    assert_eq!(
        got[1],
        framed(" 2 tasks · M+S · ~150 calls · critical t1 › t2 › t3", 80)
    );
    assert_eq!(
        got[2],
        framed(" ⚠ t2 and t3 both own crates/c/src/lib.rs", 80)
    );
    assert!(
        got[4].starts_with("│▌t2 report_product in c  cl opus "),
        "{got:#?}"
    );
}

/// A 40-character title at 80 columns: the title is cut, the deps stay whole. Narrower,
/// the route tag goes, then the stage column, and only then are the deps cut.
#[test]
fn deps_survive_at_80_columns() {
    let mut snap = overlapping_plan();
    snap.runs[0].tasks[0].title = "a forty character title for task one xyz".into();
    assert_eq!(snap.runs[0].tasks[0].title.len(), 40);
    let mut app = app_with(snap, ReviewTarget::Gate);
    let got = rows(&draw(&mut app, 80, 24));
    assert_eq!(
        got[4],
        framed(
            "▌t1 a forty character titl…  cx gpt-6-sol  S tdd   stage 1",
            80
        )
    );
    assert_eq!(
        got[6],
        framed(
            " t3 docs                     cl haiku      S none  stage 2  after t1 (implied)",
            80
        )
    );
    // 50 columns: no route tag, no stage column, the deps whole.
    let got = rows(&draw(&mut app, 50, 24));
    assert_eq!(got[5], framed(" t2 report_product …  M tdd   after t1", 50));
    assert_eq!(
        got[6],
        framed(" t3 docs              S none  after t1 (implied)", 50)
    );
    // 40 columns: the title at its eight, then the deps cut with `…`.
    let got = rows(&draw(&mut app, 40, 24));
    assert_eq!(got[5], framed(" t2 report_…  M tdd   after t1", 40));
    assert_eq!(got[6], framed(" t3 docs      S none  after t1 (impli…", 40));
}

#[test]
fn the_detail_is_labelled_rows() {
    let mut app = app_with(overlapping_plan(), ReviewTarget::Gate);
    press(&mut app, KeyCode::Char('j'));
    let got = rows(&draw(&mut app, 80, 24));
    assert_eq!(
        &got[8..12],
        &[
            framed(" t2  report_product in c", 80),
            framed(" brief      Build report_product in c.", 80),
            framed(" owns       crates/c/src/lib.rs", 80),
            framed(" done when  ◌ t2 works", 80),
        ]
    );
    assert_eq!(got[14], framed(" deps       after t1", 80));
    // In ASCII: the criterion's mark is its twin, the separators folded.
    app.settings.badges.ascii = true;
    let got = rows(&draw(&mut app, 80, 24));
    let ascii = |inner: &str| framed(inner, 80).replace('│', "|");
    assert_eq!(got[11], ascii(" done when  . t2 works"));
    assert_eq!(
        got[15],
        ascii(" route      claude - opus - standard - medium effort")
    );
}

#[test]
fn the_list_scrolls_with_its_selection() {
    let mut snap = overlapping_plan();
    let run = &mut snap.runs[0];
    run.tasks = (1..=12)
        .map(|n| plan_task(&format!("t{n}"), "step", Size::S, 1, 10))
        .collect();
    run.critical_path.clear();
    let mut app = app_with(snap, ReviewTarget::Gate);
    // Interior 21 rows: the header, a rule, ten list rows (half the interior), a rule.
    let got = rows(&draw(&mut app, 80, 24));
    assert_eq!(got[1], framed(" 12 tasks · 12S · ~120 calls", 80));
    assert!(got[3].starts_with("│▌t1 step"), "{got:#?}");
    assert!(got[11].starts_with("│ t9 step"), "{got:#?}");
    assert_eq!(got[12], framed(" ↓ 3 more", 80));
    assert_eq!(got[13], rule(80));
    for _ in 0..11 {
        press(&mut app, KeyCode::Char('j'));
    }
    let got = rows(&draw(&mut app, 80, 24));
    assert_eq!(got[3], framed(" ↑ 4 more", 80));
    assert!(got[4].starts_with("│ t5 step"), "{got:#?}");
    assert!(got[11].starts_with("│▌t12 step"), "{got:#?}");
    assert_eq!(got[12], framed("", 80));
    assert_eq!(got[13], rule(80));
    assert_eq!(got[14], framed(" t12  step", 80));
}

/// Review focus 5: agent text in the header, the overlap warnings, the columns and the
/// frame's title.
#[test]
fn plan_header_text_is_sanitised() {
    let bad = hostile_text();
    let mut snap = overlapping_plan();
    let run = &mut snap.runs[0];
    run.goal = format!("goal {bad}");
    run.critical_path = run.critical_path.iter().map(|id| hostile_id(id)).collect();
    for task in &mut run.tasks {
        task.title = format!("{bad}{}", task.title);
        task.owns = vec![format!("src/{bad}/lib.rs")];
        task.route.model = format!("m{bad}");
        task.id = hostile_id(&task.id);
        task.deps = task.deps.iter().map(|d| hostile_id(d)).collect();
        task.implicit_deps = task.implicit_deps.iter().map(|d| hostile_id(d)).collect();
    }
    for (width, height) in [(80, 24), (120, 40), (40, 12)] {
        for ascii in [false, true] {
            let mut app = app_with(snap.clone(), ReviewTarget::Gate);
            app.settings.badges.ascii = ascii;
            for step in 0..3 {
                let buffer = draw(&mut app, width, height);
                let what = format!("{width}x{height} ascii {ascii} step {step}");
                assert_clean(&buffer, &what);
                assert_spans_clean(&app, Rect::new(0, 0, width, height - 1), &what);
                assert!(
                    width < 80 || rows(&buffer).iter().any(|r| r.contains("both own src/")),
                    "{what}: the overlaps show"
                );
                press(&mut app, KeyCode::Char('j'));
            }
        }
    }
    // A hold's id in the frame's title.
    let id = format!("epic:{bad}");
    let run = &mut snap.runs[0];
    run.state = RunState::Running;
    run.tasks[1].hold = Some(id.clone());
    let t2 = run.tasks[1].id.clone();
    run.holds = vec![hold(&id, HoldState::Awaiting, &[t2.as_str()])];
    let mut app = app_with(snap, ReviewTarget::Hold(id));
    let buffer = draw(&mut app, 120, 40);
    assert!(
        rows(&buffer)[0].starts_with("╭ hold epic:"),
        "the hold review shows"
    );
    assert_clean(&buffer, "hold review");
}

#[test]
fn the_review_is_the_one_accented_frame() {
    for ascii in [false, true] {
        let mut app = app_with(overlapping_plan(), ReviewTarget::Gate);
        app.settings.badges.ascii = ascii;
        for (w, h) in [(80, 24), (120, 40)] {
            let buffer: Buffer = draw(&mut app, w, h);
            assert_eq!(
                audit::accented_frames(&buffer, app.palette()),
                1,
                "{w}x{h} ascii {ascii}:\n{}",
                rows(&buffer).join("\n")
            );
            if ascii {
                assert_eq!(audit::first_non_ascii(&buffer), None, "{w}x{h}");
            }
        }
    }
}

#[test]
fn no_panic_at_tiny_sizes() {
    for (width, height) in [(20, 5), (1, 1), (5, 40), (3, 3), (2, 8), (40, 4)] {
        let mut app = app_with(overlapping_plan(), ReviewTarget::Gate);
        draw(&mut app, width, height);
        press(&mut app, KeyCode::PageDown);
        press(&mut app, KeyCode::Char('j'));
        draw(&mut app, width, height);
        press(&mut app, KeyCode::PageUp);
    }
    // A Codex route with an empty model reads `default`.
    let mut snap = overlapping_plan();
    snap.runs[0].tasks[0].route = route(Runtime::Codex, "");
    let mut app = app_with(snap, ReviewTarget::Gate);
    assert!(rows(&draw(&mut app, 120, 40))[4].contains("cx default"));
}

/// Review finding 4: one task reads `1 task`, now in the header (moved from
/// `plan_review_tests.rs`, at its bound).
#[test]
fn a_one_task_review_says_one_task() {
    let mut snap = super::tests::hold_plan();
    snap.runs[0].holds = vec![hold("epic:mail", HoldState::Awaiting, &["t3"])];
    let mut app = super::tests::app_with(snap, ReviewTarget::Hold("epic:mail".into()));
    let got = rows(&draw(&mut app, 120, 40));
    assert_eq!(
        got[1],
        format!("│{:<118}│", " 1 task · 1 epic · S · ~100 calls")
    );
}

/// Fix round 1 (I1): the frame's title is the Block's, not a placed row, so it is
/// checked twice: the title text before ratatui sees it, and the exact top row. The
/// hostile characters are visible ones (a zero-width joiner, a bidi override, a
/// zero-width space) inside the drawn columns.
#[test]
fn the_frame_title_is_sanitised() {
    let id = "epic:x\u{200D}y\u{202E}z".to_owned();
    let mut snap = overlapping_plan();
    let run = &mut snap.runs[0];
    run.goal = "g\u{202E}\u{200B}oal".into();
    run.state = RunState::Running;
    run.tasks[1].hold = Some(id.clone());
    run.holds = vec![hold(&id, HoldState::Awaiting, &["t2"])];
    let mut app = app_with(snap, ReviewTarget::Hold(id));
    let review = app.plan_review.clone().expect("open");
    let (title, right) = super::frame_title(&app, &review, Some(&app.runs.runs[0]), 80);
    assert_eq!(
        crate::safe_text::tests::first_hostile(&title),
        None,
        "{title:?}"
    );
    assert_eq!(
        (title.as_str(), right),
        ("hold epic:xyz · goal · 0723", true)
    );
    let got = rows(&draw(&mut app, 80, 24));
    assert_eq!(got[0], top(" hold epic:xyz · goal · 0723 ", 80));
}

/// Fix round 1 (m3): in ASCII the run name is folded before it is cut, so a goal's
/// `…` (three columns once folded) cannot push the title into the right-hand one.
#[test]
fn an_ascii_title_folds_the_run_name_before_cutting_it() {
    let goal = format!("Add mul… — x · y › z {}", "w".repeat(40));
    let mut snap = overlapping_plan();
    snap.runs[0].goal = goal.clone();
    let mut app = app_with(snap, ReviewTarget::Gate);
    app.settings.badges.ascii = true;
    let got = rows(&draw(&mut app, 80, 24));
    // ` plan - ` (8) and ` awaiting approval ` (19) leave the name 49 columns: the
    // folded goal's first 39, `...`, ` - 0723`.
    let folded = theme::fold(&goal, true);
    let name = format!("{}... - 0723", &folded[..39]);
    let left = format!("+ plan - {name} ");
    let dashes = 80 - left.len() - " awaiting approval +".len();
    assert_eq!(
        got[0],
        format!("{left}{} awaiting approval +", "-".repeat(dashes))
    );
    assert_eq!(audit::first_non_ascii(&draw(&mut app, 80, 24)), None);
}

/// Fix round 1 (m2): under a modal (the `a` confirm, the help) the review's frame and
/// its `├─…─┤` rules are muted together; the modal's frame is the one accented.
#[test]
fn the_review_mutes_its_frame_and_rules_under_a_modal() {
    let muted = theme::fg(Role::Muted);
    let accent = theme::fg(Role::Accent);
    let check = |app: &mut App, what: &str| {
        let buffer = draw(app, 80, 24);
        assert_eq!(audit::accented_frames(&buffer, app.palette()), 1, "{what}");
        for (x, y) in [(0, 0), (0, 3), (79, 3), (0, 7), (79, 7)] {
            assert_eq!(buffer[(x, y)].fg, muted, "{what}: ({x}, {y})");
        }
    };
    let mut app = app_with(overlapping_plan(), ReviewTarget::Gate);
    let buffer = draw(&mut app, 80, 24);
    assert_eq!((buffer[(0, 3)].symbol(), buffer[(0, 3)].fg), ("├", accent));
    press(&mut app, KeyCode::Char('a'));
    assert!(matches!(app.modal, Some(crate::app::Modal::Confirm { .. })));
    check(&mut app, "the approve confirm");
    press(&mut app, KeyCode::Esc);
    assert_eq!(app.modal, None);
    let prefix = crossterm::event::KeyEvent::new(
        KeyCode::Char('b'),
        crossterm::event::KeyModifiers::CONTROL,
    );
    let _ = app.on_key(prefix);
    let _ = app.on_key(crossterm::event::KeyEvent::new(
        KeyCode::Char('?'),
        crossterm::event::KeyModifiers::NONE,
    ));
    assert!(app.modal.is_some(), "the help is open");
    check(&mut app, "the help");
}
