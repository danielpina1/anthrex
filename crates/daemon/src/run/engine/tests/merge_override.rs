//! M8a.25: a task blocked at rung 3 (its worker session stopped by the engine), then
//! overridden, whose merge candidate conflicts: the hand-back's conflict message is
//! carried by a resume of that stopped session (decision 36 step 6: "decision 29
//! resumes an ended session to carry it"), whether the kill's exit reached the engine
//! before the hand-back or after it. Found by `e2e_conflict_is_handed_back_and_resolved`:
//! the message sat in the outbox and the task stayed `working` with no session.

use proto::TaskState;

use super::control::{blocked, override_task};
use super::dispatch::replies;
use super::fixture::*;
use super::gates::only_op;
use super::liveness::assert_alive;
use super::turns::{killed_exit, working};
use crate::run::contract::conflict_message;
use crate::run::engine::{Effect, OpKind, OpResult};

/// `t1` blocked by `task_blocked mis_sized` (rung 3), overridden, its candidate
/// conflicting, and handed back; the kill's exit arrives first when `exit_first`.
/// Every effect from the hand-back's result on.
fn handed_back_after_rung_3(exit_first: bool) -> (Fixture, Vec<Effect>) {
    let (mut fx, window) = working();
    blocked(&mut fx, window, "mis_sized", "too big");
    assert_eq!(fx.task("t1").rung, 3);
    if exit_first {
        killed_exit(&mut fx, window);
    }
    let effects = override_task(&mut fx, "t1", "shared-ok");
    let (op, _) = only_op(&effects, "CountCommits");
    let effects = fx.done(
        op,
        OpResult::Commits {
            count: 1,
            head: HEAD.into(),
            protected: Vec::new(),
        },
    );
    let (op, _) = only_op(&effects, "MergeCandidate");
    let effects = fx.done(
        op,
        OpResult::Conflict {
            files: vec!["b/shared.txt".into()],
            tree: None,
        },
    );
    let (op, _) = only_op(&effects, "HandBack");
    let mut effects = fx.done(
        op,
        OpResult::HandedBack {
            files: vec!["b/shared.txt".into()],
            head: None,
            onto: Some(HEAD.into()),
            merged: Vec::new(),
            merged_total: 0,
        },
    );
    assert_eq!(fx.task("t1").state, TaskState::Working);
    if !exit_first {
        effects.extend(killed_exit(&mut fx, window));
    }
    effects.extend(fx.tick());
    (fx, effects)
}

fn assert_resumed_with_the_conflict(fx: &Fixture, effects: &[Effect]) {
    let session = fx.task("t1").rounds[0]
        .session_id
        .clone()
        .expect("session 1's id");
    let resumes = ops_in(effects, "ResumeSession");
    assert_eq!(resumes.len(), 1, "{effects:#?}");
    let OpKind::ResumeSession {
        session_id,
        message,
        ..
    } = &resumes[0].1
    else {
        unreachable!()
    };
    assert_eq!(*session_id, session);
    assert_eq!(*message, conflict_message(&["b/shared.txt".to_string()]));
    let round = &fx.task("t1").rounds[0];
    assert!(!round.ended && !round.retiring, "{round:#?}");
    assert_alive(fx);
}

#[test]
fn a_hand_back_resumes_the_session_rung_3_stopped() {
    let (fx, effects) = handed_back_after_rung_3(true);
    assert_resumed_with_the_conflict(&fx, &effects);
}

#[test]
fn a_hand_back_before_the_kills_exit_resumes_the_session_once_it_ends() {
    let (fx, effects) = handed_back_after_rung_3(false);
    assert_resumed_with_the_conflict(&fx, &effects);
}

/// [`super::wake_notes::approved`] with `t1` working, then blocked, its start commit
/// recorded: an override counts its commits.
fn blocked_with_commits() -> Fixture {
    let mut fx = super::wake_notes::approved();
    let windows = fx.launch_all();
    let (_, window) = windows.iter().find(|(t, _)| t == "t1").cloned().unwrap();
    blocked(&mut fx, window, "environment", "locked");
    fx.task_mut("t1").start_commit = Some(BASE.into());
    fx
}

fn orch_override(fx: &mut Fixture) -> Vec<Effect> {
    let edit = serde_json::json!({"op": "override", "task_id": "t1", "reason": "work is done"});
    super::orch::edit_plan(fx, serde_json::json!({"edits": [edit]}))
}

/// The count of `t1`'s one commit, which changes `AGENTS.md`.
fn counted_agents_md(fx: &mut Fixture) -> Vec<Effect> {
    let (count, _) = fx.op("CountCommits");
    fx.done(
        count,
        OpResult::Commits {
            count: 1,
            head: HEAD.into(),
            protected: vec!["AGENTS.md".into()],
        },
    )
}

/// Milestone 9.13 decision 5: the orchestrator's override counts the protected paths;
/// one listed refuses it, and the block becomes the user's.
#[test]
fn orchestrator_override_of_a_protected_change_is_refused_and_user_only() {
    let mut fx = blocked_with_commits();
    let effects = orch_override(&mut fx);
    assert!(
        replies(&effects).is_empty(),
        "the reply waits for the count"
    );
    let (_, kind) = fx.op("CountCommits");
    let OpKind::CountCommits {
        owns, protected, ..
    } = kind
    else {
        unreachable!()
    };
    assert_eq!(owns, fx.task("t1").spec.owns);
    assert!(!protected.is_empty());
    assert_eq!(protected, fx.run().profile.protected);
    let effects = counted_agents_md(&mut fx);
    let refusal = "task t1 changes protected AGENTS.md; only the user can merge it unchecked";
    assert_eq!(replies(&effects), vec![Err(refusal.to_string())]);
    let t1 = fx.task("t1");
    assert_eq!(t1.state, TaskState::Blocked);
    assert!(t1.block.as_ref().is_some_and(|b| b.user_only), "{t1:#?}");
    assert!(t1.merged_without_approval.is_none());
    assert!(fx.run().orch.handled.is_empty());
    let record = fx.run().plan_edits.last().unwrap();
    assert_eq!(
        (
            record.source.as_str(),
            record.accepted,
            record.error.as_deref()
        ),
        ("orchestrator", false, Some(refusal))
    );
}

/// Decision 5: the user's override of the same change goes through and says so.
#[test]
fn user_override_lists_protected_paths() {
    let mut fx = blocked_with_commits();
    override_task(&mut fx, "t1", "the user read it");
    let effects = counted_agents_md(&mut fx);
    let text = replies(&effects)[0].clone().unwrap();
    assert_eq!(
        text,
        "task t1 goes to the merge queue without review: the user read it; it changes protected AGENTS.md"
    );
    assert!(fx.task("t1").merged_without_approval.is_some());
    assert_eq!(ops_in(&effects, "MergeCandidate").len(), 1, "{effects:#?}");
    assert!(
        fx.run()
            .log
            .iter()
            .any(|e| e.text == "the user overrode t1, which changes protected AGENTS.md"),
        "{:#?}",
        fx.run().log
    );
}

/// Pinning (decision 5): a task in `review` already passed decision 56 at `done`; the
/// orchestrator's override sends it to the queue with no count.
#[test]
fn an_override_from_review_is_not_counted() {
    let mut fx = super::wake_notes::approved();
    fx.launch_all();
    fx.task_mut("t1").head = Some(HEAD.into());
    fx.force("t1", TaskState::Review);
    let effects = orch_override(&mut fx);
    assert!(ops_in(&effects, "CountCommits").is_empty(), "{effects:#?}");
    assert!(replies(&effects)[0].is_ok(), "{effects:#?}");
    assert_eq!(fx.task("t1").state, TaskState::MergeQueue);
}

/// Task 4 review M6 (pinning): a `CountCommits` op journalled before milestone 9.13 has
/// no `owns` or `protected`, and a `Commits` result no `protected`; both load empty.
#[test]
fn an_old_count_and_its_result_load_with_empty_lists() {
    let kind = OpKind::CountCommits {
        worktree: "/w".into(),
        start: "a".repeat(40),
        run_head: "b".repeat(40),
        not_own: vec!["c".repeat(40)],
        not_run: Vec::new(),
        owns: Vec::new(),
        protected: Vec::new(),
    };
    let mut old = serde_json::to_value(&kind).unwrap();
    let fields = old["CountCommits"].as_object_mut().unwrap();
    assert!(fields.remove("owns").is_some() && fields.remove("protected").is_some());
    assert_eq!(serde_json::from_value::<OpKind>(old).unwrap(), kind);

    let result = OpResult::Commits {
        count: 2,
        head: "d".repeat(40),
        protected: Vec::new(),
    };
    let mut old = serde_json::to_value(&result).unwrap();
    let fields = old["Commits"].as_object_mut().unwrap();
    assert!(fields.remove("protected").is_some());
    assert_eq!(serde_json::from_value::<OpResult>(old).unwrap(), result);
}
