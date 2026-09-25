//! M8a.13: the lifecycle of review rounds — results no longer awaited, a reviewer's
//! failed delivery and failed resume, and the verdict-less count.

use proto::{AgentRole, BlockReason, TaskState};
use serde_json::json;

use super::dispatch::replies;
use super::fixture::*;
use super::gates::{CHECK_MODE, accepted, check_result, only_op, working_on};
use super::gates_review::{blocking, in_review, reviewed, reviewer, submit, verdict};
use super::turns_fixes::assert_alive;
use crate::run::contract::REVIEW_NUDGE;
use crate::run::engine::{AgentSignal, Effect, EventKind, OpResult};

fn deliveries(effects: &[Effect]) -> Vec<(u32, String)> {
    effects
        .iter()
        .filter_map(|e| match e {
            Effect::Deliver {
                window_id, text, ..
            } => Some((*window_id, text.clone())),
            _ => None,
        })
        .collect()
}

fn override_t1(fx: &mut Fixture) -> Vec<Effect> {
    let reply = fx.reply();
    fx.next(EventKind::Override {
        reply,
        run_id: RUN_ID.into(),
        task_id: "t1".into(),
        reason: "r".into(),
    })
}

/// A `PrepareReview` still in flight when its task left `review` is not awaited when
/// the task comes back: its late result starts no reviewer, and the new round's does.
#[test]
fn a_stale_prepare_review_is_dropped_after_the_task_comes_back() {
    let (mut fx, window) = working_on(PROFILE, CHECK_MODE);
    let (stale, _) = in_review(&mut fx, window);
    override_t1(&mut fx);
    assert_eq!(fx.task("t1").state, TaskState::MergeQueue);
    // Stands in for M8a.14's hand-back: the task is working again.
    fx.run_mut().merge_queue.clear();
    fx.task_mut("t1").merged_without_approval = None;
    fx.force("t1", TaskState::Working);
    fx.turn_completed(window);
    let effects = fx.tool(window, "task_done", json!({"summary": "s"}));
    let (op, _) = ops_in(&effects, "VerifyDone")[0].clone();
    let result = fx.clean_check("t1");
    let effects = fx.done(op, result);
    let (op, _) = only_op(&effects, "Check");
    let effects = fx.done(op, check_result(true));
    // One review worktree at a time: the new round waits for the stale op.
    assert!(ops_in(&effects, "PrepareReview").is_empty(), "{effects:#?}");
    assert_eq!(fx.task("t1").state, TaskState::Review);
    assert_alive(&fx);
    let review = OpResult::Review {
        base: BASE.into(),
        head: HEAD.into(),
        patch: "p".into(),
    };
    let effects = fx.done(stale, review);
    assert!(ops_in(&effects, "CreateWindow").is_empty(), "{effects:#?}");
    let (fresh, _) = only_op(&effects, "PrepareReview");
    assert_ne!(fresh, stale);
    let (_, kind) = reviewer(&mut fx, fresh, "p");
    let crate::run::engine::OpKind::CreateWindow { name, .. } = kind else {
        unreachable!()
    };
    assert_eq!(name, format!("{H4}/t1.r1"));
    assert_alive(&fx);
}

/// A reviewer window that comes back after its task left `review` is stopped.
#[test]
fn a_reviewer_window_for_a_task_no_longer_under_review_is_killed() {
    let (mut fx, window) = working_on(PROFILE, CHECK_MODE);
    let (op, _) = in_review(&mut fx, window);
    let effects = fx.done(
        op,
        OpResult::Review {
            base: BASE.into(),
            head: HEAD.into(),
            patch: "p".into(),
        },
    );
    let (launch, _) = only_op(&effects, "CreateWindow");
    override_t1(&mut fx);
    let effects = fx.done(
        launch,
        OpResult::Window {
            window_id: 77,
            pid: None,
        },
    );
    assert!(
        effects.contains(&Effect::KillWindow { window_id: 77 }),
        "{effects:#?}"
    );
}

/// A verdict resets the count of verdict-less rounds: the next verdict-less round after
/// it starts another round instead of blocking the task.
#[test]
fn a_verdict_resets_the_verdictless_count() {
    let (mut fx, window, rwindow) = reviewed(PROFILE, "");
    fx.turn_completed(window);
    // Round 1 ends without a verdict.
    fx.turn_completed(rwindow);
    fx.turn_completed(rwindow);
    assert_eq!(fx.task("t1").review_misses, 1);
    let (op, _) = fx.op("PrepareReview");
    let (rwindow2, _) = reviewer(&mut fx, op, "p");
    // Round 2 asks for changes: rung 1.
    submit(&mut fx, rwindow2, verdict("changes", blocking()));
    super::turns::exited(&mut fx, rwindow2);
    assert_eq!(fx.task("t1").review_misses, 0);
    assert_eq!(fx.task("t1").state, TaskState::Working);
    fx.turn_completed(window);
    let (op, _) = in_review(&mut fx, window);
    let (rwindow3, _) = reviewer(&mut fx, op, "p");
    // Round 3 ends without a verdict: one in a row, so round 4.
    fx.turn_completed(rwindow3);
    let effects = fx.turn_completed(rwindow3);
    assert_eq!(fx.task("t1").state, TaskState::Review);
    only_op(&effects, "PrepareReview");
    assert_alive(&fx);
}

/// A failed delivery of the nudge is retried later, like a worker's (decision 29).
#[test]
fn a_reviewer_nudge_that_fails_to_deliver_is_retried() {
    let (mut fx, _, rwindow) = reviewed(PROFILE, "");
    let effects = fx.turn_completed(rwindow);
    assert_eq!(
        deliveries(&effects),
        vec![(rwindow, REVIEW_NUDGE.to_string())]
    );
    let ids: Vec<u64> = fx.run().outbox.iter().map(|m| m.id).collect();
    fx.next(EventKind::Delivered {
        run_id: RUN_ID.into(),
        message_ids: ids,
        ok: false,
        error: Some("pipe".into()),
    });
    let round = fx.task("t1").rounds.last().unwrap().clone();
    assert_eq!((round.turn_open, round.delivery_failures), (false, 1));
    assert!(
        deliveries(&fx.tick()).is_empty(),
        "not before the retry delay"
    );
    assert_alive(&fx);
    let effects = fx.send(fx.now + 5, EventKind::Tick);
    assert_eq!(
        deliveries(&effects),
        vec![(rwindow, REVIEW_NUDGE.to_string())]
    );
}

/// A reviewer's failed resume ends its round without a verdict.
#[test]
fn a_reviewer_whose_resume_fails_is_replaced() {
    let (mut fx, _, rwindow) = reviewed(PROFILE, "");
    fx.signal(
        rwindow,
        AgentSignal::Init {
            session_id: "r-1".into(),
        },
    );
    let effects = super::turns::exited(&mut fx, rwindow);
    let (op, _) = only_op(&effects, "ResumeSession");
    let effects = fx.done(
        op,
        OpResult::ResumeFailed {
            error: "no such session".into(),
        },
    );
    only_op(&effects, "PrepareReview");
    let t1 = fx.task("t1");
    assert_eq!((t1.review_misses, t1.failures), (1, 0));
    assert_alive(&fx);
}

/// A Claude reviewer whose process ends between turns is resumed by the delivery of its
/// nudge; its window is still the reviewer's and may submit.
#[test]
fn a_claude_reviewer_that_exits_between_turns_is_resumed_with_its_nudge() {
    let (mut fx, _, rwindow) = reviewed(PROFILE, "[task.route]\nruntime = \"codex\"\nmodel = \"\"");
    assert_eq!(
        fx.task("t1").rounds.last().unwrap().route.runtime,
        proto::Runtime::Claude
    );
    fx.turn_completed(rwindow);
    let ids: Vec<u64> = fx.run().outbox.iter().map(|m| m.id).collect();
    fx.next(EventKind::Delivered {
        run_id: RUN_ID.into(),
        message_ids: ids,
        ok: false,
        error: None,
    });
    // Between turns, the process exits.
    super::turns::exited(&mut fx, rwindow);
    assert!(fx.task("t1").rounds.last().unwrap().ended);
    let effects = fx.send(fx.now + 5, EventKind::Tick);
    let (op, kind) = only_op(&effects, "ResumeSession");
    let crate::run::engine::OpKind::ResumeSession { message, .. } = kind else {
        unreachable!()
    };
    assert_eq!(message, REVIEW_NUDGE);
    fx.done(op, OpResult::Resumed);
    assert!(fx.run().outbox.is_empty(), "{:#?}", fx.run().outbox);
    let effects = submit(&mut fx, rwindow, verdict("approve", vec![]));
    assert!(matches!(&replies(&effects)[..], [Ok(_)]), "{effects:#?}");
    assert_eq!(fx.task("t1").state, TaskState::MergeQueue);
}

/// Every gate failure counts toward the same ladder: a check failure then a review
/// rejection is the second failure, rung 2 (spec §11.4's last paragraph).
#[test]
fn failures_of_different_gates_share_the_ladder() {
    let (mut fx, window) = working_on(PROFILE, CHECK_MODE);
    let effects = accepted(&mut fx, window, json!({"summary": "s"}));
    let (op, _) = only_op(&effects, "Check");
    fx.done(op, check_result(false));
    fx.turn_completed(window);
    let (op, _) = in_review(&mut fx, window);
    let (rwindow, _) = reviewer(&mut fx, op, "p");
    let effects = submit(&mut fx, rwindow, verdict("changes", blocking()));
    assert!(effects.contains(&Effect::KillWindow { window_id: window }));
    let t1 = fx.task("t1");
    assert_eq!(
        (t1.rung, t1.failures, t1.bounces.check, t1.bounces.review),
        (2, 2, 1, 1)
    );
    assert!(t1.fresh_session.is_some());
    assert_ne!(
        t1.block.as_ref().map(|b| b.reason),
        Some(BlockReason::MisSized)
    );
    let _ = AgentRole::Worker;
}

/// A round given up (its second verdict-less turn) is no longer the reviewer, even
/// before its process is gone and the next round has started.
#[test]
fn a_given_up_reviewer_cannot_submit() {
    let (mut fx, _, rwindow) = reviewed(PROFILE, "");
    fx.turn_completed(rwindow);
    fx.turn_completed(rwindow);
    // A Codex reviewer between turns has no process: its round ends at once (T13-I2).
    let last = fx.task("t1").rounds.last().unwrap().clone();
    assert!(last.retiring && last.ended, "{last:#?}");
    let effects = submit(&mut fx, rwindow, verdict("approve", vec![]));
    assert_eq!(
        replies(&effects),
        vec![Err("this window is not the reviewer of task t1".to_string())]
    );
    assert_eq!(fx.task("t1").state, TaskState::Review);
}

/// A verdict drops the reviewer's mail, the nudge in flight included: its failed
/// delivery changes nothing and leaves nothing behind.
#[test]
fn a_verdict_drops_the_reviewers_mail() {
    let (mut fx, _, rwindow) = reviewed(PROFILE, "");
    fx.turn_completed(rwindow);
    let ids: Vec<u64> = fx.run().outbox.iter().map(|m| m.id).collect();
    assert_eq!(ids.len(), 1);
    submit(&mut fx, rwindow, verdict("approve", vec![]));
    fx.next(EventKind::Delivered {
        run_id: RUN_ID.into(),
        message_ids: ids,
        ok: false,
        error: None,
    });
    assert!(fx.run().outbox.is_empty(), "{:#?}", fx.run().outbox);
    assert_eq!(fx.task("t1").state, TaskState::MergeQueue);
}

/// Final review A-I5: the review is prepared against the run head, which the git layer
/// turns into the merge base with the claimed commit, so a run head that moved on (a
/// task merged, a hand-back) never puts another task's work in the reviewer's diff.
#[test]
fn the_review_is_prepared_against_the_run_head() {
    let (mut fx, window) = working_on(PROFILE, CHECK_MODE);
    let moved = "c7".repeat(20);
    fx.run_mut().run_head = moved.clone();
    let (_, kind) = in_review(&mut fx, window);
    let crate::run::engine::OpKind::PrepareReview { base_ref, .. } = kind else {
        unreachable!()
    };
    assert_eq!(base_ref, moved);
}

/// Final review A-6: the reviewer is on the other runtime from the session that wrote
/// the claimed commit. A route amended while that session lived (a `blocked` task's
/// `amend_task { route }`, then an answer that resumes the same session) is the next
/// fresh session's, not the author's.
#[test]
fn the_reviewer_is_picked_against_the_authoring_sessions_route() {
    let (mut fx, window) = working_on(PROFILE, CHECK_MODE);
    let author = fx.task("t1").rounds[0].route.clone();
    assert_eq!(author.runtime, proto::Runtime::Claude);
    fx.task_mut("t1").route.runtime = proto::Runtime::Codex;
    let (op, _) = in_review(&mut fx, window);
    let _ = reviewer(&mut fx, op, "diff --git a/x b/x");
    let t1 = fx.task("t1");
    let level = t1.review_level.unwrap();
    let expected = crate::run::roster::pick_reviewer(&fx.run().roster, &author, level);
    assert_eq!(expected.runtime, proto::Runtime::Codex);
    assert_eq!(t1.review_route, Some(expected));
}
