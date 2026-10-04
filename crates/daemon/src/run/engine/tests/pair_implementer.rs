//! Milestone 9.5 task M9.5.16, the implementer's half: it inherits the writer's test
//! and red, its weakening signals are read from red (ruling RP-2), `run retry` restarts
//! the phase the task was in, the pair holds one writer slot, and the test writer's
//! rate limits count for its own runtime (ruling RC-1, review ruling I4).

use proto::{AgentRole, PairPhase, TaskState};
use serde_json::json;

use super::super::control::retry;
use super::super::dispatch::replies;
use super::super::done::one_reply;
use super::super::gates::only_op;
use super::super::gates_review::reviewer;
use super::super::turns::killed_exit;
use super::*;
use crate::run::contract::DONE_ACCEPTED;
use crate::run::contract_patterns::pair_reviewer_note;
use crate::run::engine::schedule::writers_busy;
use crate::run::engine::{AgentSignal, OpKind, OpResult};
use crate::run::tiers::{ClaimSignals, Signal, SignalsSpec};

/// The implementer's head.
const IMPL: &str = "e2e2e2e2e2e2e2e2e2e2e2e2e2e2e2e2e2e2e2e2";

/// A clean `DoneChecked` for `t1` at the implementer's head.
fn at_impl(fx: &Fixture) -> OpResult {
    let mut result = fx.clean_check("t1");
    if let OpResult::DoneChecked { head, .. } = &mut result {
        *head = IMPL.into();
    }
    result
}

#[test]
fn the_implementer_inherits_test_and_red() {
    let (mut fx, _, window, _) = implementing();
    let wrong = "task_done rejected: this task's test was written by the test writer; name test a::works and red d1d1d1d, or leave both out";
    for args in [
        json!({"summary": "s", "test": "a::other"}),
        json!({"summary": "s", "red": "abcdef1"}),
        json!({"summary": "s", "test": TEST, "red": "abcdef1"}),
    ] {
        let effects = fx.tool(window, "task_done", args.clone());
        assert_eq!(replies(&effects), vec![Err(wrong.to_string())], "{args}");
        assert!(ops_in(&effects, "VerifyDone").is_empty(), "{args}");
    }
    // The same test and red, named, go on to the check (one the check could not read
    // is answered and leaves the task working).
    let args = json!({"summary": "s", "test": TEST, "red": "d1d1d1d"});
    let effects = fx.tool(window, "task_done", args);
    let (op, kind) = only_op(&effects, "VerifyDone");
    let OpKind::VerifyDone { red, .. } = kind else {
        unreachable!()
    };
    assert_eq!(red.as_deref(), Some("d1d1d1d"));
    let failed = OpResult::Failed {
        message: "busy".into(),
    };
    assert!(one_reply(&fx.done(op, failed)).is_err());
    // Left out, both are filled from the pair.
    let effects = fx.tool(window, "task_done", json!({"summary": "made it pass"}));
    let (op, kind) = only_op(&effects, "VerifyDone");
    let OpKind::VerifyDone { red, .. } = kind else {
        unreachable!()
    };
    assert_eq!(red.as_deref(), Some(HEAD), "filled from the pair");
    let effects = fx.done(op, at_impl(&fx));
    assert_eq!(replies(&effects), vec![Ok(DONE_ACCEPTED.to_string())]);
    let (_, kind) = only_op(&effects, "Proof");
    let OpKind::Proof {
        red,
        head,
        command,
        red_only,
        ..
    } = kind
    else {
        unreachable!()
    };
    assert!(!red_only, "the implementer's proof is the full one");
    assert_eq!((red.as_str(), head.as_str()), (HEAD, IMPL));
    assert_eq!(command, crate::run::proof::proof_command(SINGLE, TEST));
    // The test writer's window is no longer the task's writer.
    let writer = fx.task("t1").rounds[0].window_id.unwrap();
    let effects = writer_tool(&mut fx, writer, "task_done", json!({"summary": "x"}));
    assert_eq!(
        replies(&effects),
        vec![Err(
            "this window is not the current worker of task t1".to_string()
        )]
    );
}

/// `PROFILE` without `check`, tiered by `test_paths` and `skip_markers`.
fn signalled() -> String {
    super::super::gates::no_check().replace(
        "setup = ",
        "test_paths = [\"tests/**\", \"crates/*/tests/**\"]\nskip_markers = [\"#[ignore]\"]\nsetup = ",
    )
}

fn spec(red: Option<&str>) -> SignalsSpec {
    SignalsSpec {
        test_paths: vec!["tests/**".into(), "crates/*/tests/**".into()],
        skip_markers: vec!["#[ignore]".into()],
        red: red.map(str::to_string),
    }
}

/// RP-2, the engine's half: the implementer's `VerifyDone` reads its signals from red,
/// and a weakened assertion in the writer's test reaches the reviewer as a `W` signal
/// with the pair's note. (Measured from the stage head the test is new and nothing
/// shows: `driver/ops_signals_tests_pair.rs` runs both through git.)
#[test]
fn a_weakened_test_raises_a_signal_from_red() {
    let tasks = [task("t1", "S", "a", PAIRED)];
    let (mut fx, _, writer) = running(
        &signalled(),
        &tasks,
        config::Orchestrator::default(),
        |_| {},
    );
    let args = json!({"summary": "the failing test", "test": TEST, "red": HEAD});
    let effects = writer_tool(&mut fx, writer, "task_done", args);
    let (op, kind) = only_op(&effects, "VerifyDone");
    let OpKind::VerifyDone { signals, .. } = kind else {
        unreachable!()
    };
    assert_eq!(
        signals,
        Some(spec(None)),
        "the writer's own claim: from the stage head"
    );
    let result = fx.clean_check("t1");
    let effects = fx.done(op, result);
    fx.turn_completed(writer);
    let (op, _) = only_op(&effects, "Proof");
    let effects = fx.done(op, red_check(true));
    let (_, _) = only_op(&effects, "CreateWindow");
    let window = fx.complete_windows()[0].1;
    let effects = fx.tool(window, "task_done", json!({"summary": "made it pass"}));
    let (op, kind) = only_op(&effects, "VerifyDone");
    let OpKind::VerifyDone { signals, .. } = kind else {
        unreachable!()
    };
    assert_eq!(
        signals,
        Some(spec(Some(HEAD))),
        "the implementer's: from red"
    );
    let weakened = Signal::AssertionLoss {
        path: "crates/a/tests/t.rs".into(),
        line: 3,
        removed: 1,
        added: 0,
    };
    let mut result = at_impl(&fx);
    if let OpResult::DoneChecked { signals, .. } = &mut result {
        *signals = Some(Box::new(ClaimSignals {
            list: vec![weakened.clone()],
            more: 0,
            base: HEAD.into(),
        }));
    }
    let effects = fx.done(op, result);
    assert_eq!(replies(&effects), vec![Ok(DONE_ACCEPTED.to_string())]);
    fx.turn_completed(window);
    assert_eq!(fx.task("t1").signals, vec![weakened]);
    let (op, _) = only_op(&effects, "Proof");
    let passed = OpResult::Proof {
        red_failed: true,
        head_passed: true,
        matched: true,
        red_tail: String::new(),
        head_tail: format!("test {TEST} ... ok"),
    };
    let effects = fx.done(op, passed);
    assert_eq!(fx.task("t1").state, TaskState::Review);
    let (op, _) = only_op(&effects, "PrepareReview");
    let (_, kind) = reviewer(&mut fx, op, "diff --git a/x b/x");
    let OpKind::CreateWindow { first_turn, .. } = kind else {
        unreachable!()
    };
    let note = pair_reviewer_note(TEST, HEAD);
    assert_eq!(
        note,
        "A separate test writer committed the test a::works at d1d1d1d; the weakening signals above are measured from that commit, so a W-signal on its files means the implementer changed the test."
    );
    let line = "- W1 crates/a/tests/t.rs:3: 1 assertion lines removed, 0 added";
    let at = first_turn.find(line).expect("the numbered signal");
    let noted = first_turn.find(&note).expect("the pair's note");
    assert!(at < noted, "the note follows the signals: {first_turn}");
    assert_eq!(first_turn.matches(&note).count(), 1);
}

#[test]
fn retry_restarts_the_phase_it_was_in() {
    // Writing: a fresh test writer.
    let (mut fx, _, writer) = paired();
    let args = json!({"kind": "environment", "reason": "no compiler"});
    assert!(one_reply(&writer_tool(&mut fx, writer, "task_blocked", args)).is_ok());
    assert_eq!(fx.task("t1").state, TaskState::Blocked);
    killed_exit(&mut fx, writer);
    let effects = retry(&mut fx, "t1");
    assert_eq!(
        one_reply(&effects),
        Ok("task t1 retried at rung 2: a fresh session starts".to_string())
    );
    let (op, _) = only_op(&effects, "DiffSoFar");
    let diff = OpResult::Diff {
        stat: String::new(),
        patch: String::new(),
    };
    let effects = fx.done(op, diff.clone());
    let (_, kind) = only_op(&effects, "CreateWindow");
    let launch = launch_of(kind);
    assert_eq!(launch.name, format!("{H4}/t1.t2"));
    assert_eq!(role_of(&launch), AgentRole::TestWriter);
    assert!(
        launch
            .first_turn
            .contains("Why a new session: the user retried it")
    );
    assert_eq!(
        fx.task("t1").pair.as_ref().unwrap().phase,
        PairPhase::Writing
    );

    // Implementing: a fresh implementer.
    let (mut fx, _, window, _) = implementing();
    let args = json!({"kind": "environment", "reason": "no compiler"});
    assert!(one_reply(&fx.tool(window, "task_blocked", args)).is_ok());
    killed_exit(&mut fx, window);
    let effects = retry(&mut fx, "t1");
    let (op, _) = only_op(&effects, "DiffSoFar");
    let effects = fx.done(op, diff);
    let (_, kind) = only_op(&effects, "CreateWindow");
    let launch = launch_of(kind);
    assert_eq!(launch.name, format!("{H4}/t1.w3"));
    assert_eq!(role_of(&launch), AgentRole::Worker);
    assert!(
        launch
            .first_turn
            .contains(&pair_implementer_note(TEST, HEAD))
    );
    assert_eq!(
        fx.task("t1").pair.as_ref().unwrap().phase,
        PairPhase::Implementing
    );
}

#[test]
fn one_writer_slot_throughout() {
    let tasks = [
        task("t1", "S", "a", PAIRED),
        task("t2", "S", "b", ""),
        task("t3", "S", "c", ""),
    ];
    let profile = profile_with("max_writers = 2");
    let (mut fx, _, writer) = running(&profile, &tasks, config::Orchestrator::default(), |_| {});
    let held = |fx: &Fixture, phase: &str| {
        assert_eq!(writers_busy(fx.run()), 2, "{phase}");
        assert_eq!(fx.task("t3").state, TaskState::Queued, "{phase}");
    };
    held(&fx, "writing");
    let (op, _) = claim_red(&mut fx, writer, HEAD);
    assert_eq!(fx.task("t1").state, TaskState::Proof);
    held(&fx, "the red check");
    fx.done(op, red_check(true));
    assert_eq!(fx.task("t1").rounds.last().unwrap().role, AgentRole::Worker);
    held(&fx, "implementing");
}

#[test]
fn a_test_writers_rate_limit_counts_for_its_runtime() {
    let tasks = [task("t1", "S", "a", PAIRED)];
    let profile = profile_with("max_writers = 2");
    let (mut fx, launch, writer) =
        running(&profile, &tasks, config::Orchestrator::default(), |_| {});
    assert_eq!(launch.spec.runtime, proto::Runtime::Codex);
    fx.signal(
        writer,
        AgentSignal::ApiRetry {
            error: "rate_limit".into(),
            delay_ms: 1_000,
        },
    );
    let run = fx.run();
    assert_eq!(run.rate_limits.get("codex"), Some(&1));
    assert_eq!(run.rate_limits.get("claude"), None);
    assert_eq!(run.concurrency["codex"].cap, 1, "Codex's cap is halved");
    assert!(!run.concurrency.contains_key("claude"));
}
