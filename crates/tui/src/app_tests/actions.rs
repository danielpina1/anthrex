//! Milestone 9.0.6 task 10: the action menu, its confirmation pages and the replies
//! (decisions 10, 12–14, 16, 19). The snapshots carry hand-built `ActionInfo`s, so the
//! client is tested without the daemon.

use super::runs::{app_with_runs, deliver, open_run_view};
use super::*;
use crate::actions_request::ActionTarget;
use crate::app::actions::ActionStep;
use crate::app::replies::REPLY_TIMEOUT;
use crate::tree::NodeKey;
use crate::tree::run_fixtures::{RUN_ID, gate_fixture, three_task_fixture};
use crate::tree::stage_fixtures::staged_fixture;
use proto::{ActionInfo, ActionKind, FinishAction, RunReply, RunRequest, RunsSnapshot};
use std::time::{Duration, Instant};

/// One daemon-built entry: `needs` from §1.1's table, `destructive` from the kind.
pub(super) fn action(kind: ActionKind, label: &str, refused: Option<&str>) -> ActionInfo {
    ActionInfo {
        needs: kind.needs(),
        destructive: kind.destructive(),
        effect: format!("{label}: the effect"),
        label: label.into(),
        refused_why: refused.map(str::to_owned),
        kind,
    }
}

const NOT_COMPLETE: &str = "run add-reset-3f9a is running; accept applies only to a complete run";

/// The gate fixture with `Approve` and `Reject` on the run and `Message`, `CancelTask`
/// on each task.
pub(super) fn gate_snapshot() -> (RunsSnapshot, Vec<WindowInfo>) {
    let (mut snap, windows) = gate_fixture();
    let run = &mut snap.runs[0];
    run.actions = vec![
        action(ActionKind::Approve, "approve", None),
        action(ActionKind::Reject, "reject", None),
    ];
    for task in &mut run.tasks {
        task.actions = vec![
            action(ActionKind::Message, "message", None),
            action(ActionKind::CancelTask, "cancel task", None),
        ];
    }
    (snap, windows)
}

/// The three-task fixture (running) with run and task actions; `Accept` and `Discard`
/// listed refused.
pub(super) fn running_snapshot() -> (RunsSnapshot, Vec<WindowInfo>) {
    let (mut snap, windows) = three_task_fixture();
    let run = &mut snap.runs[0];
    run.actions = vec![
        action(ActionKind::Pause, "pause", None),
        action(ActionKind::Cancel, "cancel run", None),
        action(ActionKind::Accept, "accept", Some(NOT_COMPLETE)),
        action(ActionKind::Discard, "discard", Some(NOT_COMPLETE)),
    ];
    run.tasks[1].actions = vec![
        action(ActionKind::Answer, "answer", None),
        action(ActionKind::Message, "message", None),
        action(ActionKind::Refresh, "refresh", None),
        action(ActionKind::CancelTask, "cancel task", None),
    ];
    run.tasks[2].actions = vec![
        action(ActionKind::Message, "message", None),
        action(ActionKind::CancelTask, "cancel task", None),
    ];
    (snap, windows)
}

pub(super) fn gate_app() -> App {
    let (snap, windows) = gate_snapshot();
    app_with_runs(windows, snap)
}

pub(super) fn running_app() -> App {
    let (snap, windows) = running_snapshot();
    app_with_runs(windows, snap)
}

pub(super) fn flow(app: &App) -> &crate::app::actions::ActionFlow {
    match &app.modal {
        Some(Modal::Action(flow)) => flow,
        other => panic!("no action menu: {other:?}"),
    }
}

pub(super) fn kinds(app: &App) -> Vec<ActionKind> {
    flow(app).items.iter().map(|a| a.kind.clone()).collect()
}

pub(super) fn tap(app: &mut App, code: KeyCode) -> Vec<Effect> {
    press(app, code, KeyModifiers::NONE)
}

/// The id of the one tagged request in `effects` matching `want`.
pub(super) fn sent_tagged_id(effects: &[Effect], want: impl Fn(&RunRequest) -> bool) -> u64 {
    let tagged: Vec<_> = effects
        .iter()
        .filter_map(|e| match e {
            Effect::Send(ClientMsg::RunTagged { id, request }) => Some((*id, request)),
            _ => None,
        })
        .collect();
    assert_eq!(tagged.len(), 1, "{effects:?}");
    assert!(want(tagged[0].1), "{effects:?}");
    tagged[0].0
}

fn select(app: &mut App, key: NodeKey) {
    let rows = crate::app::nav_rows_of(&app.windows, &app.runs, &app.tree, app.run_view.as_ref());
    app.tree.select(&rows, key.clone());
    assert_eq!(app.tree.selected, Some(key));
}

fn task_key(id: &str) -> NodeKey {
    NodeKey::Task {
        run: RUN_ID.into(),
        id: id.into(),
    }
}

/// Opens the run menu on the gate and moves to `kind`.
pub(super) fn gate_menu_at(app: &mut App, kind: ActionKind) {
    app.open_actions((RUN_ID.into(), ActionTarget::Run), Some(kind.clone()));
    let at = flow(app).items.iter().position(|a| a.kind == kind).unwrap();
    assert_eq!(flow(app).selected, at);
}

#[test]
fn dot_opens_the_menu_on_the_selected_node() {
    let mut app = running_app();
    open_run_view(&mut app, RUN_ID);
    assert!(tap(&mut app, KeyCode::Char('.')).is_empty());
    assert_eq!(flow(&app).target, ActionTarget::Run);
    assert_eq!(flow(&app).run_id, RUN_ID);
    tap(&mut app, KeyCode::Esc);
    assert!(app.modal.is_none(), "Esc closes the menu");

    select(&mut app, task_key("t1"));
    tap(&mut app, KeyCode::Char('.'));
    assert_eq!(flow(&app).target, ActionTarget::Task("t1".into()));
    tap(&mut app, KeyCode::Esc);

    // A stage node.
    let (snap, windows) = staged_fixture();
    let mut app = app_with_runs(windows, snap);
    open_run_view(&mut app, RUN_ID);
    select(
        &mut app,
        NodeKey::Stage {
            run: RUN_ID.into(),
            n: 1,
        },
    );
    tap(&mut app, KeyCode::Char('.'));
    assert_eq!(flow(&app).target, ActionTarget::Stage(1));

    // A project or a window node does nothing.
    let mut app = running_app();
    prefix(&mut app);
    tap(&mut app, KeyCode::Char('t'));
    for key in [NodeKey::Project("/r/demo".into()), NodeKey::Window(1)] {
        let rows = tree::build_with_runs(&app.windows, &app.runs.runs, &app.tree);
        app.tree.select(&rows, key.clone());
        assert_eq!(app.tree.selected, Some(key.clone()));
        assert!(tap(&mut app, KeyCode::Char('.')).is_empty());
        assert!(app.modal.is_none(), "{key:?}");
    }
}

#[test]
fn the_menu_lists_local_kinds_around_the_daemons() {
    let mut app = gate_app();
    app.open_actions((RUN_ID.into(), ActionTarget::Run), None);
    assert_eq!(
        kinds(&app),
        vec![
            ActionKind::ReviewPlan,
            ActionKind::Approve,
            ActionKind::Reject,
            ActionKind::Stats,
        ]
    );
    let review = &flow(&app).items[0];
    assert_eq!(review.label, "review plan");
    assert_eq!(
        review.effect,
        "review plan: read every task before approving"
    );
    assert_eq!(
        flow(&app).items[3].effect,
        "stats: this project's run history"
    );
    assert_eq!(flow(&app).selected, 0);

    // A running run with no hold: no review; stats still last.
    let mut app = running_app();
    app.open_actions((RUN_ID.into(), ActionTarget::Run), None);
    assert_eq!(kinds(&app)[0], ActionKind::Pause);
    assert_eq!(kinds(&app).last(), Some(&ActionKind::Stats));

    // A task with an agent round opens its conversation first; one without has none.
    app.modal = None;
    app.open_actions((RUN_ID.into(), ActionTarget::Task("t1".into())), None);
    assert_eq!(kinds(&app)[0], ActionKind::OpenConversation);
    assert_eq!(
        flow(&app).items[0].effect,
        "open conversation: t1's conversation, read-only"
    );
    app.modal = None;
    app.open_actions((RUN_ID.into(), ActionTarget::Task("t2".into())), None);
    assert_eq!(
        kinds(&app),
        vec![ActionKind::Message, ActionKind::CancelTask]
    );
}

#[test]
fn j_k_move_and_enter_on_a_refused_entry_toasts_its_reason() {
    let mut app = running_app();
    app.open_actions((RUN_ID.into(), ActionTarget::Run), None);
    tap(&mut app, KeyCode::Char('j'));
    tap(&mut app, KeyCode::Down);
    assert_eq!(
        flow(&app).items[flow(&app).selected].kind,
        ActionKind::Accept
    );
    assert!(tap(&mut app, KeyCode::Enter).is_empty());
    assert_eq!(app.toast_text(), Some(NOT_COMPLETE));
    assert_eq!(app.toast_level(), Some(ToastLevel::Warn));
    assert_eq!(flow(&app).step, ActionStep::Menu, "the menu stays");
    tap(&mut app, KeyCode::Char('k'));
    assert_eq!(flow(&app).selected, 1);
    for _ in 0..3 {
        tap(&mut app, KeyCode::Up);
    }
    assert_eq!(flow(&app).selected, 0, "k stops at the top");
    for _ in 0..9 {
        tap(&mut app, KeyCode::Char('j'));
    }
    assert_eq!(flow(&app).selected, flow(&app).items.len() - 1);
}

#[test]
fn disconnected_menu_offers_nothing() {
    let mut app = gate_app();
    app.on_link_lost("gone");
    gate_menu_at(&mut app, ActionKind::Approve);
    assert!(tap(&mut app, KeyCode::Enter).is_empty());
    assert_eq!(app.toast_text(), Some("not connected"));
    assert_eq!(flow(&app).step, ActionStep::Menu);
    for key in [KeyCode::Char('y'), KeyCode::Enter, KeyCode::Char('j')] {
        assert!(tap(&mut app, key).is_empty());
    }

    // A page reached while connected sends nothing once the link is gone.
    let mut app = gate_app();
    gate_menu_at(&mut app, ActionKind::Approve);
    tap(&mut app, KeyCode::Enter);
    app.on_link_lost("gone");
    assert!(tap(&mut app, KeyCode::Char('y')).is_empty());
    assert_eq!(app.toast_text(), Some("not connected"));
    assert!(app.replies.is_empty(), "nothing queued");
}

#[test]
fn a_confirm_kind_goes_to_its_page_then_sends_on_y() {
    let mut app = gate_app();
    gate_menu_at(&mut app, ActionKind::Approve);
    assert!(tap(&mut app, KeyCode::Enter).is_empty());
    let ActionStep::Confirm(page) = &flow(&app).step else {
        panic!("not a page: {:?}", flow(&app).step);
    };
    assert_eq!(page.info.effect, "approve: the effect");
    assert!(!page.y_only());
    // Esc goes back to the menu, Enter again to the page.
    tap(&mut app, KeyCode::Esc);
    assert_eq!(flow(&app).step, ActionStep::Menu);
    tap(&mut app, KeyCode::Enter);
    let effects = tap(&mut app, KeyCode::Char('y'));
    assert_eq!(
        effects,
        vec![Effect::Send(ClientMsg::RunTagged {
            id: 1,
            request: RunRequest::Approve {
                run_id: RUN_ID.into()
            },
        })]
    );
    assert!(app.modal.is_none());
    assert!(app.replies.contains(1));

    // A non-destructive page also confirms on Enter.
    let mut app = gate_app();
    gate_menu_at(&mut app, ActionKind::Approve);
    tap(&mut app, KeyCode::Enter);
    let effects = tap(&mut app, KeyCode::Enter);
    sent_tagged_id(&effects, |r| matches!(r, RunRequest::Approve { .. }));

    // Cancel task is destructive: y only.
    let mut app = gate_app();
    app.open_actions(
        (RUN_ID.into(), ActionTarget::Task("t1".into())),
        Some(ActionKind::CancelTask),
    );
    tap(&mut app, KeyCode::Enter);
    assert!(
        tap(&mut app, KeyCode::Enter).is_empty(),
        "cancel task is y only"
    );
    assert_eq!(app.toast_text(), Some("press y to cancel task"));
}

#[test]
fn a_destructive_page_ignores_enter() {
    let mut app = super::actions_accept::app_with_complete_run("add-mul-0723");
    app.open_actions(
        ("add-mul-0723".into(), ActionTarget::Run),
        Some(ActionKind::Discard),
    );
    tap(&mut app, KeyCode::Enter);
    let ActionStep::Confirm(page) = &flow(&app).step else {
        panic!("not a page");
    };
    assert!(page.y_only());
    assert!(tap(&mut app, KeyCode::Enter).is_empty());
    assert_eq!(app.toast_text(), Some("press y to discard"));
    assert_eq!(app.toast_level(), Some(ToastLevel::Warn));
    let effects = tap(&mut app, KeyCode::Char('y'));
    sent_tagged_id(&effects, |r| {
        matches!(r, RunRequest::Finish { action: FinishAction::Discard, confirm: Some(c), .. }
            if c == "add-mul-0723")
    });

    // Accept is destructive-grade too (Global Constraint 3).
    let mut app = super::actions_accept::app_with_complete_run("add-mul-0723");
    app.open_actions(
        ("add-mul-0723".into(), ActionTarget::Run),
        Some(ActionKind::Accept),
    );
    tap(&mut app, KeyCode::Enter);
    assert!(tap(&mut app, KeyCode::Enter).is_empty());
    assert_eq!(app.toast_text(), Some("press y to accept"));
}

#[test]
fn done_and_refused_replies_toast_by_severity() {
    let mut app = gate_app();
    gate_menu_at(&mut app, ActionKind::Approve);
    tap(&mut app, KeyCode::Enter);
    let id = sent_tagged_id(&tap(&mut app, KeyCode::Char('y')), |_| true);
    app.on_run_reply(RunReply::Done {
        request: "approve".into(),
        message: "approved add-reset-3f9a".into(),
        request_id: Some(id),
    });
    assert_eq!(app.toast_text(), Some("approved add-reset-3f9a"));
    assert_eq!(app.toast_level(), Some(ToastLevel::Info));
    assert!(app.replies.is_empty());

    gate_menu_at(&mut app, ActionKind::Approve);
    tap(&mut app, KeyCode::Enter);
    let id = sent_tagged_id(&tap(&mut app, KeyCode::Char('y')), |_| true);
    app.on_run_reply(RunReply::Refused {
        request: "approve".into(),
        message: "run add-reset-3f9a is running\nsecond line".into(),
        request_id: Some(id),
    });
    assert_eq!(
        app.toast_text(),
        Some("run add-reset-3f9a is running (+1 more)")
    );
    assert_eq!(app.toast_level(), Some(ToastLevel::Error));
    assert!(app.replies.is_empty());

    // A ConfirmNeeded without a moved base is a warning of its prompt.
    gate_menu_at(&mut app, ActionKind::Approve);
    tap(&mut app, KeyCode::Enter);
    let id = sent_tagged_id(&tap(&mut app, KeyCode::Char('y')), |_| true);
    app.on_run_reply(RunReply::ConfirmNeeded {
        run_id: RUN_ID.into(),
        prompt: "really?".into(),
        base_moved: None,
        request_id: Some(id),
    });
    assert_eq!(app.toast_text(), Some("really?"));
    assert_eq!(app.toast_level(), Some(ToastLevel::Warn));
    assert!(app.replies.is_empty());
}

#[test]
fn a_reply_with_no_answer_times_out() {
    let mut app = gate_app();
    gate_menu_at(&mut app, ActionKind::Approve);
    tap(&mut app, KeyCode::Enter);
    let id = sent_tagged_id(&tap(&mut app, KeyCode::Char('y')), |_| true);
    app.on_tick();
    assert!(app.replies.contains(id), "not yet due");
    let past = Instant::now()
        .checked_sub(REPLY_TIMEOUT + Duration::from_secs(1))
        .unwrap();
    app.set_reply_sent_at(id, past);
    app.on_tick();
    assert_eq!(app.toast_text(), Some("no reply from daemon"));
    assert_eq!(app.toast_level(), Some(ToastLevel::Error));
    assert!(!app.replies.contains(id));

    // A late reply is still shown.
    app.on_run_reply(RunReply::Done {
        request: "approve".into(),
        message: "approved late".into(),
        request_id: Some(id),
    });
    assert_eq!(app.toast_text(), Some("approved late"));
}

#[test]
fn a_lost_link_drops_every_pending_reply() {
    let mut app = gate_app();
    gate_menu_at(&mut app, ActionKind::Approve);
    tap(&mut app, KeyCode::Enter);
    tap(&mut app, KeyCode::Char('y'));
    assert!(!app.replies.is_empty());
    app.on_link_lost("gone");
    assert!(app.replies.is_empty());
}

#[test]
fn the_menu_closes_when_its_node_goes() {
    let (snap, windows) = running_snapshot();
    let mut app = app_with_runs(windows, snap.clone());
    app.open_actions((RUN_ID.into(), ActionTarget::Task("t2".into())), None);
    let mut without = snap.clone();
    without.runs[0].tasks.remove(2);
    deliver(&mut app, without.clone());
    assert!(app.modal.is_none());
    assert_eq!(app.toast_text(), Some("t2 is gone"));

    // The run itself.
    app.open_actions((RUN_ID.into(), ActionTarget::Run), None);
    let mut none = without;
    none.runs.clear();
    deliver(&mut app, none);
    assert!(app.modal.is_none());
    assert_eq!(app.toast_text(), Some("Add password reset · 3f9a is gone"));

    // A snapshot that keeps the node re-reads its entries and keeps the selection.
    let mut app = app_with_runs(vec![], snap.clone());
    app.open_actions((RUN_ID.into(), ActionTarget::Run), Some(ActionKind::Cancel));
    let mut paused = snap;
    paused.runs[0].actions.remove(0);
    deliver(&mut app, paused);
    assert_eq!(
        flow(&app).items[flow(&app).selected].kind,
        ActionKind::Cancel
    );
}

#[test]
fn a_confirm_page_shows_a_new_refusal() {
    let (snap, windows) = gate_snapshot();
    let mut app = app_with_runs(windows, snap.clone());
    gate_menu_at(&mut app, ActionKind::Approve);
    tap(&mut app, KeyCode::Enter);
    let mut refused = snap;
    refused.runs[0].actions[0].refused_why = Some("run add-reset-3f9a is paused".into());
    deliver(&mut app, refused);
    let ActionStep::Confirm(page) = &flow(&app).step else {
        panic!("not a page");
    };
    assert_eq!(
        page.info.refused_why.as_deref(),
        Some("run add-reset-3f9a is paused")
    );
    assert_eq!(
        super::actions_replies::cell_fg(&app, "run add-reset-3f9a is paused"),
        {
            let failed = crate::theme::role(crate::theme::Role::Failed, app.palette());
            failed.fg
        }
    );
    assert!(tap(&mut app, KeyCode::Char('y')).is_empty());
    assert_eq!(app.toast_text(), Some("run add-reset-3f9a is paused"));
    assert_eq!(app.toast_level(), Some(ToastLevel::Warn));
    assert!(app.replies.is_empty());
}

#[test]
fn open_conversation_and_review_plan_act_locally() {
    let mut app = gate_app();
    gate_menu_at(&mut app, ActionKind::ReviewPlan);
    assert!(tap(&mut app, KeyCode::Enter).is_empty());
    assert!(app.modal.is_none());
    let review = app.plan_review.as_ref().expect("the review opened");
    assert_eq!(review.target, crate::app::ReviewTarget::Gate);

    // OpenConversation does what Enter on the task does.
    let mut app = running_app();
    let mut twin = running_app();
    let enter = twin.activate_run_node(task_key("t1"));
    app.open_actions(
        (RUN_ID.into(), ActionTarget::Task("t1".into())),
        Some(ActionKind::OpenConversation),
    );
    let effects = tap(&mut app, KeyCode::Enter);
    assert_eq!(effects, enter);
    assert!(app.modal.is_none());
    assert!(app.conversation.is_open());
    assert_eq!(app.conversation.window_id(), Some(6));
    assert!(
        !effects.iter().any(|e| matches!(
            e,
            Effect::Send(ClientMsg::Input { .. } | ClientMsg::Subscribe { .. })
        )),
        "Global Constraint 1: {effects:?}"
    );
}

/// No key in the menu, on any page, sends terminal input.
#[test]
fn no_menu_key_sends_input() {
    let mut app = running_app();
    let keys = [
        KeyCode::Char('j'),
        KeyCode::Enter,
        KeyCode::Char('x'),
        KeyCode::Char('y'),
        KeyCode::Esc,
        KeyCode::Char('k'),
        KeyCode::Enter,
        KeyCode::Char('a'),
        KeyCode::Backspace,
    ];
    for target in [ActionTarget::Run, ActionTarget::Task("t1".into())] {
        app.modal = None;
        app.open_actions((RUN_ID.into(), target), None);
        for key in keys {
            if app.modal.is_none() {
                break;
            }
            let effects = tap(&mut app, key);
            assert!(
                !effects
                    .iter()
                    .any(|e| matches!(e, Effect::Send(ClientMsg::Input { .. }))),
                "{effects:?}"
            );
        }
    }
}
