//! Milestone 9.9 (OFA §4.2): the orchestrator's retry, override and approve_hold, with
//! the user's preconditions, its reason, the run log, the handled list and the edit log.

use proto::{HoldState, RunState, TaskState};
use serde_json::json;

use super::control::{blocked, retry};
use super::dispatch::replies;
use super::fixture::*;
use super::orch::{edit_plan, error};
use super::wake_notes::{approved, notes};

/// [`approved`] with `t1` working in its window and then blocked as `kind`.
fn blocked_t1(kind: &str, reason: &str) -> Fixture {
    let mut fx = approved();
    let windows = fx.launch_all();
    let (_, window) = windows.iter().find(|(t, _)| t == "t1").cloned().unwrap();
    blocked(&mut fx, window, kind, reason);
    fx
}

fn op(fx: &mut Fixture, edit: serde_json::Value) -> Vec<crate::run::engine::Effect> {
    edit_plan(fx, json!({"edits": [edit]}))
}

fn logged(fx: &Fixture, line: &str) -> bool {
    fx.run().log.iter().any(|e| e.text == line)
}

#[test]
fn the_orchestrator_retries_an_environment_block_as_the_user_would() {
    let mut fx = blocked_t1("environment", "the build cache is locked");
    let effects = op(
        &mut fx,
        json!({"op": "retry", "task_id": "t1", "reason": "the lock was transient"}),
    );
    let text = replies(&effects)[0].clone().unwrap();
    assert!(text.starts_with("task t1 retried at rung 2: "), "{text}");
    let t1 = fx.task("t1");
    assert_ne!(t1.state, TaskState::Blocked);
    assert_eq!(t1.rung, 2);
    assert!(
        t1.history.iter().any(|e| e.text.starts_with("retried by the orchestrator at rung 2 (it was blocked(environment): the build cache is locked)")),
        "{:#?}", t1.history
    );
    assert!(logged(
        &fx,
        "orchestrator: retried t1 — the lock was transient"
    ));
    let handled = &fx.run().orch.handled;
    assert_eq!(handled.len(), 1);
    assert_eq!(
        (
            handled[0].op.as_str(),
            handled[0].target.as_str(),
            handled[0].reason.as_str()
        ),
        ("retry", "t1", "the lock was transient")
    );
    assert_eq!(fx.run().orch.handled_total, 1);
    let entry = fx.run().plan_edits.last().unwrap();
    assert_eq!(
        (entry.text.as_str(), entry.source.as_str(), entry.accepted),
        ("retry t1", "orchestrator", true)
    );
}

#[test]
fn the_orchestrators_retry_is_refused_where_the_users_is() {
    // Not blocked: the user's refusal, word for word.
    let mut fx = approved();
    fx.launch_all();
    let user = replies(&retry(&mut fx, "t1"))[0].clone().unwrap_err();
    let effects = op(
        &mut fx,
        json!({"op": "retry", "task_id": "t1", "reason": "why not"}),
    );
    assert_eq!(error(&effects), user);
    assert!(fx.run().orch.handled.is_empty());
    let entry = fx.run().plan_edits.last().unwrap();
    assert!(!entry.accepted);
}

#[test]
fn an_op_needs_a_reason_and_comes_alone() {
    let mut fx = blocked_t1("environment", "locked");
    for reason in ["", "   "] {
        let effects = op(
            &mut fx,
            json!({"op": "retry", "task_id": "t1", "reason": reason}),
        );
        assert_eq!(error(&effects), "retry needs a reason the user can read");
    }
    let long = "x".repeat(501);
    let effects = op(
        &mut fx,
        json!({"op": "retry", "task_id": "t1", "reason": long}),
    );
    assert_eq!(error(&effects), "reason: at most 500 characters");
    let effects = edit_plan(
        &mut fx,
        json!({"edits": [
            {"op": "retry", "task_id": "t1", "reason": "r"},
            {"op": "cancel_task", "task_id": "t2"}
        ]}),
    );
    assert_eq!(error(&effects), crate::run::orch::contract::ACTION_ALONE);
    assert_eq!(fx.task("t1").state, TaskState::Blocked, "nothing applied");
}

#[test]
fn the_orchestrator_overrides_a_task_in_review_with_its_reason() {
    let mut fx = approved();
    fx.launch_all();
    // A claimed head, as a task in review has: without one the merge queue blocks it at
    // once ("no claimed commit to merge") in the step that follows the override.
    fx.task_mut("t1").head = Some(HEAD.into());
    fx.force("t1", TaskState::Review);
    let effects = op(
        &mut fx,
        json!({"op": "override", "task_id": "t1", "reason": "the reviewer loops on a style nit"}),
    );
    assert_eq!(
        replies(&effects)[0].clone().unwrap(),
        "task t1 goes to the merge queue without review: the reviewer loops on a style nit"
    );
    assert_eq!(fx.task("t1").state, TaskState::MergeQueue);
    assert!(logged(
        &fx,
        "orchestrator: overrode t1 — the reviewer loops on a style nit"
    ));
    assert_eq!(fx.run().orch.handled[0].op, "override");
}

#[test]
fn an_override_that_counts_commits_records_when_it_lands() {
    let mut fx = blocked_t1("environment", "locked");
    fx.task_mut("t1").start_commit = Some(BASE.into());
    let effects = op(
        &mut fx,
        json!({"op": "override", "task_id": "t1", "reason": "work is done"}),
    );
    assert!(
        replies(&effects).is_empty(),
        "the reply waits for the count"
    );
    assert!(fx.run().orch.handled.is_empty());
    let (count, _) = fx.op("CountCommits");
    let effects = fx.done(
        count,
        crate::run::engine::OpResult::Commits {
            count: 2,
            head: HEAD.into(),
        },
    );
    assert!(replies(&effects)[0].is_ok(), "{effects:#?}");
    assert_eq!(fx.run().orch.handled[0].target, "t1");
}

#[test]
fn the_orchestrator_approves_a_hold_without_noting_itself() {
    let mut fx = super::gate_holds::held(false);
    super::gate_holds::awaiting(&mut fx);
    super::wake_notes::clear(&mut fx);
    let effects = op(
        &mut fx,
        json!({"op": "approve_hold", "hold": "epic:mail", "reason": "the epic is in the goal"}),
    );
    assert!(
        replies(&effects)[0]
            .clone()
            .unwrap()
            .starts_with("hold epic:mail of run ")
    );
    let hold = fx
        .run()
        .orch
        .gate_holds
        .iter()
        .find(|h| h.id == "epic:mail")
        .unwrap();
    assert_eq!(
        (hold.state, hold.decided_by.as_deref()),
        (HoldState::Approved, Some("orchestrator"))
    );
    assert!(logged(
        &fx,
        "orchestrator: approved hold epic:mail — the epic is in the goal"
    ));
    assert!(
        !notes(&fx).iter().any(|n| n.contains("approved hold")),
        "{:?}",
        notes(&fx)
    );
    // The user's own approval still notes and records `user` (pinning).
    assert_eq!(fx.run().state, RunState::Running);
}

#[test]
fn a_full_handled_list_keeps_the_newest_fifty_and_counts_all() {
    let mut fx = approved();
    for n in 0..55 {
        crate::run::orch::handled::record(
            fx.run_mut(),
            n,
            ("retry", "retried", "t1"),
            &format!("r{n}"),
        );
    }
    let run = fx.run();
    assert_eq!(
        run.orch.handled.len(),
        crate::run::orch::handled::HANDLED_KEPT
    );
    assert_eq!(run.orch.handled_total, 55);
    assert_eq!(run.orch.handled[0].reason, "r5");
}
