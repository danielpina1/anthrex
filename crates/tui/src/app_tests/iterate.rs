//! Milestone 9.3 task 10a: the iterate dialog reached by the keys a user presses
//! (decision 32): the action menu's `iterate`, listed only when the daemon lists it,
//! Ctrl-S's tagged `Iterate`, its `Done` and `Refused`, Esc's confirm page, a paste,
//! and a request the connection never sent.

use super::actions::{action, flow, kinds, sent_tagged_id, tap};
use super::runs::{app_with_runs, open_run_view};
use super::*;
use crate::run_iterate::{IterateForm, NOT_SENT};
use crate::tree::run_fixtures::{RUN_ID, three_task_fixture};
use proto::run_wire::request::ITERATE;
use proto::{ActionKind, RunReply, RunRequest, RunState};

const STARTED: &str = "run 3f9a round 2 started; its orchestrator plans it";

/// The three-task run, complete, listing accept and discard, and `iterate` when
/// `iterate`; its run view open, the last frame 80x24.
fn complete_app(iterate: Option<Option<&str>>) -> App {
    let (mut snap, windows) = three_task_fixture();
    let run = &mut snap.runs[0];
    run.state = RunState::Complete;
    run.actions = vec![
        action(ActionKind::Accept, "accept", None),
        action(ActionKind::Discard, "discard", None),
    ];
    if let Some(refused) = iterate {
        run.actions
            .push(action(ActionKind::Iterate, "iterate", refused));
    }
    let mut app = app_with_runs(windows, snap);
    open_run_view(&mut app, RUN_ID);
    app.set_body_area(ratatui::layout::Rect::new(0, 0, 80, 23));
    app
}

fn dialog(app: &App) -> &IterateForm {
    match &app.modal {
        Some(Modal::Iterate(form)) => form,
        other => panic!("no iterate dialog: {other:?}"),
    }
}

/// `.` on the run, `j` to `iterate`, Enter.
fn open(app: &mut App) {
    tap(app, KeyCode::Char('.'));
    let at = kinds(app)
        .iter()
        .position(|k| *k == ActionKind::Iterate)
        .expect("iterate is listed");
    for _ in 0..at {
        tap(app, KeyCode::Char('j'));
    }
    assert!(tap(app, KeyCode::Enter).is_empty());
}

fn type_text(app: &mut App, text: &str) {
    for c in text.chars() {
        let code = if c == '\n' {
            KeyCode::Enter
        } else {
            KeyCode::Char(c)
        };
        assert!(tap(app, code).is_empty(), "{c:?}");
    }
}

fn ctrl(app: &mut App, c: char) -> Vec<Effect> {
    press(app, KeyCode::Char(c), KeyModifiers::CONTROL)
}

fn is_iterate(request: &RunRequest, goal: &str) -> bool {
    *request
        == RunRequest::Iterate {
            run: RUN_ID.into(),
            goal: goal.into(),
        }
}

#[test]
fn iterate_opens_from_the_menu_only_when_listed() {
    let mut app = complete_app(None);
    tap(&mut app, KeyCode::Char('.'));
    assert_eq!(
        kinds(&app),
        vec![ActionKind::Accept, ActionKind::Discard, ActionKind::Stats]
    );

    let mut app = complete_app(Some(None));
    tap(&mut app, KeyCode::Char('.'));
    assert_eq!(
        kinds(&app),
        vec![
            ActionKind::Accept,
            ActionKind::Discard,
            ActionKind::Iterate,
            ActionKind::Stats
        ]
    );
    assert_eq!(flow(&app).items[2].label, "iterate");
    tap(&mut app, KeyCode::Esc);
    open(&mut app);
    assert_eq!(dialog(&app), &IterateForm::new(RUN_ID.into(), 2));

    // The round is the snapshot's next one.
    let mut app = complete_app(Some(None));
    let mut snap = app.runs.clone();
    snap.runs[0].round = 4;
    super::runs::deliver(&mut app, snap);
    open(&mut app);
    assert_eq!(dialog(&app).round, 5);

    // A listed refusal is toasted, and no dialog opens.
    let mut app = complete_app(Some(Some(
        "run 3f9a is running; iterate it when it completes",
    )));
    open(&mut app);
    assert!(matches!(app.modal, Some(Modal::Action(_))));
    assert_eq!(
        app.toast_text(),
        Some("run 3f9a is running; iterate it when it completes")
    );
}

#[test]
fn ctrl_s_sends_iterate_and_done_closes_with_a_toast() {
    let mut app = complete_app(Some(None));
    open(&mut app);
    type_text(&mut app, "also add b\nand c");
    let effects = ctrl(&mut app, 's');
    let id = sent_tagged_id(&effects, |r| is_iterate(r, "also add b\nand c"));
    assert!(dialog(&app).submitting);
    assert_eq!(dialog(&app).request_id, Some(id));
    // A second Ctrl-S while it waits sends nothing.
    assert!(ctrl(&mut app, 's').is_empty());
    // A reply to another request leaves the dialog.
    app.on_run_reply(RunReply::Done {
        request: ITERATE.into(),
        message: "other".into(),
        request_id: Some(id + 100),
    });
    assert!(matches!(app.modal, Some(Modal::Iterate(_))));
    app.on_run_reply(RunReply::Done {
        request: ITERATE.into(),
        message: STARTED.into(),
        request_id: Some(id),
    });
    assert!(app.modal.is_none(), "Done closes the dialog");
    assert_eq!(app.toast_text(), Some(STARTED));
}

#[test]
fn a_refusal_shows_in_the_error_row() {
    let mut app = complete_app(Some(None));
    open(&mut app);
    type_text(&mut app, "more");
    let id = sent_tagged_id(&ctrl(&mut app, 's'), |r| is_iterate(r, "more"));
    let refusal = "run 3f9a has had 20 rounds; accept or discard it and start a new goal";
    app.on_run_reply(RunReply::Refused {
        request: ITERATE.into(),
        message: refusal.into(),
        request_id: Some(id),
    });
    let form = dialog(&app);
    assert_eq!(form.error.as_deref(), Some(refusal));
    assert!(!form.submitting && form.request_id.is_none());
    assert_eq!(form.text.text(), "more", "the text stays");
    assert_ne!(app.toast_text(), Some(refusal), "the row, not a toast");
    let screen = super::goal_editor::screen(&app);
    assert!(screen.contains(refusal), "{screen}");
    // Ctrl-S again sends again, and clears the row.
    let effects = ctrl(&mut app, 's');
    sent_tagged_id(&effects, |r| is_iterate(r, "more"));
    assert_eq!(dialog(&app).error, None);
}

#[test]
fn esc_confirm_applies_to_the_iterate_dialog_too() {
    let mut app = complete_app(Some(None));
    open(&mut app);
    // Esc on an empty text closes at once.
    tap(&mut app, KeyCode::Esc);
    assert!(app.modal.is_none());
    open(&mut app);
    type_text(&mut app, "more");
    tap(&mut app, KeyCode::Esc);
    assert!(dialog(&app).discarding);
    let screen = super::goal_editor::screen(&app);
    assert!(screen.contains("discard this goal text?"), "{screen}");
    assert!(
        screen.contains("y discard · any other key back"),
        "{screen}"
    );
    // Any other key goes back to the text, kept.
    tap(&mut app, KeyCode::Char('n'));
    assert!(!dialog(&app).discarding);
    assert_eq!(dialog(&app).text.text(), "more");
    // `y` closes, and nothing is kept: the next dialog is empty.
    tap(&mut app, KeyCode::Esc);
    tap(&mut app, KeyCode::Char('y'));
    assert!(app.modal.is_none());
    open(&mut app);
    assert!(
        dialog(&app).text.is_empty(),
        "the iterate dialog keeps no draft"
    );
    // While it waits, Esc closes at once, and the late reply is a toast.
    type_text(&mut app, "more");
    let id = sent_tagged_id(&ctrl(&mut app, 's'), |r| is_iterate(r, "more"));
    tap(&mut app, KeyCode::Esc);
    assert!(app.modal.is_none());
    app.on_run_reply(RunReply::Done {
        request: ITERATE.into(),
        message: STARTED.into(),
        request_id: Some(id),
    });
    assert_eq!(app.toast_text(), Some(STARTED));
}

/// A bracketed paste goes to the dialog's editor, its line breaks kept and its carriers
/// dropped, never to the pane underneath.
#[test]
fn a_paste_goes_to_the_iterate_editor() {
    let mut app = complete_app(Some(None));
    open(&mut app);
    assert!(app.on_paste("a\r\nb\u{200D}c\u{1b}".into()).is_empty());
    assert_eq!(dialog(&app).text.text(), "a\nbc");
    let big = "x".repeat(proto::GOAL_MAX_CHARS + 1);
    app.on_paste(big);
    assert_eq!(
        dialog(&app).text.text().chars().count(),
        proto::GOAL_MAX_CHARS
    );
    let screen = super::goal_editor::screen(&app);
    assert!(
        screen.contains("goal is at its 16,384-character limit"),
        "{screen}"
    );
}

/// The connection refused the request, or the link went while it waited: the dialog
/// stops waiting and says so; Ctrl-S sends again.
#[test]
fn a_request_that_was_not_sent_frees_the_dialog() {
    let mut app = complete_app(Some(None));
    open(&mut app);
    type_text(&mut app, "more");
    let effects = ctrl(&mut app, 's');
    let id = sent_tagged_id(&effects, |r| is_iterate(r, "more"));
    let Effect::Send(msg) = effects[0].clone() else {
        panic!("{effects:?}");
    };
    app.on_send_failed(&msg);
    assert_eq!(dialog(&app).error.as_deref(), Some(NOT_SENT));
    assert!(!dialog(&app).submitting);
    let id2 = sent_tagged_id(&ctrl(&mut app, 's'), |r| is_iterate(r, "more"));
    assert_ne!(id, id2);
    app.on_link_lost("gone");
    assert_eq!(dialog(&app).error.as_deref(), Some(NOT_SENT));
    assert!(!dialog(&app).submitting);
}

/// Task 4b's carried item, the client's half: a later round's reject cancels that
/// round's tasks and keeps the run, so its confirm page says so; one round's page is
/// unchanged.
#[test]
fn a_later_rounds_reject_page_names_the_round() {
    use crate::app::actions::confirm_details;
    use proto::{RoundInfo, RoundOrigin, RoundOutcome, TaskState};
    let (mut snap, _) = crate::tree::run_fixtures::gate_fixture();
    let run = &mut snap.runs[0];
    let one = confirm_details(run, &ActionKind::Reject, false);
    assert_eq!(
        one[0].1,
        format!("the run's remaining worktrees and its anthrex/{RUN_ID}/* branches")
    );
    let round = |n, outcome| RoundInfo {
        n,
        goal_head: "g".into(),
        origin: RoundOrigin::User,
        outcome,
        summary_head: None,
        ended: outcome.is_some(),
    };
    run.round = 2;
    run.rounds = vec![round(1, Some(RoundOutcome::Completed)), round(2, None)];
    run.tasks[0].state = TaskState::Merged;
    run.tasks[1].round = 2;
    let row = |l: &str, v: &str| (l.to_string(), v.to_string());
    assert_eq!(
        confirm_details(run, &ActionKind::Reject, false),
        vec![
            row("cancels", "round 2's 1 task"),
            row("keeps", "the earlier rounds · main unchanged"),
        ]
    );
    assert_eq!(
        confirm_details(run, &ActionKind::Reject, true)[1],
        row("keeps", "the earlier rounds - main unchanged")
    );
}
