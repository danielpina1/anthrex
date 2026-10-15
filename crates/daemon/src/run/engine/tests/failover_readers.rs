//! Milestone 9.16 task M9.16.6, the readers' half of cross-runtime failover: research
//! tasks, sub-planners and run scouts fail over on their rows; the reviewer rule
//! (decision 40) for review tasks, a failed-over worker's reviewer and the document
//! reviewer; the routing records (decision 42); the orchestrator never fails over
//! (decision 36). The helpers are `failover.rs`'s.

use proto::models::{Role, RoleChoice};
use proto::{AgentRole, Runtime, TaskOutcome, TaskState};
use serde_json::Value;

use super::brakes::trip_claude;
use super::design_agents::launches;
use super::design_review_fixture::*;
use super::failover::{HAIKU, OPUS, SOL, SONNET, failover_line, failover_lines, route, rows, trip};
use super::fixture::*;
use super::gates::CHECK_MODE;
use super::gates_support::{in_review, reviewer};
use super::orch::answer;
use super::planners::spawn;
use super::run_scouts::scout;
use crate::run::design::state::DesignAgentState;
use crate::run::engine::OpKind;
use crate::run::engine::failover::QUOTA_FAILOVER;
use crate::run::model::Run;

/// `run`'s frozen `role` row set to `model` with `fallback`.
fn set_row(run: &mut Run, role: Role, model: &str, fallback: Option<&str>) {
    let parse = |m: &str| proto::models::ModelRef::parse(m).expect("a model");
    let models = run.limits.models.as_mut().expect("frozen at start");
    let row = RoleChoice {
        model: parse(model),
        effort: None,
        fallback: fallback.map(parse),
    };
    models.rows.insert(role, row);
}

#[test]
fn readers_planners_and_scouts_fail_over() {
    // A research task on its `research` row.
    let config = rows(&[(Role::Research, HAIKU, Some(SOL))]);
    let plan = plan_with(PROFILE, &[super::kinds::research("r1", "")]);
    let mut fx = Fixture::with_config(&plan, config);
    trip_claude(&mut fx);
    fx.ready(true);
    fx.tick();
    assert_eq!(fx.task("r1").state, TaskState::Working);
    assert_eq!(fx.task("r1").route, route(SOL));
    let (_, kind) = fx.op("CreateWindow");
    let OpKind::CreateWindow { spec, .. } = kind else {
        unreachable!()
    };
    assert_eq!(spec.runtime, Runtime::Codex);
    assert_eq!(failover_lines(&fx), [failover_line("r1", SOL)]);

    // A sub-planner on the `planner` row and a run scout on the `research` row. The
    // orchestrator runs on Codex, so the run's start vetted Codex (task 6 review I1).
    let mut fx = super::orch::launched(false);
    (fx.run_mut().orch.orchestrator.as_mut().unwrap()).route = route(SOL);
    set_row(fx.run_mut(), Role::Planner, OPUS, Some(SOL));
    set_row(fx.run_mut(), Role::Research, HAIKU, Some(SOL));
    trip_claude(&mut fx);
    assert_eq!(spawn(&mut fx, "mail")["state"], "planning");
    let epic = &fx.run().orch.epics[0];
    assert_eq!(epic.route, route(SOL));
    let (_, kind) = fx.op("StartPlanner");
    let OpKind::StartPlanner { spec } = kind else {
        unreachable!()
    };
    assert_eq!(spec.route, route(SOL));
    let record = (fx.run().role_routing_decisions.iter())
        .find(|d| d.role == AgentRole::Planner)
        .expect("the sub-planner's record");
    assert_eq!(
        (&record.chosen, record.source.as_str(), &record.failover),
        (&route(SOL), QUOTA_FAILOVER, &Some(route(OPUS)))
    );
    let (ok, value) = answer(&scout(&mut fx, "s1"));
    assert!(ok, "{value}");
    assert_eq!(value["state"], "starting");
    let id = fx.run().orch.run_scouts[0].id.clone();
    let run = fx.run();
    assert_eq!(
        crate::run::orch::launch::run_scout_route(run, &id),
        route(SOL)
    );
    let record = crate::run::orch::roles::scout_record(run, &id, fx.now);
    assert_eq!(
        (&record.chosen, record.source.as_str(), &record.failover),
        (&route(SOL), QUOTA_FAILOVER, &Some(route(HAIKU)))
    );
    let who = format!("scout {id}");
    assert_eq!(
        failover_lines(&fx),
        [failover_line("planner mail", SOL), failover_line(&who, SOL)]
    );
}

/// A review task `r1` waiting on `t1`, whose worker ran on `small`'s row; Claude
/// tripped once `t1` launched, then `t1` merged.
fn review_after(small: &str) -> Fixture {
    let config = rows(&[
        (Role::ImplementerSmall, small, None),
        (Role::Reviewer, OPUS, Some(SOL)),
    ]);
    let r1 = task_toml(
        "r1",
        "S",
        "[]",
        "kind = \"review\"\nreview_target = \"main..feature\"\ndeps = [\"t1\"]",
    );
    let plan = plan_with(PROFILE, &[task("t1", "S", "b", ""), r1]);
    let mut fx = Fixture::with_config(&plan, config);
    fx.ready(true);
    fx.launch_all();
    trip_claude(&mut fx);
    fx.merge("t1", HEAD);
    fx
}

#[test]
fn reviewer_never_gets_author_model_via_failover() {
    // (1) A merged task's writing round ran the reviewer row's fallback: it waits.
    let fx = review_after(SOL);
    assert_eq!(fx.task("r1").state, TaskState::Queued);
    assert_eq!(fx.task("r1").route, route(OPUS));
    assert!(failover_lines(&fx).is_empty());
    // (2) Written on Sonnet instead: it fails over.
    let fx = review_after(SONNET);
    assert_eq!(fx.task("r1").route, route(SOL));
    assert_eq!(fx.task("r1").state, TaskState::Review);
    assert_eq!(failover_lines(&fx), [failover_line("r1", SOL)]);

    // (3) A worker that failed over to the fallback is reviewed off the author's model,
    // on braked Claude: reviewers of dispatched tasks are not gated.
    let config = rows(&[(Role::Reviewer, SOL, Some(OPUS))]);
    let plan = plan_with(PROFILE, &[task("t1", "M", "a", CHECK_MODE)]);
    let mut fx = Fixture::with_config(&plan, config);
    trip_claude(&mut fx);
    fx.ready(true);
    assert_eq!(fx.task("t1").route, route(SOL));
    let window = fx.launch_all()[0].1;
    let (op, _) = in_review(&mut fx, window);
    let (_, kind) = reviewer(&mut fx, op, "diff --git a/x b/x");
    let OpKind::CreateWindow { spec, .. } = kind else {
        unreachable!()
    };
    assert_eq!(
        (spec.runtime, spec.model.as_str()),
        (Runtime::Claude, "claude-opus-5-5")
    );
    assert_eq!(fx.run().failovers.len(), 1, "the worker's only");

    // (4) A document reviewer whose fallback is the orchestrator's model waits.
    let mut fx = specifying();
    set_row(fx.run_mut(), Role::Reviewer, SONNET, Some(SOL));
    (fx.run_mut().orch.orchestrator.as_mut().unwrap()).route = route(SOL);
    trip_claude(&mut fx);
    let before = launches(&fx).len();
    outcome(&submit_spec(&mut fx, false, Value::Null)).unwrap();
    let agent = design(&fx).reviewer.clone().expect("queued");
    assert_eq!(
        (agent.state, agent.route),
        (DesignAgentState::Queued, route(SONNET))
    );
    assert_eq!(launches(&fx).len(), before);
    assert!(failover_lines(&fx).is_empty());
}

#[test]
fn a_document_reviewer_fails_over_off_the_authors_model() {
    let mut fx = specifying();
    set_row(fx.run_mut(), Role::Reviewer, SONNET, Some(SOL));
    let orchestrator = fx.run().orch.orchestrator.as_ref().unwrap().route.clone();
    assert_eq!(orchestrator.runtime, Runtime::Claude);
    trip_claude(&mut fx);
    outcome(&submit_spec(&mut fx, false, Value::Null)).unwrap();
    let (_, spec) = launches(&fx).last().cloned().unwrap();
    assert_eq!(spec.kind.label(), "spec-r1");
    assert_eq!(spec.route, route(SOL));
    let record = (fx.run().role_routing_decisions.iter())
        .rfind(|d| d.role == AgentRole::DocReviewer)
        .expect("its record");
    assert_eq!(
        (&record.chosen, record.source.as_str(), &record.failover),
        (&route(SOL), QUOTA_FAILOVER, &Some(route(SONNET)))
    );
    assert_eq!(
        failover_lines(&fx),
        [failover_line("doc review spec-r1", SOL)]
    );
}

#[test]
fn failover_recorded_in_history() {
    let plan = plan_with(PROFILE, &[task("t3", "M", "a", "")]);
    let mut fx = Fixture::with_config(&plan, rows(&[]));
    trip_claude(&mut fx);
    fx.ready(true);
    fx.launch_all();
    fx.merge("t3", HEAD);
    let run = fx.run();
    let task = run.task("t3").unwrap();
    let record = crate::run::history::task_record(run, task, TaskOutcome::Merged, fx.now);
    let d = (record.routing_decisions.iter())
        .find(|d| d.role == AgentRole::Worker)
        .expect("the worker's record");
    assert_eq!(d.chosen, route(SOL));
    assert_eq!(d.failover, Some(route(SONNET)));
    assert_eq!(d.source, QUOTA_FAILOVER);
    let selected = d.selected_index as usize;
    assert_eq!(d.candidates[selected].route, route(SOL));
    let own = (d.candidates.iter()).position(|c| c.route == route(SONNET));
    assert!(own.is_some_and(|k| k < selected), "{:?}", d.candidates);
}

/// Pinning (decision 36): the orchestrator's launch and relaunch are not gated, so a
/// relaunch due while its runtime is braked runs on its own route.
#[test]
fn orchestrator_never_fails_over() {
    let mut fx = super::orch::launched(false);
    set_row(fx.run_mut(), Role::Orchestrator, OPUS, Some(SOL));
    let own = fx.run().orch.orchestrator.as_ref().unwrap().route.clone();
    super::orch_restore::restart(&mut fx);
    trip(&mut fx, own.runtime);
    let effects = super::orch_restore::resume(&mut fx);
    assert_eq!(
        ops_in(&effects, "RestartOrchestrator").len(),
        1,
        "{effects:#?}"
    );
    assert_eq!(fx.run().orch.orchestrator.as_ref().unwrap().route, own);
    assert!(fx.run().failovers.is_empty());
    assert!(failover_lines(&fx).is_empty());
}

/// `run`'s frozen role table on Claude alone, no row falling back anywhere, the
/// `brainstorm` pair included.
fn claude_only(run: &mut Run) {
    let roles: Vec<Role> = (run.limits.models.as_ref().expect("frozen").rows.keys())
        .copied()
        .collect();
    for role in roles {
        set_row(run, role, OPUS, None);
    }
    let models = run.limits.models.as_mut().expect("frozen at start");
    models.brainstorm.first = proto::models::ModelRef::parse(OPUS).expect("a model");
    models.brainstorm.second = proto::models::ModelRef::parse(HAIKU).expect("a model");
}

/// Task 6 review I1: a fallback on a runtime the run's start never vetted (decisions 50
/// and 53 check only `reach::reachable_runtimes`) is not taken. In a Claude-only run with
/// Claude braked, a sub-planner, a run scout and a document reviewer whose rows fall back
/// to Codex wait.
#[test]
fn a_fallback_the_start_never_vetted_waits() {
    let mut fx = super::orch::launched(false);
    claude_only(fx.run_mut());
    set_row(fx.run_mut(), Role::Planner, OPUS, Some(SOL));
    set_row(fx.run_mut(), Role::Research, HAIKU, Some(SOL));
    let reach = crate::run::reach::reachable_runtimes(fx.run());
    assert_eq!(reach, [Runtime::Claude]);
    trip_claude(&mut fx);
    assert_eq!(spawn(&mut fx, "mail")["state"], "queued");
    assert_eq!(fx.run().orch.epics[0].route, route(OPUS));
    let (ok, value) = answer(&scout(&mut fx, "s1"));
    assert!(ok, "{value}");
    assert_ne!(value["state"], "starting", "{value}");
    assert!(ops_in(&fx.log, "StartPlanner").is_empty());
    assert!(ops_in(&fx.log, "StartScout").is_empty());
    assert!(fx.run().failovers.is_empty());
    assert!(failover_lines(&fx).is_empty());

    let mut fx = specifying();
    claude_only(fx.run_mut());
    set_row(fx.run_mut(), Role::Reviewer, SONNET, Some(SOL));
    let reach = crate::run::reach::reachable_runtimes(fx.run());
    assert_eq!(reach, [Runtime::Claude]);
    trip_claude(&mut fx);
    let before = launches(&fx).len();
    outcome(&submit_spec(&mut fx, false, Value::Null)).unwrap();
    let agent = design(&fx).reviewer.clone().expect("queued");
    assert_eq!(
        (agent.state, agent.route),
        (DesignAgentState::Queued, route(SONNET))
    );
    assert_eq!(launches(&fx).len(), before);
    assert!(failover_lines(&fx).is_empty());
}
