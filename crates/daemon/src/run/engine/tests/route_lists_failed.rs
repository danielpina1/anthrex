//! Milestone 9.5 task 10b, ruling RL-1 in the reducer: a session whose own failed turn
//! blocks its task `blocked(environment)` marks its route failed in this task, and
//! `run retry` (and the next reviewer) take another route.

use proto::{AgentRole, BlockReason, ModelEntry, Runtime, Strength, TaskState};

use super::control::retry;
use super::fixture::*;
use super::gates::{CHECK_MODE, only_op, working_with};
use super::gates_review::{in_review, reviewer};
use super::turns::killed_exit;
use crate::headless::FailureKind;
use crate::run::engine::{OpKind, OpResult, TurnOutcome};
use crate::run::model::ReviewLevel;
use crate::run::model_roles::FAILED_IN_TASK;
use crate::run::roster::{pick_reviewer, pick_reviewer_skipping};

/// The default roster with two more Codex rows, so a reviewer has another to go to.
fn config() -> config::Orchestrator {
    let mut config = config::Orchestrator::default();
    for (model, strength) in [
        ("gpt-6-luna", Strength::Fast),
        ("gpt-6.1-sol", Strength::Frontier),
    ] {
        config.models.push(ModelEntry {
            runtime: Runtime::Codex,
            model: model.into(),
            strength,
            note: String::new(),
        });
    }
    config
}

#[test]
#[ignore = "M9.8.7a: tasks take their role-table rows, so no model list picks a worker, reviewer or rung-2 step; deleted with route_pick.rs in M9.8.13"]
fn retry_reroutes_a_reviewer_that_failed_in_this_task() {
    let (mut fx, window) = working_with(PROFILE, CHECK_MODE, config());
    let (op, _) = in_review(&mut fx, window);
    let (rwindow, _) = reviewer(&mut fx, op, "diff --git a/x b/x");
    let author = fx.task("t1").route.clone();
    let level = fx.task("t1").review_level.unwrap_or(ReviewLevel::Medium);
    let first = fx
        .task("t1")
        .rounds
        .last()
        .expect("the reviewer")
        .route
        .clone();
    assert_eq!(first, pick_reviewer(&fx.run().roster, &author, level));

    // Ruling F-1: the reviewer's client error blocks the task on its environment.
    let failed = TurnOutcome::Failed {
        error: "model not found".into(),
        kind: FailureKind::ClientError,
    };
    fx.turn_ended(rwindow, failed);
    let t1 = fx.task("t1");
    assert_eq!(t1.state, TaskState::Blocked);
    assert_eq!(
        t1.block.as_ref().map(|b| b.reason),
        Some(BlockReason::Environment)
    );
    let round = t1.rounds.last().expect("the reviewer's round");
    assert!(round.role == AgentRole::Reviewer && round.environment_failed);

    // `run retry`: a fresh worker session; its next reviewer skips the failed route.
    let effects = retry(&mut fx, "t1");
    let effects = if ops_in(&effects, "DiffSoFar").is_empty() {
        killed_exit(&mut fx, window)
    } else {
        effects
    };
    let (op, _) = only_op(&effects, "DiffSoFar");
    let effects = fx.done(
        op,
        OpResult::Diff {
            stat: String::new(),
            patch: String::new(),
        },
    );
    let (_, kind) = only_op(&effects, "CreateWindow");
    assert!(matches!(kind, OpKind::CreateWindow { .. }));
    let worker = fx.complete_windows()[0].1;
    let author = fx.task("t1").route.clone();
    let (op, _) = in_review(&mut fx, worker);
    let (_, kind) = reviewer(&mut fx, op, "diff --git a/x b/x");
    let second = fx
        .task("t1")
        .rounds
        .last()
        .expect("the new reviewer")
        .route
        .clone();
    assert_ne!(
        (second.runtime, &second.model),
        (first.runtime, &first.model)
    );
    let roster = fx.run().roster.clone();
    let expected = pick_reviewer_skipping(&roster, &author, level, std::slice::from_ref(&first));
    assert_eq!(second, expected);
    assert!(format!("{kind:?}").contains(&second.model), "{kind:?}");
    let d = fx
        .task("t1")
        .routing_decisions
        .last()
        .expect("the reviewer's")
        .clone();
    let skipped = (d.candidates.iter())
        .find(|c| c.route.runtime == first.runtime && c.route.model == first.model)
        .and_then(|c| c.skipped_reason.as_deref());
    assert_eq!(skipped, Some(FAILED_IN_TASK));
}

/// Milestone 9.8: the default config, its `implementer.small` row (Sonnet at `low`)
/// falling back to Codex's default.
fn small_falls_back_to_codex() -> config::Orchestrator {
    let mut config = config::Orchestrator::default();
    let row = proto::models::RoleChoice {
        model: proto::models::ModelRef::parse("claude:claude-sonnet-5").unwrap(),
        effort: Some("low".into()),
        fallback: Some(proto::models::ModelRef::parse("codex:default").unwrap()),
    };
    (config.roles.rows).insert(proto::models::Role::ImplementerSmall, row);
    config
}

/// The action menu's retry preview names the route a retry takes: here a worker whose
/// own route failed with a client error, so no higher effort on that model, but its
/// row's fallback, Codex's default, at its default effort (milestone 9.8 decision 29).
#[test]
fn the_retry_preview_names_the_route_a_retry_takes() {
    use crate::run::engine::actions::{self, ActionNode};
    let (mut fx, window) = working_with(PROFILE, CHECK_MODE, small_falls_back_to_codex());
    let failed = TurnOutcome::Failed {
        error: "model not found".into(),
        kind: FailureKind::ClientError,
    };
    fx.turn_ended(window, failed);
    assert_eq!(fx.task("t1").state, TaskState::Blocked);
    assert!(fx.task("t1").rounds[0].environment_failed);
    let preview = actions::available(fx.run(), &ActionNode::Task("t1"))
        .into_iter()
        .find(|a| a.kind == proto::ActionKind::Retry)
        .expect("retry is listed")
        .effect;
    assert_eq!(
        preview,
        "retry t1: a fresh session at rung 2 on codex default (default effort)"
    );
    retry(&mut fx, "t1");
    let route = &fx.task("t1").route;
    assert_eq!((route.runtime, route.model.as_str()), (Runtime::Codex, ""));
}

/// Ruling RL-4 through `run retry`: a review task whose reviewer failed in this task
/// takes its `review` list's next candidate, and that review records the list.
#[test]
#[ignore = "M9.8.7a: tasks take their role-table rows, so no model list picks a worker, reviewer or rung-2 step; deleted with route_pick.rs in M9.8.13"]
fn a_retried_review_task_takes_and_records_its_lists_next_candidate() {
    use super::kinds::{review, reviewer_window};
    use config::{Candidate, Pick, RouteList, RouteLists};
    use proto::Effort;
    let cand = |runtime, model: &str, effort| Candidate {
        runtime,
        model: model.into(),
        effort,
    };
    let lists = RouteLists {
        review: RouteList {
            candidates: vec![
                cand(Runtime::Codex, "gpt-6.1-sol", None),
                cand(Runtime::Claude, "claude-opus-5-5", Some(Effort::HIGH)),
            ],
            pick: Pick::First,
        },
        ..Default::default()
    };
    let mut fx = Fixture::with_config(&plan_with(PROFILE, &[review("v1", "S")]), config());
    fx.tuning = crate::run::refit::Tuned {
        lists,
        ..Default::default()
    };
    fx.ready(true);
    assert_eq!(fx.task("v1").route.model, "gpt-6.1-sol");
    let window = reviewer_window(&mut fx, "v1", "diff --git a/x b/x");
    let failed = TurnOutcome::Failed {
        error: "model not found".into(),
        kind: FailureKind::ClientError,
    };
    fx.turn_ended(window, failed);
    let v1 = fx.task("v1");
    assert_eq!(v1.state, TaskState::Blocked);
    assert!(v1.rounds.last().expect("the reviewer").environment_failed);

    retry(&mut fx, "v1");
    reviewer_window(&mut fx, "v1", "diff --git a/x b/x");
    let v1 = fx.task("v1");
    assert_eq!(
        (v1.route.model.as_str(), v1.route.effort.clone()),
        ("claude-opus-5-5", Effort::HIGH)
    );
    let d = v1.routing_decisions.last().expect("the second review's");
    assert_eq!(
        (d.source.as_str(), d.selected_index),
        ("configured_list", 1)
    );
    assert_eq!(d.chosen, v1.route);
}
