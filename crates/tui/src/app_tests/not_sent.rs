//! Milestone 9.0.6 final review I1: a tagged request whose send the connection refused
//! never left, so nothing waits for its reply: its `App.replies` entry goes at once, and
//! no `no reply from daemon` follows the `daemon is not responding` toast 30 s or 660 s
//! later. A screen or form that owns the request says so itself instead of a toast.

use super::actions::{gate_app, gate_menu_at, sent_tagged_id, tap};
use super::*;
use crate::actions_request::ActionTarget;
use crate::app::replies::{PendingWhat, reply_timeout};
use crate::tree::run_fixtures::RUN_ID;
use proto::{ActionKind, FinishAction, PlanEdit, RunRequest};
use std::time::{Duration, Instant};

/// Every action request the menu sends, beside the kind it is recorded as.
fn action_requests() -> Vec<(ActionKind, RunRequest)> {
    let run_id = || RUN_ID.to_string();
    vec![
        (
            ActionKind::Approve,
            RunRequest::Approve { run_id: run_id() },
        ),
        (ActionKind::Reject, RunRequest::Reject { run_id: run_id() }),
        (
            ActionKind::Pause,
            RunRequest::Edit {
                run_id: run_id(),
                edits: vec![PlanEdit::Pause],
                submit: false,
            },
        ),
        (
            ActionKind::Retry,
            RunRequest::Retry {
                run_id: run_id(),
                task_id: "t1".into(),
            },
        ),
        (ActionKind::Cancel, RunRequest::Cancel { run_id: run_id() }),
        (
            ActionKind::Accept,
            RunRequest::Finish {
                run_id: run_id(),
                action: FinishAction::Accept,
                confirm: Some(run_id()),
            },
        ),
        (
            ActionKind::Discard,
            RunRequest::Finish {
                run_id: run_id(),
                action: FinishAction::Discard,
                confirm: Some(run_id()),
            },
        ),
        (
            ActionKind::Resume,
            RunRequest::Resume {
                run_id: run_id(),
                rebaseline: false,
            },
        ),
        (
            ActionKind::Override,
            RunRequest::Override {
                run_id: run_id(),
                task_id: "t1".into(),
                reason: "flaky".into(),
            },
        ),
        (
            ActionKind::Promote,
            RunRequest::Promote {
                run_id: run_id(),
                orchestrator: None,
            },
        ),
        (
            ActionKind::ApproveHold { hold: "h".into() },
            RunRequest::ApproveHold {
                run_id: run_id(),
                hold: "h".into(),
            },
        ),
        (
            ActionKind::RejectHold { hold: "h".into() },
            RunRequest::RejectHold {
                run_id: run_id(),
                hold: "h".into(),
            },
        ),
    ]
}

/// The menu's own `Approve`, refused by a full queue while connected: one toast, and no
/// entry left to expire into a second.
#[test]
fn a_refused_action_send_is_not_waited_on() {
    let mut app = gate_app();
    gate_menu_at(&mut app, ActionKind::Approve);
    tap(&mut app, KeyCode::Enter);
    let effects = tap(&mut app, KeyCode::Char('y'));
    let id = sent_tagged_id(&effects, |r| matches!(r, RunRequest::Approve { .. }));
    let Some(Effect::Send(msg)) = effects.into_iter().next() else {
        panic!("no send");
    };
    assert!(app.replies.contains(id));
    assert!(app.on_send_failed(&msg).is_empty());
    assert!(!app.replies.contains(id), "never sent, never waited on");
    assert_eq!(app.toast_text(), Some("daemon is not responding"));
    assert!(
        app.replies
            .expire(Instant::now() + Duration::from_secs(700))
            .is_empty()
    );
}

/// Every action kind's request: its entry goes with the refused send.
#[test]
fn every_refused_action_send_drops_its_entry() {
    for (n, (kind, request)) in action_requests().into_iter().enumerate() {
        let mut app = gate_app();
        let id = 9_000 + n as u64;
        let what = PendingWhat::Action {
            run_id: RUN_ID.into(),
            target: ActionTarget::Run,
            kind,
        };
        app.replies.insert(id, what, reply_timeout(&request));
        app.on_send_failed(&ClientMsg::RunTagged {
            id,
            request: request.clone(),
        });
        assert!(app.replies.is_empty(), "{request:?}");
        assert_eq!(
            app.toast_text(),
            Some("daemon is not responding"),
            "{request:?}"
        );
    }
}
