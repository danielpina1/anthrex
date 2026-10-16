//! Task 7's review M2: the writer ruling (U17 read symmetrically, task 6's review) holds
//! through a reviewer's later re-pick. A reviewer never lands on the model the task's
//! failed-over test writer runs: not when the worker's own failover re-picks it (the
//! worker then waits), and not at the reviewer's launch (RL-1, rung 2), which skips that
//! model when the row has another.

use proto::models::Role;

use super::brakes::trip_claude;
use super::failover::{OPUS, SOL, SONNET, failover_line, failover_lines, route, rows};
use super::fixture::*;
use super::gates::{check_result, only_op};
use super::gates_support::reviewer;
use super::pair::{PAIRED, TEST, claim_red, red_check};
use crate::run::engine::failover::{self, Admit, Starter};
use crate::run::engine::{OpKind, OpResult};
use crate::run::model_roles::reviewer_at_launch_avoiding;
use serde_json::json;

const LUNA: &str = "codex:gpt-6-luna";

/// The test writer fails over to Luna (the reviewer row's fallback) through the real
/// flow; the reviewer row is Sol, the implementer's fallback. A worker of this task
/// gated at dispatch (a paired task queued again with its pair past writing, where
/// `pair::dispatch_route` gives the worker's own route) would fail over to Sol, and
/// `record` would move the reviewer off Sol (now the author's) onto Luna, the tests'
/// author. Asked directly, as `missing_or_failed_fallback_waits` asks: it waits.
#[test]
fn a_worker_failover_never_moves_the_reviewer_onto_the_test_writers_model() {
    let config = rows(&[
        (Role::TestWriter, SONNET, Some(LUNA)),
        (Role::ImplementerMedium, SONNET, Some(SOL)),
        (Role::Reviewer, SOL, Some(LUNA)),
    ]);
    let tasks = [task("t1", "M", "a", PAIRED)];
    let mut fx = Fixture::with_config(&plan_with(PROFILE, &tasks), config);
    trip_claude(&mut fx);
    fx.ready(true);
    let t1 = fx.task("t1");
    assert_eq!(t1.review_route, Some(route(SOL)));
    let writer_route = t1.pair.as_ref().map(|p| p.writer_route.clone());
    assert_eq!(writer_route, Some(route(LUNA)), "the writer failed over");
    assert_eq!(failover_lines(&fx), [failover_line("t1", LUNA)]);
    fx.complete_prepares();
    let writer = fx.complete_windows()[0].1;
    let (op, _) = claim_red(&mut fx, writer, &HEAD[..7]);
    fx.done(op, red_check(true));
    let i = (fx.run().tasks.iter())
        .position(|t| t.id() == "t1")
        .unwrap();
    let admitted = failover::admit(
        &fx.state.gate,
        fx.run(),
        Starter::Worker(i),
        Role::ImplementerMedium,
        &route(SONNET),
    );
    assert_eq!(admitted, Admit::Wait);
    // With no failed-over writer, the same worker fails over (its reviewer re-picked
    // onto Luna is nobody's model then).
    fx.run_mut().failovers.clear();
    let admitted = failover::admit(
        &fx.state.gate,
        fx.run(),
        Starter::Worker(i),
        Role::ImplementerMedium,
        &route(SONNET),
    );
    assert!(matches!(admitted, Admit::FailOver(_)), "{admitted:?}");
}

/// RL-1 at launch: the reviewer row's model failed in this task, and its fallback is
/// the failed-over test writer's model: the reviewer keeps off it.
#[test]
fn a_reviewer_at_launch_skips_the_test_writers_model() {
    let config = rows(&[(Role::Reviewer, OPUS, Some(LUNA))]);
    let tasks = [task("t1", "M", "a", PAIRED)];
    let mut fx = Fixture::with_config(&plan_with(PROFILE, &tasks), config);
    fx.ready(true);
    let models = fx.run().limits.models();
    let author = route(SONNET);
    let failed = [route(OPUS)];
    assert_eq!(
        reviewer_at_launch_avoiding(models, &author, &failed, None).0,
        route(LUNA)
    );
    let writer = route(LUNA);
    let (picked, _) = reviewer_at_launch_avoiding(models, &author, &failed, Some(&writer));
    assert!(!models.same_model(&picked, &writer), "{picked:?}");
    // No other model on the row: the row's own route, never the writer's.
    assert_eq!(picked, models.route(Role::Reviewer));
}

/// The launch itself (`review.rs`): Claude braked, the test writer failed over to Luna;
/// the reviewer row's Opus failed in this task (RL-1), so its other model would be
/// Luna, the tests' author. The reviewer that starts is not on Luna.
#[test]
fn the_reviewer_launched_keeps_off_the_failed_over_writer() {
    let config = rows(&[
        (Role::TestWriter, SONNET, Some(LUNA)),
        (Role::ImplementerMedium, SONNET, None),
        (Role::Reviewer, OPUS, Some(LUNA)),
    ]);
    let tasks = [task("t1", "M", "a", PAIRED)];
    let mut fx = Fixture::with_config(&plan_with(PROFILE, &tasks), config);
    trip_claude(&mut fx);
    fx.ready(true);
    assert_eq!(failover_lines(&fx), [failover_line("t1", LUNA)]);
    fx.complete_prepares();
    let writer = fx.complete_windows()[0].1;
    let (op, _) = claim_red(&mut fx, writer, &HEAD[..7]);
    fx.done(op, red_check(true));
    let worker = fx.complete_windows()[0].1;
    // Ruling RL-1: a reviewer round on Opus failed in this task for an environment
    // reason, as `missing_or_failed_fallback_waits` records one.
    let mut failed = fx.task("t1").rounds[0].clone();
    failed.role = proto::AgentRole::Reviewer;
    failed.route = route(OPUS);
    failed.environment_failed = true;
    fx.task_mut("t1").rounds.push(failed);
    // The implementer's done, its proof, then the review (`pair_implementer.rs`' path).
    let effects = fx.tool(worker, "task_done", json!({"summary": "made it pass"}));
    let (op, _) = only_op(&effects, "VerifyDone");
    let mut done = fx.clean_check("t1");
    if let OpResult::DoneChecked { head, .. } = &mut done {
        *head = "e2e2e2e2e2e2e2e2e2e2e2e2e2e2e2e2e2e2e2e2".into();
    }
    fx.done(op, done);
    fx.turn_completed(worker);
    let (op, _) = fx.op("Proof");
    let passed = OpResult::Proof {
        red_failed: true,
        head_passed: true,
        matched: true,
        red_tail: String::new(),
        head_tail: format!("test {TEST} ... ok"),
    };
    let effects = fx.done(op, passed);
    let (op, _) = only_op(&effects, "Check");
    let effects = fx.done(op, check_result(true));
    let (op, _) = only_op(&effects, "PrepareReview");
    let (_, kind) = reviewer(&mut fx, op, "");
    let OpKind::CreateWindow { spec, .. } = kind else {
        unreachable!()
    };
    assert_ne!(
        (spec.runtime, spec.model.as_str()),
        (proto::Runtime::Codex, "gpt-6-luna"),
        "the reviewer started on the test writer's model"
    );
}
