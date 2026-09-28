//! M8c.9: the plan gate — approve, reject, remove and the task edit form (decisions
//! 32–34). The client sends only the user's own requests, each after a confirm.

use super::*;
use crate::app::{Modal, PendingAction, RunView};
use crate::run_edit::tests::edit_fixture_task;
use crate::run_edit::{EditField, TaskEditForm};
use crate::tree::NodeKey;
use crate::tree::run_fixtures::{RUN_ID, gate_fixture, snapshot, task};
use proto::{
    Effort, PlanEdit, RouteSpec, RunPath, RunReply, RunRequest, RunState, Size, Strength,
    TaskState, TestMode,
};

use super::runs::{app_with_runs, deliver, open_run_view};

#[path = "gate/filter.rs"]
mod filter;
#[path = "gate/replies.rs"]
mod replies;
#[path = "gate/stale.rs"]
mod stale;
#[path = "gate/unsent.rs"]
mod unsent;

/// The gate fixture with the brief's edit-form `t1`, the run view open on it.
pub(super) fn gate() -> App {
    let (mut snap, windows) = gate_fixture();
    snap.runs[0].tasks[0] = edit_fixture_task();
    let mut app = app_with_runs(windows, snap);
    open_run_view(&mut app, RUN_ID);
    app
}

pub(super) fn tap(app: &mut App, code: KeyCode) -> Vec<Effect> {
    press(app, code, KeyModifiers::NONE)
}

pub(super) fn task_key(id: &str) -> NodeKey {
    NodeKey::Task {
        run: RUN_ID.into(),
        id: id.into(),
    }
}

pub(super) fn select(app: &mut App, key: NodeKey) {
    let rows = crate::app::nav_rows_of(
        &app.windows,
        &app.runs.runs,
        &app.tree,
        app.run_view.as_ref(),
    );
    app.tree.select(&rows, key.clone());
    assert_eq!(app.tree.selected, Some(key));
}

pub(super) fn send(request: RunRequest) -> Vec<Effect> {
    vec![Effect::Send(ClientMsg::Run(request))]
}

pub(super) fn edit(edits: Vec<PlanEdit>) -> Vec<Effect> {
    send(RunRequest::Edit {
        run_id: RUN_ID.into(),
        edits,
    })
}

pub(super) fn amend(route: Option<RouteSpec>, size: Option<Size>) -> PlanEdit {
    PlanEdit::AmendTask {
        task_id: "t1".into(),
        route,
        size,
        brief: None,
        acceptance: None,
        test_mode: None,
        test_mode_reason: None,
        priority: None,
        deps: None,
    }
}

pub(super) fn form(app: &App) -> &TaskEditForm {
    match &app.modal {
        Some(Modal::EditTask(form)) => form,
        other => panic!("no edit form: {other:?}"),
    }
}

pub(super) fn open_form(app: &mut App, id: &str) {
    select(app, task_key(id));
    assert!(tap(app, KeyCode::Char('e')).is_empty());
    assert_eq!(form(app).task_id, id);
}

/// Tabs to `field` in the open form.
pub(super) fn focus(app: &mut App, field: EditField) {
    for _ in 0..16 {
        if form(app).focus == field {
            return;
        }
        assert!(tap(app, KeyCode::Tab).is_empty());
    }
    panic!("{field:?} not reachable");
}

pub(super) fn typed(app: &mut App, text: &str) {
    for c in text.chars() {
        assert!(tap(app, KeyCode::Char(c)).is_empty());
    }
}

fn confirm(app: &App) -> (&str, &PendingAction) {
    match &app.modal {
        Some(Modal::Confirm { message, action }) => (message, action),
        other => panic!("no confirm: {other:?}"),
    }
}

pub(super) fn reply(app: &mut App, reply: RunReply) -> Vec<Effect> {
    app.on_daemon(DaemonMsg::Run(reply))
}

pub(super) fn done(request: &str, message: &str) -> RunReply {
    RunReply::Done {
        request: request.into(),
        message: message.into(),
        request_id: None,
    }
}

pub(super) fn refused(request: &str, message: &str) -> RunReply {
    RunReply::Refused {
        request: request.into(),
        message: message.into(),
        request_id: None,
    }
}

/// Changes the size to S and submits, leaving the form submitting.
pub(super) fn submit_a_size_change(app: &mut App) -> Vec<Effect> {
    open_form(app, "t1");
    focus(app, EditField::Size);
    assert!(tap(app, KeyCode::Right).is_empty());
    tap(app, KeyCode::Enter)
}

#[test]
fn a_asks_then_approves() {
    let mut app = gate();
    assert!(tap(&mut app, KeyCode::Char('a')).is_empty());
    assert_eq!(
        confirm(&app),
        (
            "Approve run add-reset-3f9a? 2 tasks start.",
            &PendingAction::ApproveRun(RUN_ID.into())
        )
    );
    assert_eq!(
        tap(&mut app, KeyCode::Char('y')),
        send(RunRequest::Approve {
            run_id: RUN_ID.into()
        })
    );
    assert_eq!(app.modal, None);

    assert!(tap(&mut app, KeyCode::Char('a')).is_empty());
    assert!(tap(&mut app, KeyCode::Char('n')).is_empty());
    assert_eq!(app.modal, None, "`n` closes and sends nothing");
    assert!(tap(&mut app, KeyCode::Char('a')).is_empty());
    assert!(tap(&mut app, KeyCode::Esc).is_empty());
    assert_eq!(app.modal, None);
}

#[test]
fn one_task_starts_and_cancelled_tasks_do_not() {
    let (mut snap, windows) = gate_fixture();
    snap.runs[0].tasks[1].state = TaskState::Cancelled;
    let mut app = app_with_runs(windows, snap);
    open_run_view(&mut app, RUN_ID);
    tap(&mut app, KeyCode::Char('a'));
    assert_eq!(
        confirm(&app).0,
        "Approve run add-reset-3f9a? 1 task starts."
    );
}

#[test]
fn x_asks_then_rejects() {
    let mut app = gate();
    select(&mut app, task_key("t2"));
    assert!(tap(&mut app, KeyCode::Char('x')).is_empty());
    assert_eq!(
        confirm(&app),
        (
            "Reject run add-reset-3f9a? Its branches and worktrees are removed; salvage refs are kept.",
            &PendingAction::RejectRun(RUN_ID.into())
        )
    );
    assert_eq!(
        tap(&mut app, KeyCode::Enter),
        send(RunRequest::Reject {
            run_id: RUN_ID.into()
        })
    );
    assert!(tap(&mut app, KeyCode::Char('x')).is_empty());
    assert!(tap(&mut app, KeyCode::Char('n')).is_empty());
    assert_eq!(app.modal, None);
}

#[test]
fn d_on_a_task_asks_then_removes() {
    let mut app = gate();
    select(&mut app, task_key("t2"));
    assert!(tap(&mut app, KeyCode::Char('d')).is_empty());
    assert_eq!(
        confirm(&app),
        (
            "Remove t2 from run add-reset-3f9a's plan?",
            &PendingAction::RemoveTask {
                run_id: RUN_ID.into(),
                task_id: "t2".into()
            }
        )
    );
    assert_eq!(
        tap(&mut app, KeyCode::Char('y')),
        edit(vec![PlanEdit::CancelTask {
            task_id: "t2".into()
        }])
    );
    assert!(tap(&mut app, KeyCode::Char('d')).is_empty());
    assert!(tap(&mut app, KeyCode::Char('N')).is_empty());
    assert_eq!(app.modal, None);
}

#[test]
fn gate_keys_outside_the_gate_toast() {
    for (fast, text) in [
        (
            false,
            "the plan gate is closed: run add-reset-3f9a is running",
        ),
        (
            true,
            "the plan gate is closed: run add-reset-3f9a is on the fast path",
        ),
    ] {
        let (mut snap, windows) = gate_fixture();
        snap.runs[0].state = RunState::Running;
        if fast {
            snap.runs[0].path = Some(RunPath::Fast);
        }
        let mut app = app_with_runs(windows, snap);
        open_run_view(&mut app, RUN_ID);
        select(&mut app, task_key("t1"));
        for c in ['a', 'x', 'e', 'd'] {
            app.toast("");
            assert!(tap(&mut app, KeyCode::Char(c)).is_empty(), "{c}");
            assert_eq!(app.modal, None, "{c}");
            assert_eq!(app.toast_text(), Some(text), "{c}");
        }
    }
    // Another state reads in the view's own words.
    let (mut snap, windows) = gate_fixture();
    snap.runs[0].state = RunState::Paused;
    let mut app = app_with_runs(windows, snap);
    open_run_view(&mut app, RUN_ID);
    tap(&mut app, KeyCode::Char('a'));
    assert_eq!(
        app.toast_text(),
        Some("the plan gate is closed: run add-reset-3f9a is paused")
    );
}

#[test]
fn gate_keys_do_nothing_in_the_project_overview() {
    let (snap, windows) = gate_fixture();
    let mut app = app_with_runs(windows, snap);
    prefix(&mut app);
    tap(&mut app, KeyCode::Char('T'));
    select(&mut app, NodeKey::Run(RUN_ID.into()));
    for c in ['a', 'x', 'e', 'd'] {
        assert!(tap(&mut app, KeyCode::Char(c)).is_empty());
        assert_eq!(app.modal, None);
    }
}

#[test]
fn e_or_d_on_the_root_toasts() {
    let mut app = gate();
    for c in ['e', 'd'] {
        app.toast("");
        assert!(tap(&mut app, KeyCode::Char(c)).is_empty());
        assert_eq!(app.modal, None);
        assert_eq!(app.toast_text(), Some("select a task to edit or remove"));
    }
}

#[test]
fn the_edit_form_sends_only_what_changed() {
    let mut app = gate();
    open_form(&mut app, "t1");
    assert_eq!(
        form(&app).value_parts(EditField::Strength),
        ("‹ policy ›".into(), Some("standard".into()))
    );
    assert_eq!(
        form(&app).value_parts(EditField::Model),
        ("policy".into(), Some("claude-sonnet-5".into()))
    );
    focus(&mut app, EditField::Effort);
    tap(&mut app, KeyCode::Right);
    focus(&mut app, EditField::Size);
    tap(&mut app, KeyCode::Left);
    assert_eq!(
        tap(&mut app, KeyCode::Enter),
        edit(vec![amend(
            Some(RouteSpec {
                runtime: Some(Runtime::Claude),
                model: None,
                strength: None,
                effort: Some(Effort::High),
            }),
            Some(Size::S)
        )])
    );
    assert!(form(&app).submitting);

    // Changing only the size sends no route.
    let mut app = gate();
    assert_eq!(
        submit_a_size_change(&mut app),
        edit(vec![amend(None, Some(Size::S))])
    );
}

#[test]
fn changing_the_runtime_clears_the_model() {
    let (mut snap, windows) = gate_fixture();
    let mut t1 = edit_fixture_task();
    t1.route_spec = RouteSpec {
        runtime: Some(Runtime::Claude),
        model: Some("claude-sonnet-5".into()),
        strength: Some(Strength::Standard),
        effort: None,
    };
    snap.runs[0].tasks[0] = t1;
    let mut app = app_with_runs(windows, snap);
    open_run_view(&mut app, RUN_ID);
    open_form(&mut app, "t1");
    assert_eq!(form(&app).model.text(), "claude-sonnet-5");
    tap(&mut app, KeyCode::Right);
    assert_eq!(form(&app).runtime, Some(Runtime::Codex));
    assert_eq!(form(&app).model.text(), "");
    assert_eq!(
        tap(&mut app, KeyCode::Enter),
        edit(vec![amend(
            Some(RouteSpec {
                runtime: Some(Runtime::Codex),
                model: None,
                strength: Some(Strength::Standard),
                effort: None,
            }),
            None
        )])
    );
}

#[test]
fn policy_is_a_choice() {
    let mut app = gate();
    open_form(&mut app, "t1");
    tap(&mut app, KeyCode::Left);
    assert_eq!(form(&app).runtime, None);
    assert_eq!(
        form(&app).value_parts(EditField::Runtime),
        ("‹ policy ›".into(), Some("claude".into()))
    );
    assert_eq!(
        tap(&mut app, KeyCode::Enter),
        edit(vec![amend(
            Some(RouteSpec {
                runtime: None,
                model: None,
                strength: None,
                effort: Some(Effort::Medium),
            }),
            None
        )])
    );
}

#[test]
fn a_mode_other_than_tdd_needs_a_reason() {
    let mut app = gate();
    open_form(&mut app, "t1");
    focus(&mut app, EditField::TestMode);
    tap(&mut app, KeyCode::Right);
    assert_eq!(form(&app).test_mode, TestMode::Check);
    assert!(tap(&mut app, KeyCode::Enter).is_empty());
    assert_eq!(form(&app).focus, EditField::Reason);
    assert_eq!(
        form(&app).error.as_deref(),
        Some("a reason is required when test mode is check or none")
    );
    assert!(!form(&app).submitting);
    typed(&mut app, "renames only");
    assert_eq!(
        tap(&mut app, KeyCode::Enter),
        edit(vec![PlanEdit::AmendTask {
            task_id: "t1".into(),
            brief: None,
            acceptance: None,
            route: None,
            test_mode: Some(TestMode::Check),
            test_mode_reason: Some("renames only".into()),
            priority: None,
            size: None,
            deps: None,
        }])
    );
}

#[test]
fn the_brief_round_trips_its_newlines() {
    let mut app = gate();
    open_form(&mut app, "t1");
    focus(&mut app, EditField::Brief);
    assert_eq!(form(&app).brief.text(), "Line one↵Line two");
    assert!(press(&mut app, KeyCode::Char('j'), KeyModifiers::CONTROL).is_empty());
    typed(&mut app, "x");
    assert_eq!(
        tap(&mut app, KeyCode::Enter),
        edit(vec![PlanEdit::AmendTask {
            task_id: "t1".into(),
            brief: Some("Line one\nLine two\nx".into()),
            acceptance: None,
            route: None,
            test_mode: None,
            test_mode_reason: None,
            priority: None,
            size: None,
            deps: None,
        }])
    );
}

#[test]
fn nothing_changed_closes_with_a_toast() {
    let mut app = gate();
    open_form(&mut app, "t1");
    // A change undone is no change.
    focus(&mut app, EditField::Size);
    tap(&mut app, KeyCode::Right);
    tap(&mut app, KeyCode::Right);
    assert!(tap(&mut app, KeyCode::Enter).is_empty());
    assert_eq!(app.modal, None);
    assert_eq!(app.toast_text(), Some("nothing changed"));
}

#[test]
fn esc_closes_the_form_and_sends_nothing() {
    let mut app = gate();
    open_form(&mut app, "t1");
    tap(&mut app, KeyCode::Right);
    assert!(tap(&mut app, KeyCode::Esc).is_empty());
    assert_eq!(app.modal, None);
    assert!(app.run_view.is_some(), "the run view stays open");
    open_form(&mut app, "t1");
    assert!(press(&mut app, KeyCode::Char('c'), KeyModifiers::CONTROL).is_empty());
    assert_eq!(app.modal, None);
}
