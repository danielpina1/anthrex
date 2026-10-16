//! Milestone 9.17 task M9.17.6 (decisions 19 and 20, rulings C7 and C20): the plan
//! review shows the plan frame, what was decided for the user and what is out of
//! scope. Pure App tests: snapshots in, keys in, state and rows out; no clock.

use super::gate::tap;
use super::plan_review::{gate_snapshot, held_app, review};
use super::runs::{app_with_runs, open_run_view};
use super::*;
use crate::app::ReviewTarget;
use crate::app::plan_frame::frame_row;
use crate::tree::orch_fixtures::{held_fixture, hold};
use crate::tree::run_fixtures::{RUN_ID, planner};
use proto::{DecidedItem, HoldKind, HoldState, PlanFrameInfo, RunsSnapshot};
use ratatui::text::Line;
use unicode_width::UnicodeWidthStr;

pub(super) fn frame(decided: &[(&str, &str)], out: &[&str]) -> PlanFrameInfo {
    PlanFrameInfo {
        decided: (decided.iter())
            .map(|(q, a)| DecidedItem {
                question: q.to_string(),
                answer: a.to_string(),
            })
            .collect(),
        out_of_scope: out.iter().map(|s| s.to_string()).collect(),
        spec_non_goals: None,
    }
}

fn text(line: &Line<'_>) -> String {
    line.spans.iter().map(|s| s.content.as_ref()).collect()
}

/// The review's detail rows at 76 columns, as text.
fn detail(app: &App) -> Vec<String> {
    app.review_detail(76).iter().map(text).collect()
}

/// The gate snapshot with `frame` on the run, reviewed from the run view with `p`.
fn gate_with(frame: Option<PlanFrameInfo>) -> App {
    let (mut snap, windows) = gate_snapshot();
    snap.runs[0].plan_frame = frame;
    let mut app = app_with_runs(windows, snap);
    open_run_view(&mut app, RUN_ID);
    assert!(tap(&mut app, KeyCode::Char('p')).is_empty());
    app
}

fn two_decided() -> PlanFrameInfo {
    frame(&[("Which DB?", "SQLite"), ("Auth?", "JWT")], &["no UI"])
}

#[test]
fn review_opens_on_the_frame_and_f_toggles() {
    let mut app = gate_with(Some(two_decided()));
    assert!(review(&app).frame);
    assert_eq!(detail(&app)[0], "Decided for you · Out of scope");
    assert!(tap(&mut app, KeyCode::Char('f')).is_empty());
    assert!(!review(&app).frame);
    assert_eq!(detail(&app)[0], "t1  reset token model");
    tap(&mut app, KeyCode::Char('f'));
    assert!(review(&app).frame);
    assert_eq!(detail(&app)[0], "Decided for you · Out of scope");
    // `j` selects the next task and leaves the frame.
    tap(&mut app, KeyCode::Char('j'));
    let review = review(&app);
    assert_eq!(
        (review.frame, review.selected.as_deref()),
        (false, Some("t2"))
    );
    assert_eq!(review.scroll, 0);
    assert_eq!(detail(&app)[0], "t2  reset endpoint");
    // `k` on the frame leaves it too, back at the first task.
    tap(&mut app, KeyCode::Char('f'));
    tap(&mut app, KeyCode::Char('k'));
    let review = super::plan_review::review(&app);
    assert_eq!(
        (review.frame, review.selected.as_deref()),
        (false, Some("t1"))
    );
}

#[test]
fn header_row_counts_both_lists() {
    let app = gate_with(Some(two_decided()));
    let run = &app.runs.runs[0];
    let row = frame_row(run, &ReviewTarget::Gate, false).unwrap();
    assert_eq!(
        (row.text.as_str(), row.attention),
        (
            "frame  2 decided for you · 1 out of scope · f shows them",
            false
        )
    );
    // In ASCII the separators fold.
    let row = frame_row(run, &ReviewTarget::Gate, true).unwrap();
    assert_eq!(
        row.text,
        "frame  2 decided for you - 1 out of scope - f shows them"
    );
    // An epic's frame counts in its gate's row; a spec's Non-goals lead it.
    let mut run = run.clone();
    let mut e1 = planner("e1", "Epic one");
    e1.frame = Some(frame(&[], &["no docs"]));
    run.planners = vec![e1];
    run.plan_frame.as_mut().unwrap().spec_non_goals = Some("- no app".into());
    let row = frame_row(&run, &ReviewTarget::Gate, false).unwrap();
    assert_eq!(
        row.text,
        "frame  spec non-goals · 2 decided for you · 2 out of scope · f shows them"
    );
    // The layout gives the header a row for it.
    let layout = app.review_layout(ratatui::layout::Rect::new(0, 0, 80, 23));
    assert_eq!(
        layout.frame.as_ref().map(|r| r.text.as_str()),
        Some("frame  2 decided for you · 1 out of scope · f shows them")
    );
}

#[test]
fn empty_lists_say_none_stated_in_attention() {
    let app = gate_with(Some(PlanFrameInfo::default()));
    let row = frame_row(&app.runs.runs[0], &ReviewTarget::Gate, false).unwrap();
    assert_eq!(
        (row.text.as_str(), row.attention),
        ("frame  none stated", true)
    );
    assert!(review(&app).frame, "the review still opens on the frame");
    let rows = detail(&app);
    assert_eq!(
        rows[1..],
        ["decided       none stated", "out of scope  none stated"]
    );
}

#[test]
fn no_frame_no_row_and_f_does_nothing() {
    let mut app = gate_with(None);
    assert!(frame_row(&app.runs.runs[0], &ReviewTarget::Gate, false).is_none());
    assert!(!review(&app).frame);
    let before = app.plan_review.clone();
    assert!(tap(&mut app, KeyCode::Char('f')).is_empty());
    assert_eq!(app.plan_review, before);
    assert_eq!(detail(&app)[0], "t1  reset token model");
}

/// Ruling C20: on a design gate's Plan doc tab the detail is hidden, so `f` does
/// nothing there.
#[test]
fn f_does_nothing_on_the_plan_doc_tab() {
    use crate::app::doc_gate::plan_doc::tests::{PLAN, on_tab, plan_run};
    let mut run = plan_run(1);
    run.plan_frame = Some(PlanFrameInfo {
        spec_non_goals: Some("- no app".into()),
        ..PlanFrameInfo::default()
    });
    let mut app = on_tab(run, PLAN, 80, 24);
    assert!(app.plan_doc().is_some());
    let before = app.plan_review.clone();
    assert!(before.as_ref().unwrap().frame);
    tap(&mut app, KeyCode::Char('f'));
    assert_eq!(app.plan_review, before);
}

#[test]
fn frame_detail_rows() {
    let mut app = gate_with(Some(two_decided()));
    let mut e1 = planner("e1", "Epic one");
    e1.frame = Some(frame(&[("Retries?", "3")], &["no docs"]));
    app.runs.runs[0].planners = vec![e1, planner("e2", "Epic two")];
    assert_eq!(
        detail(&app),
        [
            "Decided for you · Out of scope",
            "decided          Which DB? → SQLite",
            "                 Auth? → JWT",
            "out of scope     no UI",
            "e1 decided       Retries? → 3",
            "e1 out of scope  no docs",
        ]
    );
    // ASCII glyphs.
    let mut p = app.palette();
    p.ascii = true;
    let run = &app.runs.runs[0];
    let rows: Vec<String> = (crate::app::plan_frame::frame_lines(run, &ReviewTarget::Gate, 77, p))
        .iter()
        .map(text)
        .collect();
    assert_eq!(rows[0], "Decided for you - Out of scope");
    assert_eq!(rows[1], "decided          Which DB? -> SQLite");
}

/// Decision 19: a hold review shows the frames of the hold's epics only.
#[test]
fn hold_review_shows_only_its_epics() {
    let (mut snap, windows) = held_fixture();
    let run = &mut snap.runs[0];
    run.tasks[2].epic = Some("ui".into());
    let mut ui = planner("ui", "UI");
    ui.frame = Some(frame(&[], &["no themes"]));
    let mut api = planner("api", "API");
    api.frame = Some(frame(&[], &["no v2"]));
    run.planners = vec![ui, api];
    let mut app = held_app((snap, windows));
    assert!(tap(&mut app, KeyCode::Char('p')).is_empty());
    assert_eq!(review(&app).target, ReviewTarget::Hold("epic:ui".into()));
    assert!(review(&app).frame);
    let rows = detail(&app);
    assert_eq!(
        rows[1..],
        ["ui decided       none stated", "ui out of scope  no themes"]
    );
    let row = frame_row(&app.runs.runs[0], &review(&app).target, false).unwrap();
    assert_eq!(
        row.text,
        "frame  0 decided for you · 1 out of scope · f shows them"
    );
}

/// Ruling C7: the `promotion` hold is a promoted run's plan approval, so its review
/// shows the run's frame.
#[test]
fn promotion_hold_review_shows_the_run_frame() {
    let (mut snap, windows): (RunsSnapshot, _) = held_fixture();
    let run = &mut snap.runs[0];
    run.tasks[2].hold = Some("promotion".into());
    let mut promotion = hold("promotion", HoldState::Awaiting, &["t2"]);
    promotion.kind = HoldKind::Promotion;
    run.holds = vec![promotion];
    run.plan_frame = Some(frame(&[("Which DB?", "SQLite")], &[]));
    let mut app = held_app((snap, windows));
    assert!(tap(&mut app, KeyCode::Char('p')).is_empty());
    assert_eq!(review(&app).target, ReviewTarget::Hold("promotion".into()));
    let rows = detail(&app);
    assert_eq!(
        rows[1..],
        [
            "decided       Which DB? → SQLite",
            "out of scope  none stated"
        ]
    );
}

#[test]
fn frame_text_is_sanitised() {
    let esc = '\u{1b}';
    let wide = "漢".repeat(60);
    let mut app = gate_with(Some(frame(
        &[("Which DB?", &format!("SQ{esc}[2JLite\u{202E}"))],
        &[&wide],
    )));
    let mut e1 = planner(&format!("e{esc}1"), "Epic");
    e1.frame = Some(frame(&[], &["x"]));
    app.runs.runs[0].planners = vec![e1];
    for width in [76u16, 30, 24] {
        let lines = app.review_detail(width);
        for line in &lines {
            let row = text(line);
            assert!(
                !row.chars().any(|c| c.is_control() || c == '\u{202E}'),
                "{row:?}"
            );
            // The bar's column is left out of the detail's width.
            assert!(row.width() < usize::from(width), "{row:?} at {width}");
        }
        let all: String = lines.iter().map(text).collect();
        assert!(all.contains("漢漢"), "the wide item is wrapped, not lost");
    }
    let row = frame_row(&app.runs.runs[0], &ReviewTarget::Gate, false).unwrap();
    assert!(!row.text.chars().any(|c| c.is_control()));
}
