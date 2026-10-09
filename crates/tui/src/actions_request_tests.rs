//! §1.1's table, one test per row: each daemon-backed kind sends exactly the request
//! the CLI sends for the same step (Global Constraint 2).

use super::*;
use crate::tree::run_fixtures::{PROJECT, run};
use proto::{ActionKind, MessageKind, MessageTarget, PlanEdit, RunState, Runtime};

const ID: &str = "r-0723";

fn info() -> RunInfo {
    run(ID, PROJECT, RunState::Running)
}

fn on_run(kind: ActionKind, input: ActionInput) -> Option<RunRequest> {
    request_for(&info(), &ActionTarget::Run, &kind, &input)
}

fn on_task(kind: ActionKind, input: ActionInput) -> Option<RunRequest> {
    request_for(&info(), &ActionTarget::Task("t1".into()), &kind, &input)
}

fn edit(edits: Vec<PlanEdit>, submit: bool) -> Option<RunRequest> {
    Some(RunRequest::Edit {
        run_id: ID.into(),
        edits,
        submit,
    })
}

#[test]
fn approve_sends_approve() {
    let want = Some(RunRequest::Approve { run_id: ID.into() });
    assert_eq!(on_run(ActionKind::Approve, ActionInput::None), want);
}

#[test]
fn reject_sends_reject() {
    let want = Some(RunRequest::Reject { run_id: ID.into() });
    assert_eq!(on_run(ActionKind::Reject, ActionInput::None), want);
}

#[test]
fn submit_sends_an_empty_edit_that_submits() {
    assert_eq!(
        on_run(ActionKind::Submit, ActionInput::None),
        edit(vec![], true)
    );
}

#[test]
fn approve_and_reject_hold_send_the_hold() {
    let hold = "epic:auth".to_string();
    assert_eq!(
        on_run(
            ActionKind::ApproveHold { hold: hold.clone() },
            ActionInput::None
        ),
        Some(RunRequest::ApproveHold {
            run_id: ID.into(),
            hold: hold.clone(),
        })
    );
    assert_eq!(
        on_run(
            ActionKind::RejectHold { hold: hold.clone() },
            ActionInput::None
        ),
        Some(RunRequest::RejectHold {
            run_id: ID.into(),
            hold,
        })
    );
}

#[test]
fn pause_and_unpause_are_edits() {
    assert_eq!(
        on_run(ActionKind::Pause, ActionInput::None),
        edit(vec![PlanEdit::Pause], false)
    );
    assert_eq!(
        on_run(ActionKind::Unpause, ActionInput::None),
        edit(vec![PlanEdit::Resume], false)
    );
}

#[test]
fn resume_carries_its_rebaseline() {
    for rebaseline in [true, false] {
        assert_eq!(
            on_run(ActionKind::Resume, ActionInput::Resume { rebaseline }),
            Some(RunRequest::Resume {
                run_id: ID.into(),
                rebaseline,
            })
        );
    }
    // Preflight F32: a held tier 3 retries with no form, without a rebaseline.
    assert_eq!(
        on_run(ActionKind::Resume, ActionInput::None),
        Some(RunRequest::Resume {
            run_id: ID.into(),
            rebaseline: false,
        })
    );
}

#[test]
fn cancel_sends_cancel() {
    let want = Some(RunRequest::Cancel { run_id: ID.into() });
    assert_eq!(on_run(ActionKind::Cancel, ActionInput::None), want);
}

#[test]
fn promote_carries_its_orchestrator() {
    let choice = OrchestratorChoice {
        runtime: Runtime::Codex,
        model: Some("gpt-6-sol".into()),
        effort: None,
    };
    assert_eq!(
        on_run(
            ActionKind::Promote,
            ActionInput::Promote(Some(choice.clone()))
        ),
        Some(RunRequest::Promote {
            run_id: ID.into(),
            orchestrator: Some(choice),
        })
    );
}

#[test]
fn accept_and_discard_confirm_with_the_run_id() {
    for (kind, action) in [
        (ActionKind::Accept, FinishAction::Accept),
        (ActionKind::Discard, FinishAction::Discard),
    ] {
        assert_eq!(
            on_run(kind, ActionInput::None),
            Some(RunRequest::Finish {
                run_id: ID.into(),
                action,
                confirm: Some(ID.into()),
            })
        );
    }
}

#[test]
fn message_stage_targets_the_stage() {
    let input = ActionInput::Message {
        kind: MessageKind::Change,
        text: "use the new api".into(),
    };
    let got = request_for(
        &info(),
        &ActionTarget::Stage(2),
        &ActionKind::MessageStage { stage: 2 },
        &input,
    );
    assert_eq!(
        got,
        edit(
            vec![PlanEdit::Message {
                to: MessageTarget::Stage(2),
                text: "use the new api".into(),
                kind: MessageKind::Change,
            }],
            false
        )
    );
}

#[test]
fn answer_is_an_answer_edit() {
    assert_eq!(
        on_task(ActionKind::Answer, ActionInput::Answer("a.txt".into())),
        edit(
            vec![PlanEdit::Answer {
                task_id: "t1".into(),
                text: "a.txt".into(),
            }],
            false
        )
    );
}

#[test]
fn message_targets_the_task() {
    let input = ActionInput::Message {
        kind: MessageKind::Info,
        text: "fyi".into(),
    };
    assert_eq!(
        on_task(ActionKind::Message, input),
        edit(
            vec![PlanEdit::Message {
                to: MessageTarget::Tasks(vec!["t1".into()]),
                text: "fyi".into(),
                kind: MessageKind::Info,
            }],
            false
        )
    );
}

#[test]
fn refresh_is_a_refresh_edit() {
    assert_eq!(
        on_task(ActionKind::Refresh, ActionInput::None),
        edit(
            vec![PlanEdit::Refresh {
                task_id: "t1".into()
            }],
            false
        )
    );
}

#[test]
fn retry_sends_retry() {
    assert_eq!(
        on_task(ActionKind::Retry, ActionInput::None),
        Some(RunRequest::Retry {
            run_id: ID.into(),
            task_id: "t1".into(),
        })
    );
}

#[test]
fn override_carries_its_reason() {
    assert_eq!(
        on_task(ActionKind::Override, ActionInput::Reason("flaky ci".into())),
        Some(RunRequest::Override {
            run_id: ID.into(),
            task_id: "t1".into(),
            reason: "flaky ci".into(),
        })
    );
}

#[test]
fn cancel_task_is_a_cancel_edit() {
    assert_eq!(
        on_task(ActionKind::CancelTask, ActionInput::None),
        edit(
            vec![PlanEdit::CancelTask {
                task_id: "t1".into()
            }],
            false
        )
    );
}

#[test]
fn local_kinds_make_no_request() {
    for kind in [
        ActionKind::ReviewPlan,
        ActionKind::Stats,
        ActionKind::OpenConversation,
    ] {
        assert_eq!(on_run(kind.clone(), ActionInput::None), None, "{kind:?}");
        assert_eq!(on_task(kind.clone(), ActionInput::None), None, "{kind:?}");
    }
}

#[test]
fn a_kind_on_the_wrong_node_or_without_its_input_makes_no_request() {
    assert_eq!(on_task(ActionKind::Approve, ActionInput::None), None);
    assert_eq!(on_run(ActionKind::Retry, ActionInput::None), None);
    assert_eq!(on_task(ActionKind::Answer, ActionInput::None), None);
    assert_eq!(on_task(ActionKind::Override, ActionInput::None), None);
    let stage = request_for(
        &info(),
        &ActionTarget::Stage(1),
        &ActionKind::MessageStage { stage: 2 },
        &ActionInput::Message {
            kind: MessageKind::Info,
            text: "x".into(),
        },
    );
    assert_eq!(stage, None);
}

#[test]
fn moved_base_confirm_is_the_full_sha() {
    let to = "0123456789abcdef0123456789abcdef01234567".to_string();
    let moved = BaseMovedInfo {
        from: "f".repeat(40),
        to: to.clone(),
        commits: vec![],
        total: 1,
    };
    assert_eq!(moved_base_confirm("r-0723", &moved), format!("r-0723@{to}"));
    assert_eq!(
        moved_base_confirm("r-0723", &moved).len(),
        "r-0723@".len() + 40
    );
}

#[test]
fn short_id_is_the_last_four() {
    assert_eq!(short_id("add-mul-0723"), "0723");
    assert_eq!(short_id("abc"), "abc");
    assert_eq!(short_id(""), "");
    assert_eq!(short_id("run-é1ü2"), "é1ü2");
}
