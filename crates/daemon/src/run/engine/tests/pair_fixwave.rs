//! The final fix wave's pair items in the reducer: a test writer's writer slot and cap
//! are its own runtime's (A-I3); its rung 2 is not the task's escalation (review B's
//! M2); the retry preview names its route (M3); the snapshot's writer failures (M7);
//! the implementer waits for room on the task's runtime (ruling FW-2 (c)), and waiting
//! implementers do not hold each other back (ruling FW-4 (I)).

use proto::{AgentRole, PairPhase, PlanEdit, Runtime, TaskState};

use super::super::control::resume;
use super::super::dispatch::edit;
use super::super::merge::window_of;
use super::*;
use crate::run::engine::concurrency::writers_busy_on;
use crate::run::engine::schedule::writer_slots;
use crate::run::model::RuntimeConcurrency;

/// `runtime`'s cap brought down to 1 by a rate limit at the fixture's start.
fn capped(run: &mut crate::run::model::Run, runtime: &str) {
    let held = RuntimeConcurrency {
        cap: 1,
        last_rate_limit_at: Some(2_000),
        ..RuntimeConcurrency::new(run.limits.max_writers)
    };
    run.concurrency.insert(runtime.into(), held);
}

/// Codex's cap brought down to 1 by a rate limit at the fixture's start.
fn codex_capped(run: &mut crate::run::model::Run) {
    capped(run, "codex");
}

/// Claude's cap brought down to 1 by a rate limit at the fixture's start.
fn claude_capped(run: &mut crate::run::model::Run) {
    capped(run, "claude");
}

/// A-I3: two paired Claude tasks whose test writers would both run on Codex, with
/// Codex's cap at 1. The first writer takes Codex's one slot, counted on Codex; the
/// second task waits for it, though Claude has room.
#[test]
fn the_peer_runtimes_cap_holds_back_a_second_test_writer() {
    let tasks = [task("t1", "S", "a", PAIRED), task("t2", "S", "b", PAIRED)];
    let plan = plan_with(&profile_with("max_writers = 3"), &tasks);
    let mut fx = Fixture::new(&plan);
    fx.start_with(true, codex_capped);
    let (op, _) = fx.op("CreateRunBranch");
    fx.done(op, OpResult::Worktree { head: BASE.into() });
    fx.launch_all();
    assert!(fx.run().limits.adaptive_concurrency);
    let t1 = fx.task("t1");
    assert_eq!(t1.route.runtime, Runtime::Claude);
    assert_eq!(
        writer_slots(t1),
        [Runtime::Codex],
        "the writer's slot is Codex's"
    );
    assert_eq!(writers_busy_on(fx.run(), Runtime::Codex), 1);
    assert_eq!(writers_busy_on(fx.run(), Runtime::Claude), 0);
    assert_eq!(fx.task("t2").state, TaskState::Queued, "Codex has no room");
    let writers: Vec<String> = (fx.ops("CreateWindow").into_iter())
        .map(|(_, kind)| launch_of(kind))
        .filter(|l| role_of(l) == proto::AgentRole::TestWriter)
        .map(|l| l.name)
        .collect();
    assert_eq!(writers, [format!("{H4}/t1.t1")]);
}

/// The test writer's two failed red checks: rung 2 for the writer.
fn writer_at_rung_2(fx: &mut Fixture, writer: u32) {
    for _ in 0..2 {
        let (op, _) = claim_red(fx, writer, HEAD);
        fx.done(op, red_check(false));
    }
    assert_eq!(fx.task("t1").rung, 2);
    assert_eq!(
        fx.task("t1").pair.as_ref().map(|p| p.phase),
        Some(proto::PairPhase::Writing)
    );
}

/// Review B's M2: a test writer's rung 2 is the writer's escalation, never the class
/// route's: neither the task's `max_rung` nor its history record reaches 2.
#[test]
fn a_test_writers_rung_2_is_not_the_tasks_escalation() {
    let (mut fx, _, writer) = paired();
    writer_at_rung_2(&mut fx, writer);
    fx.tick();
    let t1 = fx.task("t1");
    assert!(t1.max_rung < 2, "max_rung {}", t1.max_rung);
    let record =
        crate::run::history::task_record(fx.run(), t1, proto::TaskOutcome::Blocked, fx.now);
    assert!(record.max_rung < 2, "recorded {}", record.max_rung);
}

/// Review B's M3: while the test is being written, the retry preview names the test
/// writer's next route, the one `run retry` escalates, not the implementer's.
#[test]
fn the_retry_preview_names_the_writers_route() {
    use crate::run::engine::actions::{self, ActionNode};
    let (mut fx, launch, writer) = paired();
    assert_eq!(launch.spec.runtime, Runtime::Codex);
    let args = serde_json::json!({"kind": "question", "reason": "which file?"});
    writer_tool(&mut fx, writer, "task_blocked", args);
    assert_eq!(fx.task("t1").state, TaskState::Blocked);
    let i = fx.run().tasks.iter().position(|t| t.id() == "t1").unwrap();
    let current = fx.task("t1").pair.as_ref().unwrap().writer_route.clone();
    let next = crate::run::role_step::writer_step(fx.run(), i, &current);
    assert_eq!(next.runtime, Runtime::Codex, "{next:?}");
    let preview = actions::available(fx.run(), &ActionNode::Task("t1"))
        .into_iter()
        .find(|a| a.kind == proto::ActionKind::Retry)
        .expect("retry is listed")
        .effect;
    let model = match next.model.is_empty() {
        true => "default".to_string(),
        false => next.model.clone(),
    };
    let effort = next.effort.as_str().to_string();
    assert_eq!(
        preview,
        format!("retry t1: a fresh session at rung 2 on codex {model} ({effort} effort)")
    );
}

/// Review B's M7: while the test is being written, the snapshot's writer failures are
/// the ones `REPORT.md` and history read (the task's counters).
#[test]
fn the_snapshots_writer_failures_match_the_report_while_writing() {
    let (mut fx, _, writer) = paired();
    let (op, _) = claim_red(&mut fx, writer, HEAD);
    fx.done(op, red_check(false));
    let t1 = fx.task("t1");
    assert_eq!(crate::run::history::writer_failures(t1), 1);
    let info = crate::run::snapshot_patterns::pair_info(t1).expect("a pair");
    assert_eq!(info.writer_failures, 1);
}

/// Ruling FW-2 (c): at the hand-over the pair's slot moves from the writer's runtime to
/// the task's. With Claude's cap at 1 and held by `t2`, `t1`'s Claude implementer waits
/// as a dispatch would, and launches once Claude has room.
#[test]
fn the_implementer_waits_for_room_on_the_tasks_runtime() {
    let tasks = [task("t1", "S", "a", PAIRED), task("t2", "S", "b", "")];
    let plan = plan_with(&profile_with("max_writers = 3"), &tasks);
    let mut fx = Fixture::new(&plan);
    fx.start_with(true, claude_capped);
    let (op, _) = fx.op("CreateRunBranch");
    fx.done(op, OpResult::Worktree { head: BASE.into() });
    let windows = fx.launch_all();
    let writer = window_of(&windows, "t1");
    assert_eq!(writer_slots(fx.task("t1")), [Runtime::Codex]);
    assert_eq!(fx.task("t2").state, TaskState::Working);
    assert_eq!(writers_busy_on(fx.run(), Runtime::Claude), 1, "t2's");
    let (op, _) = claim_red(&mut fx, writer, &HEAD[..7]);
    let effects = fx.done(op, red_check(true));
    let t1 = fx.task("t1");
    assert_eq!(
        t1.pair.as_ref().map(|p| p.phase),
        Some(PairPhase::Implementing)
    );
    assert!(ops_in(&effects, "CreateWindow").is_empty(), "{effects:#?}");
    let effects = fx.tick();
    assert!(
        ops_in(&effects, "CreateWindow").is_empty(),
        "Claude has no room"
    );
    assert!(
        !fx.task("t1")
            .rounds
            .iter()
            .any(|r| r.role == AgentRole::Worker)
    );
    // Claude's cap comes back up: the implementer launches.
    fx.run_mut().concurrency.get_mut("claude").unwrap().cap = 2;
    let effects = fx.tick();
    let launches = ops_in(&effects, "CreateWindow");
    assert_eq!(launches.len(), 1, "{launches:#?}");
    let launch = launch_of(launches[0].1.clone());
    assert_eq!(role_of(&launch), AgentRole::Worker);
    assert_eq!(launch.spec.runtime, Runtime::Claude);
}

/// Task `id`'s test writer's `task_done` naming red, accepted by a clean `VerifyDone`,
/// then its turn's end; the red check's `Proof` op.
fn claim_red_of(fx: &mut Fixture, id: &str, window: u32) -> OpId {
    let args = serde_json::json!({"summary": "the failing test", "test": TEST, "red": &HEAD[..7]});
    let effects = fx.tool_as(AgentRole::TestWriter, window, id, "task_done", args);
    let (op, _) = only_op(&effects, "VerifyDone");
    let result = fx.clean_check(id);
    let effects = fx.done(op, result);
    fx.turn_completed(window);
    only_op(&effects, "Proof").0
}

/// The tasks a launch effect list starts a worker session for.
fn worker_launches(effects: &[Effect]) -> Vec<String> {
    (ops_in(effects, "CreateWindow").into_iter())
        .map(|(_, kind)| launch_of(kind))
        .inspect(|l| assert_eq!(role_of(l), AgentRole::Worker, "{}", l.name))
        .inspect(|l| assert_eq!(l.spec.runtime, Runtime::Claude, "{}", l.name))
        .map(|l| l.name)
        .collect()
}

/// Ruling FW-4 (I): a handed implementer that is still waiting holds no live session,
/// so it never counts against another's room. Claude is capped at 1; two paired Claude
/// tasks whose test writers ran on Codex hand over before a running pass (the run is
/// paused). On resume exactly one implementer launches, counted before the second is
/// checked; when it ends, the other launches.
#[test]
fn waiting_implementers_do_not_hold_each_other_back() {
    let tasks = [task("t1", "S", "a", PAIRED), task("t2", "S", "b", PAIRED)];
    let plan = plan_with(&profile_with("max_writers = 3"), &tasks);
    let mut fx = Fixture::new(&plan);
    fx.start_with(true, claude_capped);
    let (op, _) = fx.op("CreateRunBranch");
    fx.done(op, OpResult::Worktree { head: BASE.into() });
    let windows = fx.launch_all();
    for id in ["t1", "t2"] {
        assert_eq!(writer_slots(fx.task(id)), [Runtime::Codex], "{id}");
    }
    let proofs: Vec<OpId> = ["t1", "t2"]
        .map(|id| claim_red_of(&mut fx, id, window_of(&windows, id)))
        .into();
    edit(&mut fx, vec![PlanEdit::Pause]);
    assert_eq!(fx.run().state, proto::RunState::Paused);
    for op in proofs {
        fx.done(op, red_check(true));
    }
    for id in ["t1", "t2"] {
        let phase = fx.task(id).pair.as_ref().map(|p| p.phase);
        assert_eq!(phase, Some(PairPhase::Implementing), "{id}");
    }
    let launched = worker_launches(&resume(&mut fx));
    assert_eq!(launched, [format!("{H4}/t1.w2")], "one, in task order");
    fx.complete_windows();
    assert!(
        worker_launches(&fx.tick()).is_empty(),
        "t1 holds Claude's slot"
    );
    let launched = worker_launches(&fx.merge("t1", "m1m1m1m"));
    assert_eq!(launched, [format!("{H4}/t2.w2")], "t1 ended");
}
