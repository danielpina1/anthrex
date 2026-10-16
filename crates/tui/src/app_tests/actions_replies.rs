//! Milestone 9.0.6 task 10: the menu follows the snapshot, and the replies by id
//! (decisions 13, 16, 18, 19).

use super::actions::{flow, gate_menu_at, gate_snapshot, sent_tagged_id, tap};
use super::actions_accept::app_with_complete_run;
use super::runs::{app_with_runs, deliver};
use super::*;
use crate::actions_request::ActionTarget;
use crate::app::actions::ActionStep;
use crate::app::replies::{LONG_REPLY_TIMEOUT, REPLY_TIMEOUT, reply_timeout};
use crate::tree::run_fixtures::RUN_ID;
use crate::tree::stage_fixtures::staged_fixture;
use proto::{
    ActionKind, BaseMovedInfo, FinishAction, MessageKind, MessageTarget, PlanEdit, RunReply,
    RunRequest,
};
use std::time::{Duration, Instant};

/// The foreground colour of the first cell of `text` on an 80×24 frame of `app`.
pub(super) fn cell_fg(app: &App, text: &str) -> Option<ratatui::style::Color> {
    let mut terminal = ratatui::Terminal::new(ratatui::backend::TestBackend::new(80, 24)).unwrap();
    terminal
        .draw(|f| {
            crate::ui::draw(f, app);
        })
        .unwrap();
    let buffer = terminal.backend().buffer().clone();
    let want: Vec<char> = text.chars().collect();
    for y in 0..24 {
        let row: Vec<char> = (0..80)
            .map(|x| buffer[(x, y)].symbol().chars().next().unwrap_or(' '))
            .collect();
        if let Some(x) = row.windows(want.len()).position(|w| w == want.as_slice()) {
            return buffer[(x as u16, y)].fg.into();
        }
    }
    panic!("{text:?} not drawn");
}

#[test]
fn a_page_whose_action_left_goes_back_to_the_menu() {
    let (snap, windows) = gate_snapshot();
    let mut app = app_with_runs(windows, snap.clone());
    gate_menu_at(&mut app, ActionKind::Approve);
    tap(&mut app, KeyCode::Enter);
    let mut approved = snap;
    approved.runs[0].actions.clear();
    deliver(&mut app, approved);
    assert_eq!(flow(&app).step, ActionStep::Menu);
    assert_eq!(app.toast_text(), Some("approve is no longer available"));
    assert_eq!(app.toast_level(), Some(ToastLevel::Warn));
    assert!(tap(&mut app, KeyCode::Char('y')).is_empty());
}

#[test]
fn a_stage_that_left_closes_its_menu() {
    let (snap, windows) = staged_fixture();
    let mut app = app_with_runs(windows, snap.clone());
    app.open_actions((RUN_ID.into(), ActionTarget::Stage(2)), None);
    assert!(
        flow(&app).items.is_empty(),
        "the fixture's stages list nothing"
    );
    let mut one = snap;
    one.runs[0].stages.truncate(1);
    deliver(&mut app, one);
    assert!(app.modal.is_none());
    assert_eq!(app.toast_text(), Some("stage 2 is gone"));
}

/// Decision 19's split: one engine step or a bounded write waits 30 s; git waits 660 s.
#[test]
fn reply_timeout_follows_decision_19() {
    let id = || RUN_ID.to_string();
    let edit = |edits| RunRequest::Edit {
        run_id: id(),
        edits,
        submit: false,
    };
    let short = [
        RunRequest::Approve { run_id: id() },
        RunRequest::Reject { run_id: id() },
        edit(vec![PlanEdit::Pause]),
        edit(vec![PlanEdit::Message {
            to: MessageTarget::Tasks(vec!["t1".into()]),
            text: "x".into(),
            kind: MessageKind::Info,
        }]),
        RunRequest::ApproveHold {
            run_id: id(),
            hold: "h".into(),
        },
        RunRequest::RejectHold {
            run_id: id(),
            hold: "h".into(),
        },
        RunRequest::Retry {
            run_id: id(),
            task_id: "t1".into(),
        },
        RunRequest::Cancel { run_id: id() },
        RunRequest::Stats {
            dir: "/r".into(),
            apply: Vec::new(),
            dismiss: Vec::new(),
            read_only: false,
        },
        RunRequest::TaskDetail {
            run_id: id(),
            task_id: "t1".into(),
        },
    ];
    for request in short {
        assert_eq!(reply_timeout(&request), REPLY_TIMEOUT, "{request:?}");
    }
    let long = [
        RunRequest::Finish {
            run_id: id(),
            action: FinishAction::Accept,
            confirm: Some(id()),
            reason: None,
        },
        RunRequest::Finish {
            run_id: id(),
            action: FinishAction::Discard,
            confirm: Some(id()),
            reason: None,
        },
        RunRequest::Resume {
            run_id: id(),
            rebaseline: true,
        },
        RunRequest::Override {
            run_id: id(),
            task_id: "t1".into(),
            reason: "r".into(),
        },
        RunRequest::Promote {
            run_id: id(),
            orchestrator: None,
        },
        edit(vec![PlanEdit::Refresh {
            task_id: "t1".into(),
        }]),
        RunRequest::Profile(proto::ProfileRequest::Detect {
            dir: "/r".into(),
            trust_project: false,
            unconfined_checks: false,
        }),
    ];
    for request in long {
        assert_eq!(reply_timeout(&request), LONG_REPLY_TIMEOUT, "{request:?}");
    }
    assert_eq!(REPLY_TIMEOUT, Duration::from_secs(30));
    assert_eq!(LONG_REPLY_TIMEOUT, Duration::from_secs(600 + 60));
}

/// A `ConfirmNeeded` whose request already timed out is shown, and opens no page.
#[test]
fn a_late_moved_base_confirm_needed_only_toasts() {
    let mut app = app_with_complete_run("add-mul-0723");
    app.open_actions(
        ("add-mul-0723".into(), ActionTarget::Run),
        Some(ActionKind::Accept),
    );
    tap(&mut app, KeyCode::Enter);
    let id = sent_tagged_id(&tap(&mut app, KeyCode::Char('y')), |_| true);
    let past = Instant::now()
        .checked_sub(LONG_REPLY_TIMEOUT + Duration::from_secs(1))
        .unwrap();
    app.set_reply_sent_at(id, past);
    app.on_tick();
    assert!(app.replies.is_empty());
    app.on_run_reply(RunReply::ConfirmNeeded {
        run_id: "add-mul-0723".into(),
        prompt: "merge onto the moved main?".into(),
        base_moved: Some(BaseMovedInfo {
            from: "b".repeat(40),
            to: "a".repeat(40),
            commits: vec![],
            total: 1,
        }),
        request_id: Some(id),
    });
    assert!(
        app.modal.is_none(),
        "no page for a request no longer pending"
    );
    assert_eq!(app.toast_text(), Some("merge onto the moved main?"));
    assert_eq!(app.toast_level(), Some(ToastLevel::Warn));
}

/// A reply ends only the entry whose id it carries.
#[test]
fn a_done_for_one_id_leaves_the_other_pending() {
    let mut app = super::actions::gate_app();
    let mut ids = vec![];
    for _ in 0..2 {
        gate_menu_at(&mut app, ActionKind::Approve);
        tap(&mut app, KeyCode::Enter);
        ids.push(sent_tagged_id(&tap(&mut app, KeyCode::Char('y')), |_| true));
    }
    app.on_run_reply(RunReply::Done {
        request: "approve".into(),
        message: "approved".into(),
        request_id: Some(ids[1]),
    });
    assert!(app.replies.contains(ids[0]));
    assert!(!app.replies.contains(ids[1]));
}

/// Milestone 9.15 decision 32 (protocol 24): a `Lessons` or `LessonsGraph` reply ends
/// the entry its id names and is otherwise dropped, with no toast, until the Lessons
/// page (task M9.15.12) and the graph page (M9.15.13) read them.
#[test]
fn lessons_replies_route_by_id_and_are_otherwise_dropped() {
    let mut app = super::actions::gate_app();
    let dir = std::path::PathBuf::from("/repo");
    for (id, reply) in [
        (
            41,
            RunReply::Lessons {
                lessons: vec![],
                message: None,
                request_id: Some(41),
            },
        ),
        (
            42,
            RunReply::LessonsGraph {
                graph: Box::default(),
                request_id: Some(42),
            },
        ),
    ] {
        let what = crate::app::replies::PendingWhat::Stats { dir: dir.clone() };
        app.replies.insert(id, what, REPLY_TIMEOUT);
        assert!(app.on_run_reply(reply).is_empty());
        assert!(!app.replies.contains(id), "the entry of {id} ends");
    }
    let untagged = RunReply::Lessons {
        lessons: vec![],
        message: Some("approved 1".into()),
        request_id: None,
    };
    assert!(app.on_run_reply(untagged).is_empty());
    assert_eq!(app.toast_text(), None);
    assert!(app.modal.is_none());
}
