//! Milestone 9.3 task 10a: a later round's plan gate (decision 32, KG §2.4 step 5, §6):
//! the review names the round in its frame title and header, and lists and counts the
//! round's tasks only; the gate alert names the round. A one-round run's review is
//! unchanged (`plan_review_frame_tests.rs`, pinning).

use super::tests::{draw, rows};
use crate::app::{App, ReviewTarget, alerts};
use crate::safe_text::tests::first_hostile;
use crate::settings::UiSettings;
use crate::tree::plan_fixtures::{PLAN_RUN, overlapping_plan};
use crate::tree::run_fixtures::{PROJECT, pty};
use proto::{RoundInfo, RoundOrigin, RoundOutcome, RunsSnapshot, Status, TaskState};

/// `inner` between the frame's sides, padded to the interior (as the frame tests draw).
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

fn round(n: u32, head: &str, outcome: Option<RoundOutcome>) -> RoundInfo {
    RoundInfo {
        n,
        goal_head: head.into(),
        origin: RoundOrigin::User,
        outcome,
        summary_head: None,
    }
}

/// The overlapping plan as round 2's gate: `t1` merged in round 1, `t2` and `t3` the
/// round's, its request `also report the product`.
fn round_two() -> RunsSnapshot {
    let mut snap = overlapping_plan();
    let run = &mut snap.runs[0];
    run.round = 2;
    run.rounds = vec![
        round(1, "Add mul()", Some(RoundOutcome::Completed)),
        round(2, "also report the product", None),
    ];
    run.tasks[0].state = TaskState::Merged;
    for task in &mut run.tasks[1..] {
        task.round = 2;
    }
    snap
}

fn app_with(snap: RunsSnapshot, target: ReviewTarget) -> App {
    let mut app = App::new(
        vec![pty(1, "shell", PROJECT, Status::Idle)],
        "/tmp".into(),
        UiSettings::default(),
    );
    app.set_terminal_size(80, 24);
    app.on_daemon(proto::DaemonMsg::Run(proto::RunReply::Snapshot(snap)));
    let run = app.runs.runs[0].run_id.clone();
    assert!(app.open_plan_review(run, target).is_empty());
    app
}

#[test]
fn the_plan_review_names_the_round() {
    let mut app = app_with(round_two(), ReviewTarget::Gate);
    let got = rows(&draw(&mut app, 80, 24));
    assert_eq!(got[0], top(" plan · Add mul() · 0723 · round 2 ", 80));
    assert_eq!(got[1], framed(" round 2 · also report the product", 80));
    // The round's two tasks only: `t1`, round 1's, is neither counted nor listed, nor
    // on the critical path the header names.
    assert_eq!(
        got[2],
        framed(" 2 tasks · M+S · ~150 calls · critical t2 › t3", 80)
    );
    assert_eq!(
        got[3],
        framed(" ⚠ t2 and t3 both own crates/c/src/lib.rs", 80)
    );
    assert!(got[5].starts_with("│▌t2 report_product in c"), "{got:#?}");
    assert!(got[6].starts_with("│ t3 docs "), "{got:#?}");
    assert!(
        !got.iter().any(|row| row.contains("t1 add mul()")),
        "{got:#?}"
    );
    // `j` stops at the round's last task.
    for _ in 0..3 {
        super::tests::press(&mut app, crossterm::event::KeyCode::Char('j'));
    }
    let review = app.plan_review.as_ref().unwrap();
    assert_eq!(review.selected.as_deref(), Some("t3"));

    // ASCII folds the title's and the header's dots.
    let mut ascii = app_with(round_two(), ReviewTarget::Gate);
    ascii.settings.badges.ascii = true;
    let got = rows(&draw(&mut ascii, 80, 24));
    assert!(
        got[0].starts_with("+ plan - Add mul() - 0723 - round 2 "),
        "{got:#?}"
    );
    assert!(
        got[1].starts_with("| round 2 - also report the product "),
        "{got:#?}"
    );
}

/// A `pr` round's gate counts the pull requests of its own stages: `t2` and `t3` are
/// both in stage 2, so one, where the whole plan would have two.
#[test]
fn a_pr_rounds_gate_counts_its_own_pull_requests() {
    let mut snap = round_two();
    snap.runs[0].delivery = Some(crate::tree::pr_fixtures::delivery(false));
    let mut app = app_with(snap, ReviewTarget::Gate);
    let got = rows(&draw(&mut app, 80, 24));
    assert_eq!(
        got[3],
        framed(" delivered as 1 pull request to fake/app", 80),
        "{got:#?}"
    );
}

/// A narrow frame cuts the goal, never the round.
#[test]
fn a_narrow_review_keeps_the_round_in_its_title() {
    let mut app = app_with(round_two(), ReviewTarget::Gate);
    let got = rows(&draw(&mut app, 30, 24));
    assert!(
        got[0].starts_with("╭ plan · A… · 0723 · round 2 "),
        "{got:#?}"
    );
}

#[test]
fn the_gate_alert_names_the_round() {
    let mut snap = round_two();
    snap.runs[0].tasks[2].state = TaskState::Cancelled;
    let app = app_with(snap, ReviewTarget::Gate);
    let texts: Vec<String> = alerts(&app).into_iter().map(|a| a.text).collect();
    assert!(
        texts.contains(&"round 2 awaits approval · 1 task".to_string()),
        "{texts:?}"
    );
    assert!(
        !texts.iter().any(|t| t.starts_with("plan awaits")),
        "{texts:?}"
    );
    // One round: today's text, every task counted.
    let app = app_with(overlapping_plan(), ReviewTarget::Gate);
    let texts: Vec<String> = alerts(&app).into_iter().map(|a| a.text).collect();
    assert!(
        texts.contains(&"plan awaits approval · 3 tasks".to_string()),
        "{texts:?}"
    );
}

/// Decision 33: visible carriers in the round's request, the goal and the run id never
/// reach the review's title or header. Mutant: the round line's `one_line` removed
/// (`plan_summary::round_line`), red.
#[test]
fn review_text_is_sanitised() {
    let mut snap = round_two();
    let run = &mut snap.runs[0];
    run.run_id = "add-mul-07\u{200D}2\u{202E}3".into();
    run.goal = "Add x\u{200D}y\u{202E}z".into();
    run.rounds[1].goal_head = "also x\u{200D}y\u{202E}z".into();
    let mut app = app_with(snap, ReviewTarget::Gate);
    let got = rows(&draw(&mut app, 80, 24));
    assert_eq!(got[0], top(" plan · Add xyz · 0723 · round 2 ", 80));
    assert_eq!(got[1], framed(" round 2 · also xyz", 80));
    for row in &got {
        assert_eq!(first_hostile(row), None, "{row}");
    }
    let alert = alerts(&app).into_iter().find(|a| a.priority == 2).unwrap();
    assert_eq!(alert.text, "round 2 awaits approval · 2 tasks");
    assert!(PLAN_RUN.ends_with("0723"));
}
