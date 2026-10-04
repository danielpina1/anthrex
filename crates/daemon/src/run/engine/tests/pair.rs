//! Milestone 9.5 task M9.5.16: a paired task's test writer commits the failing test, a
//! red-only proof confirms it fails there, and a separate implementer makes it pass in
//! the same checkout (decisions 24–26; rulings RP-1, RP-2). The implementer's half,
//! its signals and the slots are in `pair_implementer.rs`.

use proto::{AgentRole, GateCounts, ModelEntry, PairPhase, Runtime, Strength, TaskState};
use serde_json::json;

use super::dispatch::{replies, task_path};
use super::fixture::*;
use super::gates::only_op;
use super::holds::delivers;
use super::turns::killed_exit;
use crate::headless::HeadlessSpec;
use crate::run::contract::{DONE_ACCEPTED, WORKER_CONTRACT};
use crate::run::contract_patterns::{
    TEST_WRITER_CONTRACT, pair_implementer_note, red_check_failed_message, test_writer_prompt,
};
use crate::run::engine::{Effect, OpKind, OpResult};
use crate::run::model::OpId;
use crate::run::proof::proof_command;

/// A paired task's extra lines.
pub(super) const PAIRED: &str = "pair = true\ntest_to_write = \"a::works\"";
pub(super) const TEST: &str = "a::works";
/// The single-test command of the fixture's profile.
pub(super) const SINGLE: &str = "cargo test -- --exact {test}";
const RED_TAIL: &str = "running 1 test\ntest a::works ... ok";

/// The launch op of a session: its name, spec, first turn and checkout.
pub(super) struct Launch {
    pub name: String,
    pub spec: HeadlessSpec,
    pub first_turn: String,
    pub worktree: std::path::PathBuf,
}

pub(super) fn launch_of(kind: OpKind) -> Launch {
    match kind {
        OpKind::CreateWindow {
            name,
            spec,
            first_turn,
            worktree,
            ..
        } => Launch {
            name,
            spec: *spec,
            first_turn,
            worktree,
        },
        other => panic!("not a launch: {other:?}"),
    }
}

pub(super) fn role_of(launch: &Launch) -> AgentRole {
    launch.spec.mcp.as_ref().expect("an MCP target").role
}

/// A running (`--yes`) run of `tasks` on `profile` with `config`, `edit` applied to the
/// built run; its first `CreateWindow`'s launch and window once the checkouts are ready.
pub(super) fn running(
    profile: &str,
    tasks: &[String],
    config: config::Orchestrator,
    edit: impl FnOnce(&mut crate::run::model::Run),
) -> (Fixture, Launch, u32) {
    let plan = plan_with(profile, tasks);
    let mut fx = Fixture::with_config(&plan, config);
    fx.start_with(true, edit);
    let (op, _) = fx.op("CreateRunBranch");
    fx.done(op, OpResult::Worktree { head: BASE.into() });
    fx.complete_prepares();
    let (_, kind) = fx.ops("CreateWindow")[0].clone();
    let windows = fx.complete_windows();
    let window = windows
        .iter()
        .find(|(t, _)| t == "t1")
        .expect("t1 launched")
        .1;
    (fx, launch_of(kind), window)
}

/// A running paired `t1` (S, owning `crates/a/**`): its test writer's launch and window.
pub(super) fn paired() -> (Fixture, Launch, u32) {
    let tasks = [task("t1", "S", "a", PAIRED)];
    running(PROFILE, &tasks, config::Orchestrator::default(), |_| {})
}

/// A tool call of `t1`'s test writer from `window`.
pub(super) fn writer_tool(
    fx: &mut Fixture,
    window: u32,
    tool: &str,
    args: serde_json::Value,
) -> Vec<Effect> {
    fx.tool_as(AgentRole::TestWriter, window, "t1", tool, args)
}

/// The test writer's `task_done` naming `red`, accepted by a clean `VerifyDone`, then its
/// turn's end; the red check's `Proof` op.
pub(super) fn claim_red(fx: &mut Fixture, window: u32, red: &str) -> (OpId, OpKind) {
    let args = json!({"summary": "the failing test", "test": TEST, "red": red});
    let effects = writer_tool(fx, window, "task_done", args);
    let (op, _) = only_op(&effects, "VerifyDone");
    let result = fx.clean_check("t1");
    let effects = fx.done(op, result);
    assert_eq!(replies(&effects), vec![Ok(DONE_ACCEPTED.to_string())]);
    fx.turn_completed(window);
    only_op(&effects, "Proof")
}

/// The red-only proof's result: the test failed at red (`red_failed`), or passed.
pub(super) fn red_check(red_failed: bool) -> OpResult {
    OpResult::Proof {
        red_failed,
        head_passed: false,
        matched: false,
        red_tail: RED_TAIL.into(),
        head_tail: String::new(),
    }
}

/// `paired()` through a confirmed red: the implementer's launch and window, and the
/// confirming step's effects.
pub(super) fn implementing() -> (Fixture, Launch, u32, Vec<Effect>) {
    let (mut fx, _, writer) = paired();
    let (op, _) = claim_red(&mut fx, writer, &HEAD[..7]);
    let effects = fx.done(op, red_check(true));
    let (_, kind) = only_op(&effects, "CreateWindow");
    let window = fx.complete_windows()[0].1;
    (fx, launch_of(kind), window, effects)
}

fn entry(runtime: Runtime, model: &str, strength: Strength) -> ModelEntry {
    ModelEntry {
        runtime,
        model: model.to_string(),
        strength,
        note: String::new(),
    }
}

#[test]
fn a_paired_task_starts_with_a_test_writer_on_the_peer_runtime() {
    let (fx, launch, _) = paired();
    let t1 = fx.task("t1");
    assert_eq!(launch.name, format!("{H4}/t1.t1"));
    assert_eq!(role_of(&launch), AgentRole::TestWriter);
    assert_eq!(t1.route.runtime, Runtime::Claude);
    assert_eq!(
        (launch.spec.runtime, launch.spec.model.as_str()),
        (Runtime::Codex, "")
    );
    assert_eq!(launch.spec.effort, t1.route.effort, "the task's effort");
    assert_eq!(launch.spec.instructions, TEST_WRITER_CONTRACT);
    assert_eq!(launch.first_turn, test_writer_prompt(fx.run(), t1));
    assert_eq!(launch.worktree, task_path("t1"), "the task's own checkout");
    let run_ref = launch.spec.run_ref.clone().expect("a run ref");
    assert_eq!((run_ref.role, run_ref.session), (AgentRole::TestWriter, 1));
    let pair = t1.pair.clone().expect("the pair starts at dispatch");
    assert_eq!(pair.phase, PairPhase::Writing);
    assert_eq!(pair.writer_route.runtime, Runtime::Codex);
    assert_eq!(pair.writer_sessions, 1);
    assert_eq!(t1.rounds.len(), 1);
    assert_eq!(t1.rounds[0].role, AgentRole::TestWriter);
    assert_eq!(t1.rounds[0].route, pair.writer_route);
    assert_eq!(t1.state, TaskState::Working);
    // Decision 9a's dispatch-time record: trigger `test_writer`, on the writer's route.
    let decision = t1.routing_decisions.last().expect("a routing decision");
    assert_eq!(
        (decision.role, decision.session),
        (AgentRole::TestWriter, 1)
    );
    assert_eq!(
        (decision.trigger.as_str(), &decision.chosen),
        ("test_writer", &pair.writer_route)
    );
    // The prompt, exactly.
    let want = format!(
        "[anthrex] Test for task t1: Title t1\nRun goal: Engine test\nWorktree: {}\n\
         Branch: {}\nStart commit: b0b0b0b\nTest to write: a::works\n\
         Single-test command: {SINGLE}\n\nYour job: write and commit only the failing test \
         a::works. A different agent implements the behaviour after you.\n\n\
         This task owns:\n- crates/a/**\nAcceptance criteria:\n- Accept t1\n\nBrief t1",
        task_path("t1").display(),
        t1.branch
    );
    assert_eq!(launch.first_turn, want);

    // A Claude-only roster: the task's own route.
    let claude_only = config::Orchestrator {
        models: vec![
            entry(Runtime::Claude, "claude-haiku-4-5", Strength::Fast),
            entry(Runtime::Claude, "claude-sonnet-5", Strength::Standard),
            entry(Runtime::Claude, "claude-opus-5-5", Strength::Frontier),
        ],
        ..config::Orchestrator::default()
    };
    let tasks = [task("t1", "S", "a", PAIRED)];
    let (fx, launch, _) = running(PROFILE, &tasks, claude_only, |_| {});
    assert_eq!(role_of(&launch), AgentRole::TestWriter);
    let route = &fx.task("t1").route;
    assert_eq!(
        (launch.spec.runtime, &launch.spec.model),
        (Runtime::Claude, &route.model)
    );
    // Codex not installed for the run: the task's own route too.
    let (fx, launch, _) = running(PROFILE, &tasks, config::Orchestrator::default(), |run| {
        run.orch.installed.insert("codex".into(), false);
    });
    assert_eq!(role_of(&launch), AgentRole::TestWriter);
    assert_eq!(launch.spec.runtime, Runtime::Claude);
    assert_eq!(
        fx.task("t1").pair.as_ref().unwrap().writer_route,
        fx.task("t1").route
    );
}

/// Lines 1, 6 and 8–12 are the worker contract's word for word; line 3 is the worker's
/// with a different first sentence.
#[test]
fn the_test_writer_contract_matches_the_workers_lines() {
    let numbered = |text: &'static str| -> Vec<&'static str> { text.lines().collect() };
    let (writer, worker) = (numbered(TEST_WRITER_CONTRACT), numbered(WORKER_CONTRACT));
    assert_eq!(writer.len(), 13);
    assert_eq!(
        writer[0],
        "You are the test writer in an anthrex orchestration run. A different agent writes the implementation after you."
    );
    for n in [1, 6, 8, 9, 10, 11, 12] {
        assert_eq!(writer[n], worker[n], "line {n}");
    }
    // "3. <first sentence>. <the rest>"
    let after_first = |line: &'static str| line.splitn(3, ". ").nth(2).expect("sentences");
    assert_eq!(after_first(writer[3]), after_first(worker[3]));
    assert_ne!(writer[3], worker[3]);
}

#[test]
fn the_writer_must_end_on_red() {
    let (mut fx, _, window) = paired();
    let args = json!({"summary": "the failing test", "test": TEST, "red": "abcdef1"});
    let effects = writer_tool(&mut fx, window, "task_done", args);
    let (op, kind) = only_op(&effects, "VerifyDone");
    let OpKind::VerifyDone { red, .. } = kind else {
        unreachable!()
    };
    assert_eq!(red.as_deref(), Some("abcdef1"));
    let result = fx.clean_check("t1");
    let effects = fx.done(op, result);
    assert_eq!(
        replies(&effects),
        vec![Err(
            "task_done rejected: you are the test writer, so red must be your last commit (HEAD is d1d1d1d, red is abcdef1). Commit only the failing test, then call task_done again."
                .to_string()
        )]
    );
    let t1 = fx.task("t1");
    assert_eq!(
        (t1.state, t1.failures),
        (TaskState::Working, 0),
        "a rejection counts nothing"
    );
    assert!(ops_in(&effects, "Proof").is_empty());
    // A worker's call from the test writer's window is not the current session's.
    let effects = fx.tool(window, "task_done", json!({"summary": "x"}));
    assert_eq!(
        replies(&effects),
        vec![Err(
            "this window is not the current worker of task t1".to_string()
        )]
    );
    // Red at the head (a short sha names it): accepted, and the red check runs alone.
    let (op, kind) = claim_red(&mut fx, window, "d1d1d1d");
    assert_eq!(fx.task("t1").state, TaskState::Proof);
    assert_eq!(fx.task("t1").gate_op, Some(op));
    let OpKind::Proof {
        red,
        head,
        command,
        red_only,
        path,
        ..
    } = kind
    else {
        unreachable!()
    };
    assert!(
        red_only,
        "the red check is the proof's red-only mode (RP-1)"
    );
    assert_eq!((red.as_str(), head.as_str()), (HEAD, HEAD), "the full red");
    assert_eq!(command, proof_command(SINGLE, TEST));
    assert_eq!(path, task_path("t1.proof"));
}

#[test]
fn red_confirmed_starts_the_implementer_in_the_same_checkout() {
    let (mut fx, writer_launch, writer) = paired();
    // One red check that saw the test pass: the writer's failure, kept on the pair.
    let (op, _) = claim_red(&mut fx, writer, HEAD);
    fx.done(op, red_check(false));
    assert_eq!(fx.task("t1").failures, 1);
    let (op, _) = claim_red(&mut fx, writer, HEAD);
    let effects = fx.done(op, red_check(true));
    let t1 = fx.task("t1");
    let pair = t1.pair.clone().unwrap();
    assert_eq!(pair.phase, PairPhase::Implementing);
    assert_eq!(
        (pair.test.as_deref(), pair.red.as_deref(), pair.red_checked),
        (Some(TEST), Some(HEAD), Some(true))
    );
    assert_eq!(pair.writer_failures, 1);
    assert_eq!((t1.failures, t1.stalls, t1.budget_exceeded), (0, 0, 0));
    assert_eq!(t1.bounces, GateCounts::default());
    assert_eq!(t1.state, TaskState::Working);
    // The writer's session is retired, not killed.
    assert!(effects.contains(&Effect::RetireWindow { window_id: writer }));
    assert!(!effects.contains(&Effect::KillWindow { window_id: writer }));
    assert!(t1.rounds[0].retiring);
    let (_, kind) = only_op(&effects, "CreateWindow");
    let launch = launch_of(kind);
    assert_eq!(launch.name, format!("{H4}/t1.w2"));
    assert_eq!(role_of(&launch), AgentRole::Worker);
    assert_eq!(launch.spec.instructions, WORKER_CONTRACT);
    assert_eq!(launch.spec.runtime, Runtime::Claude, "the task's own route");
    assert_eq!(launch.worktree, writer_launch.worktree);
    assert_eq!(launch.spec.cwd, writer_launch.spec.cwd);
    assert_eq!(t1.session, 2);
    assert_eq!(t1.rounds.last().unwrap().role, AgentRole::Worker);
    let note = pair_implementer_note(TEST, HEAD);
    assert_eq!(
        note,
        "The failing test a::works was committed in d1d1d1d by a separate test writer. Make it pass without weakening it; you may add more tests. Call task_done with test a::works and red d1d1d1d, or leave both out."
    );
    assert!(
        launch
            .first_turn
            .contains(&format!("{note}\n\n{}", t1.spec.brief)),
        "the note comes right before the brief: {}",
        launch.first_turn
    );
    let logged = fx
        .run()
        .log
        .iter()
        .map(|l| l.text.as_str())
        .collect::<Vec<_>>();
    assert!(
        logged.contains(&"pair t1: red d1d1d1d fails as it should; implementer started"),
        "{logged:#?}"
    );
}

#[test]
fn a_red_that_passes_is_a_proof_failure_for_the_writer() {
    let (mut fx, _, writer) = paired();
    let route = fx.task("t1").route.clone();
    let (op, _) = claim_red(&mut fx, writer, HEAD);
    let effects = fx.done(op, red_check(false));
    let text = red_check_failed_message(HEAD, TEST, &proof_command(SINGLE, TEST), RED_TAIL);
    assert_eq!(
        text,
        format!(
            "[anthrex] The red check failed: at your red commit d1d1d1d the test a::works passed, so it does not check missing behaviour. Change the test so it fails without the implementation, commit, then call task_done again.\nCommand: {}\nLast 40 lines:\n{RED_TAIL}",
            proof_command(SINGLE, TEST)
        )
    );
    // Rung 1: to the same test writer.
    assert_eq!(delivers(&effects), vec![text]);
    let t1 = fx.task("t1");
    assert_eq!(
        (t1.state, t1.rung, t1.bounces.proof, t1.failures),
        (TaskState::Working, 1, 1, 1)
    );
    assert_eq!(t1.pair.as_ref().unwrap().phase, PairPhase::Writing);
    assert!(t1.pair.as_ref().unwrap().red.is_none());
    assert!(ops_in(&effects, "CreateWindow").is_empty());
    // Rung 2: a fresh test writer, once the first has exited.
    let (op, _) = claim_red(&mut fx, writer, HEAD);
    let effects = fx.done(op, red_check(false));
    assert!(effects.contains(&Effect::KillWindow { window_id: writer }));
    let effects = killed_exit(&mut fx, writer);
    let (op, _) = only_op(&effects, "DiffSoFar");
    let effects = fx.done(
        op,
        OpResult::Diff {
            stat: " a | 1 +".into(),
            patch: "+x".into(),
        },
    );
    let (_, kind) = only_op(&effects, "CreateWindow");
    let launch = launch_of(kind);
    assert_eq!(launch.name, format!("{H4}/t1.t2"));
    assert_eq!(role_of(&launch), AgentRole::TestWriter);
    assert_eq!(launch.spec.instructions, TEST_WRITER_CONTRACT);
    let t1 = fx.task("t1");
    assert!(
        launch
            .first_turn
            .starts_with(&test_writer_prompt(fx.run(), t1)),
        "{}",
        launch.first_turn
    );
    assert!(
        launch.first_turn.contains(
            "\n\nThis is session 2 of this task.\nWhy a new session: the proof gate failed again"
        ),
        "{}",
        launch.first_turn
    );
    assert_eq!(
        t1.route, route,
        "rung 2 escalates the writer, not the implementer's route"
    );
    let pair = t1.pair.as_ref().unwrap();
    assert_eq!((pair.phase, pair.writer_sessions), (PairPhase::Writing, 2));
    assert_eq!(t1.rounds.last().unwrap().route, pair.writer_route);
}

/// Decision 26: a restart resumes the live session of the phase the task was in, a
/// test writer's as a worker's.
#[test]
fn a_restart_resumes_the_test_writer() {
    let (mut fx, _, writer) = paired();
    let session_id = "codex-thread-1".to_string();
    fx.signal(
        writer,
        crate::run::engine::AgentSignal::Init {
            session_id: session_id.clone(),
        },
    );
    super::control_restore::restart(&mut fx, Vec::new());
    let effects = super::control::resume(&mut fx);
    assert_eq!(
        super::control_restore::resumes(&effects),
        vec![(
            writer,
            session_id,
            crate::run::contract::RESUME_WORKER.to_string()
        )]
    );
    let t1 = fx.task("t1");
    assert_eq!(t1.state, TaskState::Working);
    assert_eq!(t1.pair.as_ref().unwrap().phase, PairPhase::Writing);

    // A test writer's launch the restart lost is launched again, as a worker's is.
    let plan = plan_with(PROFILE, &[task("t1", "S", "a", PAIRED)]);
    let mut fx = Fixture::new(&plan);
    fx.ready(true);
    fx.complete_prepares();
    let (lost, _) = fx.op("CreateWindow");
    super::control_restore::restart(&mut fx, Vec::new());
    let effects = super::control::resume(&mut fx);
    let (op, kind) = only_op(&effects, "CreateWindow");
    assert_ne!(op, lost);
    let launch = launch_of(kind);
    assert_eq!(launch.name, format!("{H4}/t1.t1"));
    assert_eq!(role_of(&launch), AgentRole::TestWriter);
    assert_eq!(fx.task("t1").rounds.len(), 1, "the same round, relaunched");
}

#[path = "pair_implementer.rs"]
mod implementer;
