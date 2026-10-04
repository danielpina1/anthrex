//! The final fix wave's pair items in the reducer: a test writer's writer slot and cap
//! are its own runtime's (A-I3); its rung 2 is not the task's escalation (review B's
//! M2); the retry preview names its route (M3); the snapshot's writer failures (M7).

use proto::{Runtime, TaskState};

use super::*;
use crate::run::engine::concurrency::writers_busy_on;
use crate::run::engine::schedule::writer_slots;
use crate::run::model::RuntimeConcurrency;

/// Codex's cap brought down to 1 by a rate limit at the fixture's start.
fn codex_capped(run: &mut crate::run::model::Run) {
    let held = RuntimeConcurrency {
        cap: 1,
        last_rate_limit_at: Some(2_000),
        ..RuntimeConcurrency::new(run.limits.max_writers)
    };
    run.concurrency.insert("codex".into(), held);
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
    let next = crate::run::route_pick::writer_step(fx.run(), i, &current);
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
    let effort = format!("{:?}", next.effort).to_lowercase();
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
