//! M8a.13 fix round 1: the review's probes as regression tests (rulings T13-I1..I3 and
//! T13-minors), each ending with the liveness check.

use proto::{AgentRole, BlockReason, TaskState};
use serde_json::json;

use super::dispatch::{edit, replies, task_path};
use super::fixture::*;
use super::gates::{CHECK_MODE, accepted, check_result, only_op, working_on};
use super::gates_review::{blocking, reviewed, reviewer, submit, verdict};
use super::holds::{add_dep, delivers};
use super::turns_fixes::assert_alive;
use crate::headless::FailureKind;
use crate::run::contract::{REVIEW_NUDGE, check_failed_message, rate_limit_continue};
use crate::run::engine::{AgentSignal, Effect, EventKind, OpKind, OpResult, TurnOutcome};

const CODEX_AUTHOR: &str = "[task.route]\nruntime = \"codex\"\nmodel = \"\"";

fn failed(kind: FailureKind, error: &str) -> TurnOutcome {
    TurnOutcome::Failed {
        error: error.into(),
        kind,
    }
}

fn to_window(effects: &[Effect]) -> Vec<(u32, String)> {
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

fn ack(fx: &mut Fixture) {
    let ids: Vec<u64> = fx
        .run()
        .outbox
        .iter()
        .filter(|m| m.delivered_at.is_some())
        .map(|m| m.id)
        .collect();
    fx.next(EventKind::Delivered {
        run_id: RUN_ID.into(),
        message_ids: ids,
        ok: true,
        error: None,
    });
}

/// Probe p1 inverted (ruling T13-I1): a rate-limited reviewer turn is not a turn
/// without a verdict. It is counted, the reviewer waits `rate_limit_retry_secs`, and a
/// continue goes to it; the round and the task go on.
#[test]
fn a_rate_limited_reviewer_waits_and_continues() {
    let (mut fx, _, rwindow) = reviewed(PROFILE, "");
    let reviewer_runtime = fx.task("t1").rounds.last().unwrap().route.runtime;
    let label = serde_json::to_value(reviewer_runtime).unwrap();
    for n in 1..=3u32 {
        let effects = fx.turn_ended(
            rwindow,
            failed(FailureKind::RateLimit, "rate limit reached"),
        );
        assert!(delivers(&effects).is_empty(), "no nudge: {effects:#?}");
        let t1 = fx.task("t1");
        assert_eq!(t1.state, TaskState::Review);
        assert_eq!((t1.review_misses, t1.failures), (0, 0));
        assert!(!t1.rounds.last().unwrap().review_nudged);
        assert_eq!(fx.run().rate_limits.get(label.as_str().unwrap()), Some(&n));
        assert_alive(&fx);
        let failed_at = fx.now;
        assert!(delivers(&fx.send(failed_at + 299, EventKind::Tick)).is_empty());
        let effects = fx.send(failed_at + 300, EventKind::Tick);
        assert_eq!(
            to_window(&effects),
            vec![(rwindow, rate_limit_continue("rate limit reached"))]
        );
        ack(&mut fx);
        assert_alive(&fx);
    }
    // A completed turn without a verdict is still nudged, once.
    let effects = fx.turn_completed(rwindow);
    assert_eq!(
        to_window(&effects),
        vec![(rwindow, REVIEW_NUDGE.to_string())]
    );
    assert_alive(&fx);
}

/// Ruling T13-I1: an authentication or billing failure blocks at once with its own text,
/// and the reviewer is stopped.
#[test]
fn a_reviewer_auth_or_billing_failure_blocks_at_once() {
    for kind in [FailureKind::Authentication, FailureKind::Billing] {
        let (mut fx, _, rwindow) = reviewed(PROFILE, "");
        fx.turn_ended(rwindow, failed(kind, "please log in"));
        // The reviewer is stopped: a Codex one between turns has no process to kill.
        let last = fx.task("t1").rounds.last().unwrap().clone();
        assert!(last.retiring && last.ended, "{last:#?}");
        let t1 = fx.task("t1");
        assert_eq!(t1.state, TaskState::Blocked);
        let block = t1.block.clone().unwrap();
        assert_eq!(
            (block.reason, block.text.as_str()),
            (BlockReason::Environment, "please log in")
        );
        assert_eq!(t1.review_misses, 0);
    }
}

/// Ruling T13-I1: another failure gets one continue after the wait; a second one in a
/// row blocks the task on its environment.
#[test]
fn two_other_reviewer_failures_in_a_row_block() {
    let (mut fx, _, rwindow) = reviewed(PROFILE, "");
    fx.turn_ended(rwindow, failed(FailureKind::Other, "overloaded"));
    assert_eq!(fx.task("t1").state, TaskState::Review);
    let effects = fx.send(fx.now + 300, EventKind::Tick);
    assert_eq!(
        to_window(&effects),
        vec![(rwindow, rate_limit_continue("overloaded"))]
    );
    ack(&mut fx);
    fx.turn_ended(rwindow, failed(FailureKind::Other, "overloaded again"));
    let t1 = fx.task("t1");
    assert_eq!(t1.state, TaskState::Blocked);
    let block = t1.block.clone().unwrap();
    assert_eq!(
        (block.reason, block.text.as_str()),
        (BlockReason::Environment, "overloaded again")
    );
}

/// Probe p3 inverted (ruling T13-I2): a reviewer holds its reader slot until its
/// process has exited, so the next round waits for that exit.
#[test]
fn the_next_review_round_waits_for_the_last_reviewers_exit() {
    // A Claude reviewer (the author is on Codex).
    let (mut fx, window, rwindow) = reviewed(PROFILE, CODEX_AUTHOR);
    fx.turn_completed(rwindow);
    ack(&mut fx);
    let effects = fx.turn_completed(rwindow);
    assert!(effects.contains(&Effect::KillWindow { window_id: rwindow }));
    assert!(ops_in(&effects, "PrepareReview").is_empty(), "{effects:#?}");
    assert!(ops_in(&fx.tick(), "PrepareReview").is_empty());
    assert_alive(&fx);
    let effects = super::turns::killed_exit(&mut fx, rwindow);
    let (op, _) = only_op(&effects, "PrepareReview");
    let (rwindow2, _) = reviewer(&mut fx, op, "p");

    // A retired reviewer too: after round 2's rejection and the fix, round 3 waits.
    submit(&mut fx, rwindow2, verdict("changes", blocking()));
    fx.turn_completed(window);
    let effects = accepted(&mut fx, window, json!({"summary": "s"}));
    let (op, _) = only_op(&effects, "Check");
    let effects = fx.done(op, check_result(true));
    assert_eq!(fx.task("t1").state, TaskState::Review);
    assert!(ops_in(&effects, "PrepareReview").is_empty(), "{effects:#?}");
    assert_alive(&fx);
    // The retired Claude reviewer exits on its EOF.
    let effects = super::turns::exited(&mut fx, rwindow2);
    only_op(&effects, "PrepareReview");
    let live: Vec<_> = fx
        .task("t1")
        .rounds
        .iter()
        .filter(|r| r.role == AgentRole::Reviewer && !r.ended)
        .collect();
    assert!(live.is_empty(), "{live:#?}");
}

/// Ruling T13-I2 for Codex: a retired Codex reviewer's normal end of turn is its exit.
#[test]
fn a_retired_codex_reviewer_ends_with_its_process() {
    let (mut fx, _, rwindow) = reviewed(PROFILE, "");
    assert_eq!(
        fx.task("t1").rounds.last().unwrap().route.runtime,
        proto::Runtime::Codex
    );
    submit(&mut fx, rwindow, verdict("approve", vec![]));
    fx.turn_completed(rwindow);
    super::turns::exited(&mut fx, rwindow);
    assert!(fx.task("t1").rounds.last().unwrap().ended);
}

/// Probe p8 (ruling T13-I3): a message queued before the claim is held while the task is
/// in a gate, and goes out with the gate's rung-1 message once it is working again.
#[test]
fn worker_mail_is_held_while_the_task_is_in_a_gate() {
    let (mut fx, window) = working_on(PROFILE, CHECK_MODE);
    for _ in 0..40 {
        fx.signal(
            window,
            AgentSignal::ToolUse {
                name: "Bash".into(),
            },
        );
    }
    assert_eq!(fx.run().outbox.len(), 1, "the soft-budget wrap-up");
    let wrap_up = fx.run().outbox[0].text.clone();
    let effects = fx.tool(window, "task_done", json!({"summary": "s"}));
    let (op, _) = ops_in(&effects, "VerifyDone")[0].clone();
    let result = fx.clean_check("t1");
    let effects = fx.done(op, result);
    let (op, _) = only_op(&effects, "Check");
    let effects = fx.turn_completed(window);
    assert!(delivers(&effects).is_empty(), "{effects:#?}");
    assert!(delivers(&fx.tick()).is_empty());
    assert_alive(&fx);
    let effects = fx.done(op, check_result(false));
    let record = fx.task("t1").checks[0].clone();
    assert_eq!(
        delivers(&effects),
        vec![format!(
            "{wrap_up}\n\n{}",
            check_failed_message("cargo test", &record)
        )]
    );
    assert_alive(&fx);
}

/// Ruling T13-I3: the check and the review run on the claimed commit, never the branch
/// tip: the check in the task's scratch worktree materialized at it, the review worktree
/// at it.
#[test]
fn the_gates_run_on_the_claimed_commit() {
    let (mut fx, window) = working_on(PROFILE, CHECK_MODE);
    let effects = accepted(&mut fx, window, json!({"summary": "s"}));
    let (op, kind) = only_op(&effects, "Check");
    let scratch = task_path("t1.proof");
    assert_eq!(
        kind,
        OpKind::Check {
            dir: scratch.clone(),
            command: "cargo test".into(),
            timeout_secs: 1800,
            env: vec![("TARGET".into(), format!("{}/target", scratch.display()))],
            scratch: Some(crate::run::engine::ScratchAt {
                root: "/tmp/x".into(),
                commit: HEAD.into(),
                setup: Some("make deps".into()),
            }),
        }
    );
    let effects = fx.done(op, check_result(true));
    let (_, kind) = only_op(&effects, "PrepareReview");
    let OpKind::PrepareReview { head_ref, .. } = kind else {
        unreachable!()
    };
    assert_eq!(head_ref, HEAD);
}

/// Minor m1: a tdd task whose profile has no `test_passed` is told at plan time that the
/// proof will look for the test's name.
#[test]
fn a_tdd_task_without_test_passed_gets_a_plan_note() {
    let (fx, _) = working_on(PROFILE, "");
    let note = crate::run::validate::NO_TEST_PASSED_NOTE;
    assert!(
        fx.task("t1").notes.iter().any(|n| n == note),
        "{:?}",
        fx.task("t1").notes
    );
    let with = PROFILE.replace("setup = ", "test_passed = 'ok {test}'\nsetup = ");
    let (fx, _) = working_on(&with, "");
    assert!(!fx.task("t1").notes.iter().any(|n| n == note));
    let (fx, _) = working_on(PROFILE, CHECK_MODE);
    assert!(!fx.task("t1").notes.iter().any(|n| n == note));
}

/// Minor m2: override refuses a held task, even one with a head from an accepted claim.
#[test]
fn override_refuses_a_held_task() {
    let plan = plan_with(
        PROFILE,
        &[
            task_toml("t1", "S", "[\"crates/a/**\"]", CHECK_MODE),
            task_toml("t2", "S", "[\"crates/a/src/**\"]", ""),
        ],
    );
    let mut fx = Fixture::new(&plan);
    fx.ready(true);
    let window = fx.launch_all()[0].1;
    let effects = accepted(&mut fx, window, json!({"summary": "s"}));
    let (op, _) = only_op(&effects, "Check");
    fx.done(op, check_result(false));
    fx.turn_completed(window);
    fx.tool(window, "task_blocked", json!({"reason": "which table?"}));
    edit(&mut fx, vec![add_dep("t1", "t2")]);
    assert!(fx.task("t1").awaiting_deps && fx.task("t1").head.is_some());
    let reply = fx.reply();
    let effects = fx.next(EventKind::Override {
        reply,
        run_id: RUN_ID.into(),
        task_id: "t1".into(),
        reason: "r".into(),
    });
    assert_eq!(
        replies(&effects),
        vec![Err(
            "task t1 waits for its dependencies; override it once they are merged".to_string()
        )]
    );
    assert_eq!(fx.task("t1").state, TaskState::Blocked);
    assert!(fx.task("t1").merged_without_approval.is_none());
}

/// Minor m3: rung 3 re-resolves the review level, reviewer and budget for the raised
/// size, so a retried task is reviewed and budgeted as what it now is.
#[test]
fn rung_3_re_resolves_review_and_budget_for_the_raised_size() {
    let config = config::Orchestrator {
        review_small: false,
        ..Default::default()
    };
    let (mut fx, window) = super::gates::working_with(PROFILE, "", config);
    assert_eq!(fx.task("t1").review_level, None, "S at small, skipped");
    let args = json!({"kind": "mis_sized", "reason": "bigger"});
    fx.tool(window, "task_blocked", args);
    let t1 = fx.task("t1");
    assert_eq!((t1.state, t1.size), (TaskState::Blocked, proto::Size::M));
    assert_eq!(
        t1.review_level,
        Some(crate::run::model::ReviewLevel::Medium)
    );
    let route = crate::run::roster::pick_reviewer(
        &fx.run().roster,
        &t1.route,
        crate::run::model::ReviewLevel::Medium,
    );
    assert_eq!(t1.review_route, Some(route));
    assert_eq!(t1.budget, fx.run().limits.budget_m);
}

/// Minor m4: the verdict-less count is reset when the task leaves `review`.
#[test]
fn the_verdictless_count_resets_when_the_task_leaves_review() {
    let (mut fx, _, rwindow) = reviewed(PROFILE, "");
    fx.turn_completed(rwindow);
    ack(&mut fx);
    fx.turn_completed(rwindow);
    assert_eq!(fx.task("t1").review_misses, 1);
    let (op, _) = fx.op("PrepareReview");
    fx.done(
        op,
        OpResult::Failed {
            message: "disk full".into(),
        },
    );
    let t1 = fx.task("t1");
    assert_eq!((t1.state, t1.review_misses), (TaskState::Blocked, 0));
}

/// Minor m6: a task in `review` waiting while a hub task holds a writer slot is alive.
#[test]
fn a_review_waiting_for_a_hub_is_alive() {
    let plan = plan_with(
        PROFILE,
        &[
            task_toml("t1", "S", "[\"crates/a/**\"]", CHECK_MODE),
            task_toml("t2", "M", "[\"crates/proto/**\"]", "deps = [\"t1\"]"),
        ],
    );
    let mut fx = Fixture::new(&plan);
    fx.ready(true);
    let window = fx.launch_all()[0].1;
    let effects = accepted(&mut fx, window, json!({"summary": "s"}));
    let (op, _) = only_op(&effects, "Check");
    // Stands in for a hub task holding a writer slot (M8a.14 merges t1 first).
    fx.task_mut("t2").state = TaskState::Working;
    assert!(fx.task("t2").hub);
    let effects = fx.done(op, check_result(true));
    assert!(ops_in(&effects, "PrepareReview").is_empty(), "{effects:#?}");
    assert_eq!(fx.task("t1").state, TaskState::Review);
    super::turns_fixes::assert_gates_alive(&fx);
}

/// Ruling T13-I1: a reviewer's rate-limit streak counts once, as a worker's does: a
/// failed turn that ends an `api_retry` streak adds nothing. The round shows when it
/// retries.
#[test]
fn a_reviewers_rate_limit_streak_counts_once() {
    let (mut fx, _, rwindow) = reviewed(PROFILE, "");
    let reviewer_runtime = fx.task("t1").rounds.last().unwrap().route.runtime;
    let label = serde_json::to_value(reviewer_runtime).unwrap();
    let label = label.as_str().unwrap();
    fx.signal(
        rwindow,
        AgentSignal::ApiRetry {
            error: "rate_limit".into(),
            delay_ms: 1000,
        },
    );
    assert_eq!(fx.run().rate_limits.get(label), Some(&1));
    fx.turn_ended(
        rwindow,
        failed(FailureKind::RateLimit, "rate limit reached"),
    );
    assert_eq!(fx.run().rate_limits.get(label), Some(&1));
    let round = fx.task("t1").rounds.last().unwrap().clone();
    assert_eq!(round.rate_limited_until, Some(fx.now + 300));
    assert_alive(&fx);
}

/// Ruling T13-I1: "two in a row" means two failed turns with no completed turn between
/// them; a completed turn after the continue starts the count again.
#[test]
fn a_completed_reviewer_turn_resets_the_other_failure_count() {
    let (mut fx, _, rwindow) = reviewed(PROFILE, "");
    fx.turn_ended(rwindow, failed(FailureKind::Other, "overloaded"));
    fx.send(fx.now + 300, EventKind::Tick);
    ack(&mut fx);
    let effects = fx.turn_completed(rwindow);
    assert_eq!(
        to_window(&effects),
        vec![(rwindow, REVIEW_NUDGE.to_string())]
    );
    ack(&mut fx);
    fx.turn_ended(rwindow, failed(FailureKind::Other, "overloaded again"));
    assert_eq!(fx.task("t1").state, TaskState::Review);
    assert_alive(&fx);
}

/// Ruling T13-I3: worker mail stays held in the merge queue too.
#[test]
fn worker_mail_is_held_in_the_merge_queue() {
    let (mut fx, window) = working_on(PROFILE, CHECK_MODE);
    for _ in 0..40 {
        fx.signal(
            window,
            AgentSignal::ToolUse {
                name: "Bash".into(),
            },
        );
    }
    assert_eq!(fx.run().outbox.len(), 1, "the soft-budget wrap-up");
    let (op, _) = super::gates_review::in_review(&mut fx, window);
    let (rwindow, _) = reviewer(&mut fx, op, "p");
    submit(&mut fx, rwindow, verdict("approve", vec![]));
    assert_eq!(fx.task("t1").state, TaskState::MergeQueue);
    let effects = fx.turn_completed(window);
    assert!(delivers(&effects).is_empty(), "{effects:#?}");
    assert!(delivers(&fx.tick()).is_empty());
    assert!(fx.run().outbox[0].delivered_at.is_none());
    assert_alive(&fx);
}

/// Ruling T13-I3's scratch check: a setup that fails in the scratch worktree blocks the
/// task on its environment, as the proof's does, instead of re-running the check.
#[test]
fn a_check_whose_setup_fails_blocks_on_the_environment() {
    let (mut fx, window) = working_on(PROFILE, CHECK_MODE);
    let effects = accepted(&mut fx, window, json!({"summary": "s"}));
    let (op, _) = only_op(&effects, "Check");
    let effects = fx.done(
        op,
        OpResult::SetupFailed {
            output: "no make".into(),
        },
    );
    assert!(ops_in(&effects, "Check").is_empty(), "{effects:#?}");
    let t1 = fx.task("t1");
    assert_eq!((t1.state, t1.failures), (TaskState::Blocked, 0));
    let block = t1.block.clone().unwrap();
    assert_eq!(block.reason, BlockReason::Environment);
    assert_eq!(block.text, "setup failed in the check worktree:\nno make");
    assert_alive(&fx);
}
