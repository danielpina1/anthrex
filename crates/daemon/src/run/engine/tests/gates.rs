//! M8a.13: the test proof (decision 33), the check (decision 34) and their failures on
//! decision 38's ladder. The review gate (decision 35) is in `gates_review.rs`. Every
//! sequence ends with the liveness check, extended to the gate states.

use proto::{BlockReason, TaskState};
use serde_json::json;

use super::dispatch::{replies, task_path};
use super::fixture::*;
use super::holds::delivers;
use super::turns_fixes::assert_alive;
use crate::run::contract::{
    DONE_ACCEPTED, DONE_NUDGE, check_failed_message, handover_prompt, proof_failed_message,
};
use crate::run::engine::{AgentSignal, Effect, OpKind, OpResult};
use crate::run::model::{CheckRecord, OpId, ProofRecord};
use crate::run::proof::{proof_command, proof_pattern};
use crate::run::roster::escalate;

/// `PROFILE` without its `check` line.
pub(super) fn no_check() -> String {
    PROFILE.replace("check = \"cargo test\"\n", "")
}

/// `PROFILE` with a `test_passed` pattern.
fn with_passed() -> String {
    PROFILE.replace(
        "setup = ",
        "test_passed = 'test {test} \\.\\.\\. ok'\nsetup = ",
    )
}

/// A check-mode task's extra lines.
pub(super) const CHECK_MODE: &str = "test_mode = \"check\"\ntest_mode_reason = \"glue code\"";

/// A working `t1` (S, owning `crates/a/**`) on `profile` with `extra`; its window.
pub(super) fn working_on(profile: &str, extra: &str) -> (Fixture, u32) {
    working_with(profile, extra, config::Orchestrator::default())
}

pub(super) fn working_with(
    profile: &str,
    extra: &str,
    config: config::Orchestrator,
) -> (Fixture, u32) {
    let plan = plan_with(profile, &[task("t1", "S", "a", extra)]);
    let mut fx = Fixture::with_config(&plan, config);
    fx.ready(true);
    let window = fx.launch_all()[0].1;
    assert_eq!(fx.task("t1").state, TaskState::Working);
    (fx, window)
}

pub(super) fn tdd_args() -> serde_json::Value {
    json!({"summary": "did it", "test": "a::works", "red": "abcdef1"})
}

/// `task_done` from `window` with `args`, accepted by a clean `VerifyDone`, then the
/// worker ends its turn as it was told. Returns the effects of the accepting step.
pub(super) fn accepted(fx: &mut Fixture, window: u32, args: serde_json::Value) -> Vec<Effect> {
    let effects = fx.tool(window, "task_done", args);
    let (op, _) = ops_in(&effects, "VerifyDone")[0].clone();
    let result = fx.clean_check("t1");
    let effects = fx.done(op, result);
    assert_eq!(replies(&effects), vec![Ok(DONE_ACCEPTED.to_string())]);
    fx.turn_completed(window);
    effects
}

/// The only op of `name` among `effects`.
pub(super) fn only_op(effects: &[Effect], name: &str) -> (OpId, OpKind) {
    let ops = ops_in(effects, name);
    assert_eq!(ops.len(), 1, "one {name}: {effects:#?}");
    ops[0].clone()
}

fn proof_result(red_failed: bool, head_passed: bool, matched: bool) -> OpResult {
    OpResult::Proof {
        red_failed,
        head_passed,
        matched,
        red_tail: "red line 1\nred line 2".into(),
        head_tail: "head line 1\nhead line 2".into(),
    }
}

pub(super) fn check_result(ok: bool) -> OpResult {
    OpResult::Check {
        ok,
        code: Some(if ok { 0 } else { 101 }),
        timed_out: false,
        tail: "compiling\ntest a::works ... FAILED".into(),
        secs: 42,
    }
}

/// A red run that passed, a head run that failed, or output that did not show the test.
#[test]
fn proof_passes_to_check() {
    let (mut fx, window) = working_on(&with_passed(), "");
    let effects = accepted(&mut fx, window, tdd_args());
    assert_eq!(fx.task("t1").state, TaskState::Proof);
    let (op, kind) = only_op(&effects, "Proof");
    let proof_path = task_path("t1.proof");
    assert_eq!(
        kind,
        OpKind::Proof {
            root: "/tmp/x".into(),
            path: proof_path.clone(),
            red: "abcdef1".into(),
            head: HEAD.into(),
            command: proof_command("cargo test -- --exact {test}", "a::works"),
            passed: proof_pattern("test {test} \\.\\.\\. ok", "a::works"),
            timeout_secs: 1800,
            setup: Some("make deps".into()),
            env: vec![("TARGET".into(), format!("{}/target", proof_path.display()))],
        }
    );
    assert_eq!(fx.task("t1").gate_op, Some(op));
    // No second proof while one is in flight.
    assert!(ops_in(&fx.tick(), "Proof").is_empty());
    assert_alive(&fx);

    let effects = fx.done(op, proof_result(true, true, true));
    let t1 = fx.task("t1");
    assert_eq!(t1.state, TaskState::Check);
    assert_eq!(t1.proofs.len(), 1);
    let record = &t1.proofs[0];
    assert!(record.red_failed && record.head_passed && record.matched);
    assert_eq!(
        (record.test.as_str(), record.red.as_str()),
        ("a::works", "abcdef1")
    );
    assert_eq!(record.head, HEAD);
    let (check_op, kind) = only_op(&effects, "Check");
    assert_eq!(
        kind,
        OpKind::Check {
            dir: proof_path.clone(),
            command: "cargo test".into(),
            timeout_secs: 1800,
            env: vec![("TARGET".into(), format!("{}/target", proof_path.display()))],
            // Ruling T13-I3: on the claimed commit, in the scratch worktree.
            scratch: Some(crate::run::engine::ScratchAt {
                root: "/tmp/x".into(),
                commit: HEAD.into(),
                setup: Some("make deps".into()),
            }),
        }
    );
    assert_eq!(fx.task("t1").gate_op, Some(check_op));
    assert_eq!(fx.task("t1").failures, 0);
    assert_alive(&fx);

    // A proof with no `test_passed` in the profile asks for the test's name.
    let (mut fx, window) = working_on(PROFILE, "");
    let effects = accepted(&mut fx, window, tdd_args());
    let (_, kind) = only_op(&effects, "Proof");
    let OpKind::Proof { passed, .. } = kind else {
        unreachable!()
    };
    assert_eq!(passed, proof_pattern("{test}", "a::works"));
}

/// Each of `proof_failed_message`'s reasons (the fourth is the fallback's, below).
#[test]
fn proof_failure_is_rung_1_with_the_message() {
    let cases = [
        // The engine does not rely on the head run being skipped after a red that
        // passed (M8a.10): a red that did not fail is the reason, whatever the head.
        (
            proof_result(false, true, true),
            "at the red commit abcdef1 the test passed, so it does not fail without your change",
            "red line 2",
        ),
        (
            proof_result(true, false, false),
            "at your head d1d1d1d the test failed",
            "head line 2",
        ),
        (
            proof_result(true, true, false),
            "the output did not show that a::works ran and passed (expected a line matching test a::works \\.\\.\\. ok)",
            "head line 2",
        ),
    ];
    for (result, reason, tail) in cases {
        let (mut fx, window) = working_on(&with_passed(), "");
        let effects = accepted(&mut fx, window, tdd_args());
        let (op, kind) = only_op(&effects, "Proof");
        let OpKind::Proof {
            command, passed, ..
        } = kind
        else {
            unreachable!()
        };
        let effects = fx.done(op, result);
        let t1 = fx.task("t1");
        assert_eq!(t1.state, TaskState::Working, "{reason}");
        assert_eq!((t1.rung, t1.failures, t1.bounces.proof), (1, 1, 1));
        let record = t1.proofs.last().unwrap().clone();
        let text = proof_failed_message(&command, &record, &passed);
        assert!(
            text.starts_with(&format!("[anthrex] The test proof failed: {reason}\n")),
            "{text}"
        );
        assert!(text.contains(&format!("\nCommand: {command}\n")), "{text}");
        assert!(text.contains("Last 40 lines:\n"), "{text}");
        assert!(
            text.contains(tail),
            "{reason}: the offending run's tail: {text}"
        );
        assert!(text.ends_with("\nFix it, commit, then call task_done again."));
        // The red run did not fail: its tail is quoted, not the (empty) head tail.
        if reason.starts_with("at the red") {
            assert!(!text.contains("head line"), "{text}");
        }
        // To the same session, as its next turn.
        assert!(effects.contains(&Effect::Deliver {
            run_id: RUN_ID.into(),
            message_ids: vec![fx.run().next_message - 1],
            window_id: window,
            text: text.clone(),
        }));
        assert_eq!(t1.failure_log, vec![text]);
        assert!(ops_in(&effects, "Check").is_empty());
        assert_alive(&fx);
    }
}

#[test]
fn proof_accepts_a_red_commit_from_an_earlier_session() {
    let (mut fx, window) = working_on(PROFILE, "");
    // Session 1 committed its red, then stalled twice: rung 2.
    fx.task_mut("t1").rounds[0].stall = crate::run::model::StallState::Nudged;
    let quiet = fx.task("t1").rounds[0].last_event;
    let effects = fx.send(quiet + 601, crate::run::engine::EventKind::Tick);
    assert!(effects.contains(&Effect::KillWindow { window_id: window }));
    super::turns::killed_exit(&mut fx, window);
    let (op, _) = fx.op("DiffSoFar");
    fx.done(
        op,
        OpResult::Diff {
            stat: "1 file".into(),
            patch: "red".into(),
        },
    );
    let window2 = fx.complete_windows()[0].1;
    assert_eq!(fx.task("t1").session, 2);
    // Session 2 names session 1's red.
    let args = json!({"summary": "s", "test": "a::works", "red": "0ed0ed1"});
    let effects = accepted(&mut fx, window2, args);
    let (_, kind) = only_op(&effects, "Proof");
    let OpKind::Proof { red, head, .. } = kind else {
        unreachable!()
    };
    assert_eq!((red.as_str(), head.as_str()), ("0ed0ed1", HEAD));
    assert_alive(&fx);
}

#[test]
fn turn_end_fallback_on_a_tdd_task_fails_the_proof_with_the_missing_names_message() {
    let (mut fx, window) = working_on(PROFILE, "");
    fx.turn_completed(window);
    let (op, _) = fx.op("CountCommits");
    let effects = fx.done(
        op,
        OpResult::Commits {
            count: 2,
            head: HEAD.into(),
        },
    );
    assert_eq!(delivers(&effects), vec![DONE_NUDGE.to_string()]);
    let effects = fx.turn_completed(window);
    let (op, _) = only_op(&effects, "VerifyDone");
    let result = fx.clean_check("t1");
    let effects = fx.done(op, result);
    assert!(ops_in(&effects, "Proof").is_empty(), "nothing to prove");
    let t1 = fx.task("t1");
    assert_eq!(t1.state, TaskState::Working);
    assert_eq!((t1.rung, t1.failures, t1.bounces.proof), (1, 1, 1));
    let record = ProofRecord {
        at: fx.now,
        test: String::new(),
        red: String::new(),
        head: HEAD.into(),
        red_failed: false,
        head_passed: false,
        matched: false,
        red_tail: String::new(),
        head_tail: String::new(),
    };
    assert_eq!(t1.proofs, vec![record.clone()]);
    let text = proof_failed_message("cargo test -- --exact {test}", &record, "{test}");
    assert_eq!(
        text,
        "[anthrex] The test proof failed: this is a tdd task and no test or red commit was named; call task_done with test and red\nFix it, commit, then call task_done again."
    );
    assert_eq!(delivers(&effects), vec![text]);
    assert_alive(&fx);
}

/// A check-mode `t1` whose claim was accepted: its `Check` op.
fn in_check(fx: &mut Fixture, window: u32) -> OpId {
    let effects = accepted(fx, window, json!({"summary": "s"}));
    assert_eq!(fx.task("t1").state, TaskState::Check);
    only_op(&effects, "Check").0
}

#[test]
fn check_failure_goes_up_the_ladder() {
    let (mut fx, window) = working_on(PROFILE, CHECK_MODE);
    let route = fx.task("t1").route.clone();

    // First failure: rung 1, the check's message to the same session.
    let op = in_check(&mut fx, window);
    let effects = fx.done(op, check_result(false));
    let t1 = fx.task("t1");
    assert_eq!(
        (t1.state, t1.rung, t1.failures, t1.bounces.check),
        (TaskState::Working, 1, 1, 1)
    );
    let record = CheckRecord {
        at: fx.now,
        ok: false,
        code: Some(101),
        timed_out: false,
        tail: "compiling\ntest a::works ... FAILED".into(),
        secs: 42,
        on_candidate: false,
    };
    assert_eq!(t1.checks, vec![record.clone()]);
    let first = check_failed_message("cargo test", &record);
    assert_eq!(
        first,
        "[anthrex] The check failed (exit 101): cargo test\nLast 40 lines:\ncompiling\ntest a::works ... FAILED\nFix it, commit, then call task_done again."
    );
    let delivered: Vec<(u32, String)> = effects
        .iter()
        .filter_map(|e| match e {
            Effect::Deliver {
                window_id, text, ..
            } => Some((*window_id, text.clone())),
            _ => None,
        })
        .collect();
    assert_eq!(delivered, vec![(window, first.clone())]);
    fx.turn_completed(window);
    assert_alive(&fx);

    // Second failure: rung 2, a fresh session on the escalated route.
    let op = in_check(&mut fx, window);
    let effects = fx.done(op, check_result(false));
    assert!(effects.contains(&Effect::KillWindow { window_id: window }));
    assert_eq!(fx.task("t1").rung, 2);
    assert!(ops_in(&effects, "DiffSoFar").is_empty(), "the kill first");
    assert_alive(&fx);
    let effects = super::turns::killed_exit(&mut fx, window);
    let (op, _) = only_op(&effects, "DiffSoFar");
    let effects = fx.done(
        op,
        OpResult::Diff {
            stat: " crates/a/x.rs | 2 +-".into(),
            patch: "diff so far".into(),
        },
    );
    let (_, kind) = only_op(&effects, "CreateWindow");
    let OpKind::CreateWindow {
        name,
        first_turn,
        spec,
        ..
    } = kind
    else {
        unreachable!()
    };
    assert_eq!(name, format!("{H4}/t1.w2"));
    let escalated = escalate(&fx.run().roster, &route);
    assert_eq!(fx.task("t1").route, escalated);
    assert_eq!(
        (spec.runtime, spec.effort),
        (escalated.runtime, escalated.effort)
    );
    let second = fx.task("t1").failure_log[1].clone();
    let reason = format!(
        "the check gate failed again: {}",
        second.lines().next().unwrap()
    );
    let expected = handover_prompt(
        fx.run(),
        fx.task("t1"),
        &reason,
        " crates/a/x.rs | 2 +-",
        "diff so far",
    );
    assert_eq!(first_turn, expected);
    assert!(first_turn.contains(&first) && first_turn.contains(&second));
    let window2 = fx.complete_windows()[0].1;
    assert_alive(&fx);

    // Third failure: rung 3.
    let op = in_check(&mut fx, window2);
    let effects = fx.done(op, check_result(false));
    assert!(effects.contains(&Effect::KillWindow { window_id: window2 }));
    let t1 = fx.task("t1");
    assert_eq!((t1.state, t1.rung, t1.failures), (TaskState::Blocked, 3, 3));
    assert_eq!(t1.block.as_ref().unwrap().reason, BlockReason::MisSized);
}

#[test]
fn a_timed_out_check_says_so() {
    let record = CheckRecord {
        at: 0,
        ok: false,
        code: None,
        timed_out: true,
        tail: "slow".into(),
        secs: 1800,
        on_candidate: false,
    };
    assert_eq!(
        check_failed_message("make test", &record),
        "[anthrex] The check failed (timed out after 30 minutes): make test\nLast 40 lines:\nslow\nFix it, commit, then call task_done again."
    );
}

#[test]
fn max_bounces_1_blocks_on_the_second_failure_of_a_gate() {
    let (mut fx, window) = working_on(&profile_with("max_bounces = 1"), CHECK_MODE);
    let op = in_check(&mut fx, window);
    fx.done(op, check_result(false));
    assert_eq!(fx.task("t1").rung, 1);
    fx.turn_completed(window);
    let op = in_check(&mut fx, window);
    let effects = fx.done(op, check_result(false));
    let t1 = fx.task("t1");
    assert_eq!(
        (t1.state, t1.rung, t1.bounces.check),
        (TaskState::Blocked, 3, 2)
    );
    assert!(effects.contains(&Effect::KillWindow { window_id: window }));
    assert!(ops_in(&fx.log, "DiffSoFar").is_empty(), "never rung 2");
}

#[test]
fn no_check_skips_the_gate_and_marks_the_run_unverified() {
    let (mut fx, window) = working_on(&no_check(), "");
    assert!(fx.run().unverified);
    let effects = accepted(&mut fx, window, tdd_args());
    let (op, _) = only_op(&effects, "Proof");
    let effects = fx.done(op, proof_result(true, true, true));
    assert!(ops_in(&fx.log, "Check").is_empty());
    let t1 = fx.task("t1");
    assert_eq!(t1.state, TaskState::Review, "raised to medium, reviewed");
    assert_eq!(
        t1.review_level,
        Some(crate::run::model::ReviewLevel::Medium)
    );
    only_op(&effects, "PrepareReview");
    assert!(fx.run().unverified);
    assert_alive(&fx);
}

/// A gate result nobody awaits any more (a stale op id, or one for a task that left the
/// gate) changes nothing.
#[test]
fn a_gate_result_no_longer_awaited_is_dropped() {
    let (mut fx, window) = working_on(PROFILE, CHECK_MODE);
    let op = in_check(&mut fx, window);
    // An op id the task does not await.
    let before = fx.task("t1").clone();
    fx.done(op + 100, check_result(false));
    assert_eq!(fx.task("t1"), &before);
    // The task left `check` (the user cancelled it): its result is dropped.
    let effects = super::dispatch::edit(
        &mut fx,
        vec![proto::PlanEdit::CancelTask {
            task_id: "t1".into(),
        }],
    );
    assert!(replies(&effects)[0].is_ok(), "{effects:#?}");
    fx.done(op, check_result(false));
    let t1 = fx.task("t1");
    assert_eq!(t1.state, TaskState::Cancelled);
    assert!(t1.checks.is_empty() && t1.failures == 0);
}

/// A setup or git failure of the proof, and a check that could not run, block the task
/// on its environment; neither is a gate failure.
#[test]
fn a_gate_that_cannot_run_blocks_on_the_environment() {
    let (mut fx, window) = working_on(PROFILE, "");
    let effects = accepted(&mut fx, window, tdd_args());
    let (op, _) = only_op(&effects, "Proof");
    fx.done(
        op,
        OpResult::SetupFailed {
            output: "no make".into(),
        },
    );
    let t1 = fx.task("t1");
    assert_eq!(t1.state, TaskState::Blocked);
    assert_eq!(t1.failures, 0);
    let block = t1.block.as_ref().unwrap();
    assert_eq!(block.reason, BlockReason::Environment);
    assert!(block.text.contains("no make"), "{}", block.text);

    let (mut fx, window) = working_on(PROFILE, CHECK_MODE);
    let op = in_check(&mut fx, window);
    fx.done(
        op,
        OpResult::Failed {
            message: "cannot spawn".into(),
        },
    );
    let t1 = fx.task("t1");
    assert_eq!((t1.state, t1.failures), (TaskState::Blocked, 0));
    assert!(t1.block.as_ref().unwrap().text.contains("cannot spawn"));
}

/// Rung 1 restarts the stall clock of a turn still open: the gates' time is not the
/// worker's silence.
#[test]
fn a_rung_1_bounce_does_not_stall_a_turn_left_open() {
    let (mut fx, window) = working_on(PROFILE, CHECK_MODE);
    let effects = fx.tool(window, "task_done", json!({"summary": "s"}));
    let (op, _) = ops_in(&effects, "VerifyDone")[0].clone();
    let result = fx.clean_check("t1");
    let effects = fx.done(op, result);
    let (op, _) = only_op(&effects, "Check");
    // The check takes 20 minutes and the worker's turn is still open.
    let later = fx.now + 1200;
    let bounced = fx.send(
        later,
        crate::run::engine::EventKind::OpDone {
            run_id: RUN_ID.into(),
            op,
            result: check_result(false),
        },
    );
    assert_eq!(fx.task("t1").state, TaskState::Working);
    assert!(!bounced.contains(&Effect::Interrupt { window_id: window }));
    let effects = fx.tick();
    assert!(
        !effects.contains(&Effect::Interrupt { window_id: window }),
        "{effects:#?}"
    );
    fx.signal(window, AgentSignal::Activity);
    assert_alive(&fx);
}
