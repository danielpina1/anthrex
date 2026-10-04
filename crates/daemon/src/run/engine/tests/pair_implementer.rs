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
            restore_from: Default::default(),
            unlimited: 0,
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

/// Ruling T16-8 (b): the done gate's restore command restores each deleted test file
/// from its own base, one `git checkout` per base, joined by `&&`.
#[test]
fn each_deleted_test_restores_from_its_own_base() {
    let (mut fx, _, window, _) = implementing();
    let effects = fx.tool(window, "task_done", json!({"summary": "made it pass"}));
    let (op, _) = only_op(&effects, "VerifyDone");
    let (writers, theirs) = ("crates/a/tests/w.rs", "crates/a/tests/x.rs");
    let deleted = |path: &str| Signal::DeletedTestFile { path: path.into() };
    let mut result = at_impl(&fx);
    let base = "e5e5e5e5e5e5e5e5e5e5e5e5e5e5e5e5e5e5e5e5";
    if let OpResult::DoneChecked { signals, .. } = &mut result {
        *signals = Some(Box::new(ClaimSignals {
            list: vec![deleted(writers), deleted(theirs)],
            more: 0,
            base: base.into(),
            restore_from: [
                (writers.to_string(), HEAD.to_string()),
                (theirs.to_string(), base.to_string()),
            ]
            .into(),
            unlimited: 0,
        }));
    }
    let effects = fx.done(op, result);
    let [Err(text)] = &replies(&effects)[..] else {
        panic!("{effects:#?}")
    };
    let command = format!(
        "git checkout {} -- '{writers}' && git checkout {} -- '{theirs}'",
        &HEAD[..7],
        &base[..7]
    );
    assert!(text.contains(&command), "{text}");
}

/// Ruling T16-9 (1): the reviewer of a paired task sees the test writer's kept signals,
/// labelled, after the implementer's; the implementer's claim no longer discards them.
#[test]
fn the_reviewer_sees_the_writers_signals_too() {
    let tasks = [task("t1", "S", "a", PAIRED)];
    let (mut fx, _, writer) = running(
        &signalled(),
        &tasks,
        config::Orchestrator::default(),
        |_| {},
    );
    let loss = |path: &str, line: u32| Signal::AssertionLoss {
        path: path.into(),
        line,
        removed: 1,
        added: 0,
    };
    let with = |mut result: OpResult, signal: Signal| {
        if let OpResult::DoneChecked { signals, .. } = &mut result {
            *signals = Some(Box::new(ClaimSignals {
                list: vec![signal],
                more: 1,
                base: HEAD.into(),
                restore_from: Default::default(),
                unlimited: 0,
            }));
        }
        result
    };
    let args = json!({"summary": "the failing test", "test": TEST, "red": HEAD});
    let effects = writer_tool(&mut fx, writer, "task_done", args);
    let (op, _) = only_op(&effects, "VerifyDone");
    let theirs = loss("crates/a/tests/old.rs", 4);
    let effects = fx.done(op, with(fx.clean_check("t1"), theirs.clone()));
    fx.turn_completed(writer);
    let (op, _) = only_op(&effects, "Proof");
    fx.done(op, red_check(true));
    let window = fx.complete_windows()[0].1;
    let effects = fx.tool(window, "task_done", json!({"summary": "made it pass"}));
    let (op, _) = only_op(&effects, "VerifyDone");
    let ours = loss("crates/a/tests/t.rs", 3);
    let effects = fx.done(op, with(at_impl(&fx), ours.clone()));
    assert_eq!(replies(&effects), vec![Ok(DONE_ACCEPTED.to_string())]);
    fx.turn_completed(window);
    let t1 = fx.task("t1");
    assert_eq!(t1.signals, vec![ours, theirs]);
    assert_eq!(t1.signals_more, 2, "both claims' overflow");
    let (op, _) = only_op(&effects, "Proof");
    let passed = OpResult::Proof {
        red_failed: true,
        head_passed: true,
        matched: true,
        red_tail: String::new(),
        head_tail: format!("test {TEST} ... ok"),
    };
    let effects = fx.done(op, passed);
    let (op, _) = only_op(&effects, "PrepareReview");
    let (_, kind) = reviewer(&mut fx, op, "diff --git a/x b/x");
    let OpKind::CreateWindow { first_turn, .. } = kind else {
        unreachable!()
    };
    assert!(
        first_turn.contains("- W1 crates/a/tests/t.rs:3: 1 assertion lines removed, 0 added\n"),
        "{first_turn}"
    );
    assert!(
        first_turn.contains(
            "- W2 crates/a/tests/old.rs:4: 1 assertion lines removed, 0 added (test writer)"
        ),
        "{first_turn}"
    );
}

/// Ruling T16-9 (2): a claim whose writer-path read ran without a pathspec (over 256
/// writer paths) leaves one run-log line naming the count.
#[test]
fn an_unlimited_writer_path_read_is_logged() {
    let (mut fx, _, window, _) = implementing();
    let effects = fx.tool(window, "task_done", json!({"summary": "made it pass"}));
    let (op, _) = only_op(&effects, "VerifyDone");
    let mut result = at_impl(&fx);
    if let OpResult::DoneChecked { signals, .. } = &mut result {
        *signals = Some(Box::new(ClaimSignals {
            unlimited: 300,
            ..ClaimSignals::default()
        }));
    }
    fx.done(op, result);
    let line = crate::run::contract_patterns::writer_paths_unlimited_line("t1", 300);
    assert_eq!(
        line,
        "task t1's test writer changed 300 paths, over the pathspec limit of 256; its signals were read without one and filtered"
    );
    let logged = (fx.run().log.iter()).filter(|l| l.text == line).count();
    assert_eq!(logged, 1);
}

/// Re-review 3's NI-3: past the restore command's limit, the fallback text still says
/// which file comes from which commit.
#[test]
fn the_fallback_restore_text_names_each_files_base() {
    let long = format!("crates/a/tests/{}.rs", "w".repeat(990));
    let groups = [
        (HEAD, vec![long.clone()]),
        (
            "e5e5e5e5e5e5e5e5e5e5e5e5e5e5e5e5e5e5e5e5",
            vec!["crates/a/tests/x.rs".to_string()],
        ),
    ];
    let text = crate::run::contract::deleted_test_files_message(&groups);
    let want = format!(
        "Restore each deleted test file from its commit ({} from {}; crates/a/tests/x.rs from e5e5e5e), then commit,",
        crate::run::contract::shown(&long),
        &HEAD[..7]
    );
    assert!(text.contains(&want), "{text}");
}

/// The reviewer's first turn of `t1`, implementing in `fx` from `window`, after a claim
/// whose `VerifyDone` carried `signals`.
fn reviewer_turn(fx: &mut Fixture, window: u32, signals: Option<ClaimSignals>) -> String {
    let effects = fx.tool(window, "task_done", json!({"summary": "made it pass"}));
    let (op, _) = only_op(&effects, "VerifyDone");
    let mut result = at_impl(fx);
    if let OpResult::DoneChecked { signals: s, .. } = &mut result {
        *s = signals.map(Box::new);
    }
    let effects = fx.done(op, result);
    assert_eq!(replies(&effects), vec![Ok(DONE_ACCEPTED.to_string())]);
    fx.turn_completed(window);
    let (op, _) = only_op(&effects, "Proof");
    let passed = OpResult::Proof {
        red_failed: true,
        head_passed: true,
        matched: true,
        red_tail: String::new(),
        head_tail: format!("test {TEST} ... ok"),
    };
    let effects = fx.done(op, passed);
    let (op, _) = only_op(&effects, "PrepareReview");
    let (_, kind) = reviewer(fx, op, "diff --git a/x b/x");
    let OpKind::CreateWindow { first_turn, .. } = kind else {
        unreachable!()
    };
    first_turn
}

/// Whole-branch review B, I1: an implementing pair's signals are read whatever the
/// profile says. Untiered, its `VerifyDone` still asks, with red and empty lists.
#[test]
fn an_untiered_pairs_implementer_still_has_its_signals_read() {
    let (mut fx, _, window, _) = implementing();
    fx.run_mut().profile.tiers = Default::default();
    let effects = fx.tool(window, "task_done", json!({"summary": "made it pass"}));
    let (_, kind) = only_op(&effects, "VerifyDone");
    let OpKind::VerifyDone { signals, .. } = kind else {
        unreachable!()
    };
    let read = SignalsSpec {
        test_paths: Vec::new(),
        skip_markers: Vec::new(),
        red: Some(HEAD.into()),
    };
    assert_eq!(signals, Some(read));
}

/// Whole-branch review B, I1 (3) and review D, M-4: the pair's note goes to the
/// reviewer only when the accepted claim's signals were read, in the shipped wording.
#[test]
fn the_pair_note_is_given_only_when_the_signals_were_read() {
    let note = pair_reviewer_note(TEST, HEAD);
    assert_eq!(
        note,
        "A separate test writer committed the test a::works at d1d1d1d; the test writer's paths are measured from the red commit, everything else from the merge base; signals marked (test writer) are the writer's own."
    );
    for (signals, noted) in [(None, 0), (Some(ClaimSignals::default()), 1)] {
        let tasks = [task("t1", "S", "a", PAIRED)];
        let no_check = super::super::gates::no_check();
        let (mut fx, _, writer) =
            running(&no_check, &tasks, config::Orchestrator::default(), |_| {});
        let (op, _) = claim_red(&mut fx, writer, HEAD);
        let effects = fx.done(op, red_check(true));
        let (_, _) = only_op(&effects, "CreateWindow");
        let window = fx.complete_windows()[0].1;
        let first_turn = reviewer_turn(&mut fx, window, signals);
        assert_eq!(first_turn.matches(&note).count(), noted, "{first_turn}");
    }
}
