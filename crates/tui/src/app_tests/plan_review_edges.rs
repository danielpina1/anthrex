//! M9.0.5.6/7 review fixes: what reaches the screen and the daemon under the plan
//! review — a paste, the scroll after the content shrinks, and a hold's `a`/`x` when
//! a task's own hold disagrees with the hold under review.

use super::gate::{send, tap};
use super::plan_review::{confirm, gate_app, gate_snapshot, held_app, review, two_holds};
use super::runs::{app_with_runs, deliver};
use super::*;
use crate::app::plan_review::detail_lines;
use crate::app::{Modal, PendingAction, ReviewTarget};
use crate::tree::run_fixtures::RUN_ID;
use proto::RunRequest;
use ratatui::layout::Rect;

#[test]
fn a_paste_under_the_review_over_the_terminal_sends_nothing() {
    let (snap, windows) = gate_snapshot();
    let mut app = app_with_runs(windows, snap);
    assert!(
        app.open_plan_review(RUN_ID.into(), ReviewTarget::Gate)
            .is_empty()
    );
    let before = app.plan_review.clone();
    assert_eq!(app.on_paste("rm -rf x\n".into()), vec![]);
    assert_eq!(app.plan_review, before);
    assert_eq!(app.modal, None);
    assert_eq!(app.tree_input, None);
    // Once the review is closed the terminal takes pastes again.
    tap(&mut app, KeyCode::Esc);
    assert_eq!(
        app.on_paste("ls\n".into()),
        vec![Effect::Send(ClientMsg::Input {
            window_id: 1,
            bytes: b"ls\r".to_vec()
        })]
    );
}

#[test]
fn a_paste_under_the_review_over_the_conversation_changes_nothing() {
    let (snap, windows) = gate_snapshot();
    let mut app = app_with_runs(windows, snap);
    let _ = app.open_conversation(1);
    assert!(app.conversation.is_open());
    tap(&mut app, KeyCode::Char('/'));
    let typing = |app: &App| {
        app.conversation
            .search()
            .map(|s| (s.query.clone(), s.typing))
    };
    assert_eq!(typing(&app), Some((String::new(), true)));
    app.open_plan_review(RUN_ID.into(), ReviewTarget::Gate);
    assert_eq!(app.on_paste("secret".into()), vec![]);
    assert_eq!(
        typing(&app),
        Some((String::new(), true)),
        "the hidden search"
    );
    assert!(app.plan_review.is_some());
}

#[test]
fn a_paste_into_a_modal_over_the_review_still_works() {
    let mut app = gate_app();
    tap(&mut app, KeyCode::Char('p'));
    assert!(tap(&mut app, KeyCode::Char('e')).is_empty());
    // Milestone 9.8: the brief is the form's text that takes a paste (the model is the
    // picker's).
    let brief = |app: &App| match &app.modal {
        Some(Modal::EditTask(form)) => (form.focus, form.brief.text().to_owned()),
        other => panic!("no edit form: {other:?}"),
    };
    // The form opens on its first field; tab to the brief.
    for _ in 0..8 {
        if brief(&app).0 == crate::run_edit::EditField::Brief {
            break;
        }
        tap(&mut app, KeyCode::Tab);
    }
    let before = brief(&app).1;
    assert_eq!(app.on_paste("gpt-5".into()), vec![]);
    assert_eq!(brief(&app).1.len(), before.len() + 5);
    assert!(brief(&app).1.contains("gpt-5"));
    assert!(app.plan_review.is_some());
}

/// The detail's row count for the gate's `t1` at `body` (milestone 9.0.7 decision 26's
/// stacked geometry).
fn t1_rows(app: &App, body: Rect) -> u16 {
    let run = &app.runs.runs[0];
    let width = app.review_layout(body).detail.width;
    detail_lines(run, &run.tasks[0], width, app.palette()).len() as u16
}

/// Review finding 3: at 80×24, t1's 40-line brief scrolled to its end, then a snapshot
/// cuts it to 10 lines. The scroll follows the shorter content, so one `PgUp` moves.
#[test]
fn the_scroll_follows_content_that_shrank() {
    let mut app = gate_app();
    let body = Rect::new(0, 0, 80, 23);
    app.set_body_area(body);
    tap(&mut app, KeyCode::Char('p'));
    for _ in 0..10 {
        tap(&mut app, KeyCode::PageDown);
    }
    let height = app.review_layout(body).detail.height;
    assert_eq!(review(&app).scroll, t1_rows(&app, body) - height);
    let (mut snap, _) = gate_snapshot();
    snap.runs[0].tasks[0].brief = (1..=10)
        .map(|n| format!("brief line {n}"))
        .collect::<Vec<_>>()
        .join("\n");
    deliver(&mut app, snap);
    let max = t1_rows(&app, body) - height;
    assert!(max > 0 && max < 21, "{max}");
    assert_eq!(review(&app).scroll, max, "clamped by the snapshot");
    assert!(tap(&mut app, KeyCode::PageUp).is_empty());
    assert_eq!(review(&app).scroll, 0, "one page up from the new end");
}

/// A taller body with no snapshot in between: the state still holds the old end.
/// `PgUp` steps from the new end, not from the stale one.
#[test]
fn page_up_after_the_body_grew_steps_from_the_new_end() {
    let mut app = gate_app();
    app.set_body_area(Rect::new(0, 0, 80, 23));
    tap(&mut app, KeyCode::Char('p'));
    for _ in 0..10 {
        tap(&mut app, KeyCode::PageDown);
    }
    let tall = Rect::new(0, 0, 80, 39);
    app.set_body_area(tall);
    let height = app.review_layout(tall).detail.height;
    let max = t1_rows(&app, tall) - height;
    assert!(review(&app).scroll > max, "the stale end");
    assert!(tap(&mut app, KeyCode::PageUp).is_empty());
    assert_eq!(review(&app).scroll, max.saturating_sub(height - 1));
    assert!(tap(&mut app, KeyCode::PageDown).is_empty());
    assert_eq!(review(&app).scroll, max);
}

/// Review finding 6: hold `epic:api` lists `t4`, but `t4.hold` names `epic:ui`. The
/// review's `a` and `x` act on the hold under review.
#[test]
fn a_hold_review_acts_on_its_own_hold() {
    let (mut snap, windows) = two_holds();
    let run = &mut snap.runs[0];
    let t4 = run.tasks.iter_mut().find(|t| t.id == "t4").unwrap();
    t4.hold = Some("epic:ui".into());
    let mut app = held_app((snap, windows));
    app.open_plan_review(RUN_ID.into(), ReviewTarget::Hold("epic:api".into()));
    assert_eq!(review(&app).selected.as_deref(), Some("t4"));
    assert_eq!(
        tap(&mut app, KeyCode::Char('a')),
        send(RunRequest::ApproveHold {
            run_id: RUN_ID.into(),
            hold: "epic:api".into(),
        })
    );
    assert!(tap(&mut app, KeyCode::Char('x')).is_empty());
    assert_eq!(
        confirm(&app).1,
        &PendingAction::RejectHold {
            run_id: RUN_ID.into(),
            hold: "epic:api".into(),
        }
    );
}

/// Review of fd9dd25: one criterion, owns entry or note is one row, whatever line
/// breaks it carries, so it cannot forge another `☐` criterion.
#[test]
fn a_list_entry_with_a_line_break_stays_one_row() {
    let (mut snapshot, _) = gate_snapshot();
    let task = &mut snapshot.runs[0].tasks[0];
    task.acceptance = vec!["real\n☐ FORGED".into(), "x\r☐ FORGED2".into()];
    task.owns = vec!["src/a.rs\nsrc/forged.rs".into()];
    task.notes = vec!["one\r\ntwo".into()];
    let run = &snapshot.runs[0];
    let palette = crate::theme::Palette::PLAIN;
    let text: Vec<String> = detail_lines(run, &run.tasks[0], 200, palette)
        .into_iter()
        .map(|line| {
            line.spans
                .iter()
                .map(|s| s.content.as_ref())
                .collect::<String>()
        })
        .collect();
    // Milestone 9.0.7 decision 25: each criterion is a `◌` row under `done when`.
    let criteria: Vec<&str> = text
        .iter()
        .filter_map(|row| row.find('◌').map(|at| row[at..].trim_end()))
        .collect();
    assert_eq!(criteria, ["◌ real ☐ FORGED", "◌ x ☐ FORGED2"], "{text:?}");
    assert!(
        text.contains(&"owns       src/a.rs src/forged.rs".to_owned()),
        "{text:?}"
    );
    assert!(text.contains(&"notes      one two".to_owned()), "{text:?}");
}
