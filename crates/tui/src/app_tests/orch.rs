//! M9.15: the run view's milestone 9 keys — `a` and `x` on an approval hold
//! (decision 28), `s` on a planning run's root (decision 13) — and the tagged replies
//! they and the forms are matched by (decision 2). Every request is the user's own,
//! sent from a key or a confirm, never on its own.

use super::gate::{select, tap, task_key};
use super::runs::{app_with_runs, deliver, open_run_view};
use super::*;
use crate::app::{Modal, PendingAction};
use crate::tree::NodeKey;
use crate::tree::orch_fixtures::{held_fixture, hold, orch_fixture};
use crate::tree::run_fixtures::RUN_ID;
use proto::{HoldState, RunReply, RunRequest, RunState};

fn held_app() -> App {
    let (snapshot, windows) = held_fixture();
    let mut app = app_with_runs(windows, snapshot);
    open_run_view(&mut app, RUN_ID);
    app
}

fn run(request: RunRequest) -> Vec<Effect> {
    vec![Effect::Send(ClientMsg::Run(request))]
}

fn approve(hold: &str) -> Vec<Effect> {
    run(RunRequest::ApproveHold {
        run_id: RUN_ID.into(),
        hold: hold.into(),
    })
}

fn reject(hold: &str) -> Vec<Effect> {
    run(RunRequest::RejectHold {
        run_id: RUN_ID.into(),
        hold: hold.into(),
    })
}

fn root() -> NodeKey {
    NodeKey::Run(RUN_ID.into())
}

fn confirm_message(app: &App) -> String {
    match &app.modal {
        Some(Modal::Confirm { message, .. }) => message.clone(),
        other => panic!("no confirm: {other:?}"),
    }
}

#[test]
fn hold_attention_line_and_keys_emit_the_requests() {
    let mut app = held_app();
    // The attention line (the inspector's projection of the root).
    let rows = app.nav_rows();
    let inspection = crate::inspector::inspect(&rows[0], &app);
    let attention = inspection
        .fields
        .iter()
        .find(|field| field.label == "attention")
        .map(|field| field.value.clone());
    assert_eq!(
        attention.as_deref(),
        Some("hold epic:ui: 1 task waits for approval")
    );

    // A held task: `a` approves its hold at once; `x` asks first.
    select(&mut app, task_key("t2"));
    assert_eq!(tap(&mut app, KeyCode::Char('a')), approve("epic:ui"));
    assert_eq!(app.modal, None);
    assert!(tap(&mut app, KeyCode::Char('x')).is_empty());
    assert_eq!(
        confirm_message(&app),
        "Reject hold epic:ui of run add-reset-3f9a? Its 1 task is cancelled."
    );
    assert_eq!(tap(&mut app, KeyCode::Char('y')), reject("epic:ui"));

    // The root while exactly one hold is awaiting.
    select(&mut app, root());
    assert_eq!(tap(&mut app, KeyCode::Char('a')), approve("epic:ui"));
    assert!(tap(&mut app, KeyCode::Char('x')).is_empty());
    assert!(matches!(
        &app.modal,
        Some(Modal::Confirm {
            action: PendingAction::RejectHold { .. },
            ..
        })
    ));
    assert!(tap(&mut app, KeyCode::Char('n')).is_empty());

    // A task that is not held: the gate is closed, as before.
    select(&mut app, task_key("t0"));
    assert!(tap(&mut app, KeyCode::Char('a')).is_empty());
    assert_eq!(
        app.toast_text(),
        Some("the plan gate is closed: run add-reset-3f9a is running")
    );

    // Two awaiting holds: the root names neither.
    let (mut snapshot, _) = held_fixture();
    snapshot.runs[0]
        .holds
        .push(hold("promotion", HoldState::Awaiting, &["t0"]));
    deliver(&mut app, snapshot);
    select(&mut app, root());
    assert!(tap(&mut app, KeyCode::Char('a')).is_empty());
    assert_eq!(
        app.toast_text(),
        Some("select a held task to approve its hold")
    );
    assert!(tap(&mut app, KeyCode::Char('x')).is_empty());
    assert_eq!(
        app.toast_text(),
        Some("select a held task to reject its hold")
    );
    assert_eq!(app.modal, None);
}

#[test]
fn a_held_task_whose_hold_is_not_awaiting_says_so() {
    let (mut snapshot, windows) = held_fixture();
    snapshot.runs[0].holds = vec![hold("epic:ui", HoldState::Drafting, &["t2"])];
    let mut app = app_with_runs(windows, snapshot);
    open_run_view(&mut app, RUN_ID);
    select(&mut app, task_key("t2"));
    assert!(tap(&mut app, KeyCode::Char('a')).is_empty());
    assert_eq!(
        app.toast_text(),
        Some("hold epic:ui is drafting; it can be approved once it awaits approval")
    );
}

#[test]
fn a_reject_confirm_closes_once_its_hold_is_decided() {
    let mut app = held_app();
    select(&mut app, task_key("t2"));
    tap(&mut app, KeyCode::Char('x'));
    let (mut snapshot, _) = held_fixture();
    snapshot.runs[0].holds = vec![hold("epic:ui", HoldState::Approved, &["t2"])];
    deliver(&mut app, snapshot);
    assert_eq!(app.modal, None);
    assert_eq!(app.toast_text(), Some("hold epic:ui is approved"));
}

/// The one tagged request in `effects`: its id and the request.
pub(super) fn tagged(effects: &[Effect]) -> (u64, RunRequest) {
    match effects {
        [Effect::Send(ClientMsg::RunTagged { id, request })] => (*id, request.clone()),
        other => panic!("not one tagged request: {other:?}"),
    }
}

#[test]
fn planning_root_submit_key_sends_the_user_submit() {
    let (snapshot, windows) = orch_fixture(RunState::Planning);
    let mut app = app_with_runs(windows, snapshot);
    open_run_view(&mut app, RUN_ID);
    assert_eq!(app.tree.selected, Some(root()));
    assert!(tap(&mut app, KeyCode::Char('s')).is_empty());
    assert_eq!(
        confirm_message(&app),
        "submit the plan of add-reset-3f9a yourself? (y/n)"
    );
    let (id, request) = tagged(&tap(&mut app, KeyCode::Char('y')));
    assert_eq!(
        request,
        RunRequest::Edit {
            run_id: RUN_ID.into(),
            edits: vec![],
            submit: true,
        }
    );
    app.on_daemon(DaemonMsg::Run(RunReply::Done {
        request: proto::run_wire::request::EDIT.into(),
        message: "the plan of run add-reset-3f9a was submitted: it awaits approval".into(),
        request_id: Some(id),
    }));
    assert_eq!(
        app.toast_text(),
        Some("the plan of run add-reset-3f9a was submitted: it awaits approval")
    );

    // Not on a task of the planning run.
    select(&mut app, task_key("t1"));
    assert!(tap(&mut app, KeyCode::Char('s')).is_empty());
    assert_eq!(app.modal, None);

    // Not on a running run's root.
    let (snapshot, windows) = orch_fixture(RunState::Running);
    let mut app = app_with_runs(windows, snapshot);
    open_run_view(&mut app, RUN_ID);
    assert!(tap(&mut app, KeyCode::Char('s')).is_empty());
    assert_eq!(app.modal, None);
}

/// M-6: a hold id is the orchestrator's text; the toast that quotes it carries no
/// control, separator or bidi character.
#[test]
fn a_toast_quoting_a_hold_id_is_sanitised() {
    let bad = format!("epic:{}", crate::safe_text::tests::hostile_text());
    let (mut snapshot, windows) = held_fixture();
    snapshot.runs[0].holds = vec![hold(&bad, HoldState::Drafting, &["t2"])];
    snapshot.runs[0].tasks[2].hold = Some(bad);
    let mut app = app_with_runs(windows, snapshot);
    open_run_view(&mut app, RUN_ID);
    select(&mut app, task_key("t2"));
    assert!(tap(&mut app, KeyCode::Char('x')).is_empty());
    let toast = app.toast_text().expect("a toast");
    assert!(toast.starts_with("hold epic:a b"), "{toast:?}");
    assert_eq!(
        crate::safe_text::tests::first_hostile(toast),
        None,
        "{toast:?}"
    );
}

/// M9.15 review: while the run awaits approval, `a` and `x` are the plan gate's,
/// even on a task that names a hold.
#[test]
fn the_gate_keys_win_while_the_run_awaits_approval() {
    let (mut snapshot, windows) = held_fixture();
    snapshot.runs[0].state = RunState::AwaitingApproval;
    let mut app = app_with_runs(windows, snapshot);
    open_run_view(&mut app, RUN_ID);
    select(&mut app, task_key("t2"));
    assert!(tap(&mut app, KeyCode::Char('a')).is_empty());
    assert!(matches!(
        &app.modal,
        Some(Modal::Confirm {
            action: PendingAction::ApproveRun(_),
            ..
        })
    ));
}
