//! Milestone 9.9.5 (decision 18): problems only the user can fix are tagged where the
//! engine raises them, and the orchestrator may not retry them (decision 10).

use proto::{BlockReason, DeliveryAlertKind};
use serde_json::json;

use super::actions_fixtures::{halted, halted_retryable};
use super::control::blocked;
use super::delivery_watch::{next_poll, view_answer, watched};
use super::dispatch::replies;
use super::fixture::*;
use super::orch::{edit_plan, error};
use super::turns::working;
use super::wake_notes::approved;
use crate::headless::FailureKind;
use crate::host::HostError;
use crate::run::delivery::ops::HostResult;
use crate::run::engine::stages::set_stage_head;
use crate::run::engine::user_only::marked;
use crate::run::engine::{OpResult, TurnOutcome};
use crate::run::snapshot::snapshot;

#[test]
fn the_markers_match_case_insensitively_and_nothing_else() {
    for text in [
        "error: No space left on device (os error 28)",
        "fatal: unable to auto-detect email address\n*** Please tell me who you are.",
        "Author identity unknown",
        "remote: Permission denied (publickey).",
        "gh: To get started with GitHub CLI, please run:  gh auth login",
        "sh: cargo-nextest: command not found",
    ] {
        assert!(marked(text), "{text}");
    }
    for text in [
        "the build cache is locked",
        "could not merge: conflict in a.rs",
        "permission denied: a.rs is outside owns",
    ] {
        assert!(!marked(text), "{text}");
    }
}

fn blocked_with(text: &str) -> Fixture {
    let mut fx = approved();
    let windows = fx.launch_all();
    let window = windows.iter().find(|(t, _)| t == "t1").unwrap().1;
    blocked(&mut fx, window, "environment", text);
    fx
}

#[test]
fn a_marked_environment_block_is_user_only_and_the_orchestrator_may_not_retry_it() {
    let mut fx = blocked_with("git commit failed: Author identity unknown");
    let block = fx.task("t1").block.clone().unwrap();
    assert_eq!(
        (block.reason, block.user_only),
        (BlockReason::Environment, true)
    );
    let effects = edit_plan(
        &mut fx,
        json!({"edits": [{"op": "retry", "task_id": "t1", "reason": "again"}]}),
    );
    assert_eq!(
        error(&effects),
        "task t1 waits on something only the user can fix, and they have been alerted: git commit failed: Author identity unknown"
    );
    // The user's retry is not refused for it.
    assert!(replies(&super::control::retry(&mut fx, "t1"))[0].is_ok());
}

#[test]
fn an_ordinary_environment_block_is_the_orchestrators() {
    let fx = blocked_with("the build cache is locked");
    assert!(!fx.task("t1").block.as_ref().unwrap().user_only);
}

#[test]
fn a_logged_out_cli_blocks_user_only() {
    for (kind, error) in [
        (FailureKind::Authentication, "all fine, nothing marked"),
        (FailureKind::Billing, "add credits"),
        (FailureKind::SandboxUnavailable, "sandbox refused"),
    ] {
        let (mut fx, window) = working();
        fx.turn_ended(
            window,
            TurnOutcome::Failed {
                error: error.into(),
                kind,
            },
        );
        let block = fx.task("t1").block.clone().expect("blocked");
        assert_eq!(
            (block.reason, block.user_only),
            (BlockReason::Environment, true),
            "{kind:?}"
        );
    }
    // A client error stays the orchestrator's (ruling R2).
    let (mut fx, window) = working();
    let kind = FailureKind::ClientError;
    fx.turn_ended(
        window,
        TurnOutcome::Failed {
            error: "too long".into(),
            kind,
        },
    );
    assert!(!fx.task("t1").block.clone().unwrap().user_only);
}

#[test]
fn a_missing_cli_blocks_user_only() {
    let mut fx = approved();
    fx.complete_prepares();
    let (op, _) = fx.op("CreateWindow");
    let message = "No such file or directory (os error 2)".to_string();
    fx.done(op, OpResult::Failed { message });
    let block = fx.task("t1").block.clone().expect("blocked");
    assert_eq!(
        (block.reason, block.user_only),
        (BlockReason::Environment, true)
    );
}

#[test]
fn a_rebaseline_halt_is_user_only_in_the_snapshot() {
    let fx = halted();
    assert!(snapshot(&fx.state, fx.now).runs[0].halt_user_only);
    let fx = halted_retryable();
    assert!(!snapshot(&fx.state, fx.now).runs[0].halt_user_only);
}

fn user_only_alerts(fx: &Fixture) -> Vec<(DeliveryAlertKind, bool)> {
    let run = &snapshot(&fx.state, fx.now).runs[0];
    (run.delivery.iter().flat_map(|d| &d.alerts))
        .map(|a| (a.kind, a.user_only))
        .collect()
}

#[test]
fn a_logged_out_gh_and_a_credential_push_are_user_only_alerts() {
    let mut fx = watched();
    let at = next_poll(&fx);
    assert!(super::delivery_watch::polls(&mut fx, at));
    let lost = HostError::Auth("gh auth login".into());
    view_answer(&mut fx, at, HostResult::Error(lost));
    assert_eq!(
        user_only_alerts(&fx),
        vec![(DeliveryAlertKind::GhLoggedOut, true)]
    );

    for (text, want) in [
        (
            "remote: Permission to o/r.git denied to x. fatal: Authentication failed",
            true,
        ),
        ("boom", false),
    ] {
        let mut fx = watched();
        fx.run_mut().delivery.watching = false;
        set_stage_head(fx.run_mut(), 1, &super::merge::commit(5));
        for _ in 0..7 {
            fx.tick();
            let (op, _) = super::delivery_open::host_op(&fx);
            let error = HostResult::Error(HostError::Failed(text.into()));
            super::delivery_open::answer(&mut fx, op, error);
        }
        assert_eq!(
            user_only_alerts(&fx),
            vec![(DeliveryAlertKind::HostOpHeld, want)],
            "{text}"
        );
    }
}

#[test]
fn the_orchestrator_may_not_resume_a_halt_that_names_a_user_only_cause() {
    let mut fx = halted_retryable();
    let orch = super::orch::launched(false).run().orch.orchestrator.clone();
    fx.run_mut().orch.orchestrator = orch;
    fx.run_mut().halted_reason = Some("git failed: No space left on device".into());
    assert!(snapshot(&fx.state, fx.now).runs[0].halt_user_only);
    let effects = edit_plan(
        &mut fx,
        json!({"edits": [{"op": "resume_run", "reason": "try again"}]}),
    );
    let run = fx.run().id.clone();
    assert_eq!(
        error(&effects),
        format!(
            "run {run} waits on something only the user can fix, and they have been alerted: git failed: No space left on device"
        )
    );
}

#[test]
fn the_orchestrator_may_not_release_a_stage_held_on_a_user_only_cause() {
    let mut fx = watched();
    let orch = super::orch::launched(false).run().orch.orchestrator.clone();
    fx.run_mut().orch.orchestrator = orch;
    let held = "remote: Permission denied (publickey).";
    fx.run_mut().delivery.stages[0].held = Some(held.into());
    let effects = edit_plan(
        &mut fx,
        json!({"edits": [{"op": "resume_run", "stage": 1, "reason": "push again"}]}),
    );
    assert_eq!(
        error(&effects),
        format!(
            "stage 1 waits on something only the user can fix, and they have been alerted: {held}"
        )
    );
    assert!(fx.run().delivery.stages[0].held.is_some());
}
