//! M9.0.5.6/7 review fixes: what reaches the screen and the daemon under the plan
//! review — a paste, the scroll after the content shrinks, and a hold's `a`/`x` when
//! a task's own hold disagrees with the hold under review.

use super::gate::{send, tap};
use super::plan_review::{confirm, gate_app, gate_snapshot, held_app, review, two_holds};
use super::runs::{app_with_runs, deliver};
use super::*;
use crate::app::plan_review::{detail_lines, panes};
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
    let model = |app: &App| match &app.modal {
        Some(Modal::EditTask(form)) => (form.focus, form.model.text().to_owned()),
        other => panic!("no edit form: {other:?}"),
    };
    // The form opens on its first field; tab to the model's.
    for _ in 0..8 {
        if model(&app).0 == crate::run_edit::EditField::Model {
            break;
        }
        tap(&mut app, KeyCode::Tab);
    }
    let before = model(&app).1;
    assert_eq!(app.on_paste("gpt-5".into()), vec![]);
    assert_eq!(model(&app).1, format!("{before}gpt-5"));
    assert!(app.plan_review.is_some());
}

/// The right pane's row count for the gate's `t1` at `body`.
fn t1_rows(app: &App, body: Rect) -> u16 {
    let run = &app.runs.runs[0];
    detail_lines(run, &run.tasks[0], panes(body).right.width).len() as u16
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
    let height = panes(body).right.height;
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
    let height = panes(tall).right.height;
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
