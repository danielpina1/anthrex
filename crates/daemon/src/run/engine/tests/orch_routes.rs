//! Milestone 9.8 decision 31 (task M9.8.9, fix round 1): the engine side of an ignored
//! route. A batch from the orchestrator (`edit_plan`) or a sub-planner (`submit_epic`)
//! whose tasks name routes logs `ROUTE_IGNORED` once, whatever the route's values; a
//! user's edit never does.

use serde_json::{Value, json};

use super::dispatch::{edit, replies};
use super::fixture::*;
use super::orch::{add, answer, edit_plan, launched};
use super::planners::{planning_mail, submit_epic, task_in};
use crate::run::model_roles::RunModels;
use crate::run::orch::contract::ROUTE_IGNORED;

/// The run log's `ROUTE_IGNORED` lines.
fn ignored(fx: &Fixture) -> usize {
    (fx.run().log.iter())
        .filter(|l| l.text == ROUTE_IGNORED)
        .count()
}

/// [`add`] of `id` in `module`, sending `route`.
fn routed(id: &str, module: &str, route: Value) -> Value {
    let mut edit = add(id, module);
    edit["task"]["route"] = route;
    edit
}

/// Task `id` runs on its row's route.
fn on_its_row(fx: &Fixture, id: &str) {
    let t = fx.task(id);
    assert_eq!(t.spec.route, proto::RouteSpec::default(), "{id}");
    let row = fx.run().limits.models().route(RunModels::task_role(t));
    assert_eq!(t.route, row, "{id}");
}

#[test]
fn an_orchestrators_routes_are_logged_once_per_batch() {
    let mut fx = launched(false);
    let codex = json!({"runtime": "codex", "model": "gpt-6-sol"});
    let effects = edit_plan(
        &mut fx,
        json!({"edits": [
            routed("t1", "a", codex.clone()),
            routed("t2", "b", json!({"effort": "high"})),
            routed("t3", "c", codex.clone()),
        ]}),
    );
    assert!(answer(&effects).0, "{effects:#?}");
    assert_eq!(ignored(&fx), 1);
    for id in ["t1", "t2", "t3"] {
        on_its_row(&fx, id);
    }
    // A later batch is noted again; one that names no route is not.
    assert!(
        answer(&edit_plan(
            &mut fx,
            json!({"edits": [routed("t4", "d", codex)]})
        ))
        .0
    );
    assert_eq!(ignored(&fx), 2);
    assert!(answer(&edit_plan(&mut fx, json!({"edits": [add("t5", "e")]}))).0);
    assert_eq!(ignored(&fx), 2);
}

#[test]
fn a_sub_planners_routes_are_logged_on_submit_epic() {
    let mut fx = planning_mail(true);
    let before = ignored(&fx);
    let mut t2 = task_in("t2", "mail");
    t2["task"]["route"] = json!({"runtime": "codex", "model": "gpt-6-sol"});
    let mut t3 = task_in("t3", "mail");
    t3["task"]["route"] = json!({"effort": "low"});
    let effects = submit_epic(&mut fx, json!([t2, t3]));
    assert!(replies(&effects)[0].is_ok(), "{effects:#?}");
    assert!(fx.run().task("t3").is_some(), "the epic was accepted");
    assert_eq!(ignored(&fx), before + 1);
    on_its_row(&fx, "t2");
    on_its_row(&fx, "t3");
}

#[test]
fn a_users_edit_never_logs_it() {
    let mut fx = Fixture::new(&plan_with(PROFILE, &[task("t1", "S", "a", "")]));
    fx.start(false);
    let mut spec = crate::run::test_support::spec_of(&task("t2", "S", "b", ""));
    spec.route = proto::RouteSpec {
        model: Some("claude-opus-5-5".into()),
        ..Default::default()
    };
    let effects = edit(
        &mut fx,
        vec![proto::PlanEdit::AddTask { task: spec.clone() }],
    );
    assert!(replies(&effects)[0].is_ok(), "{effects:#?}");
    assert_eq!(ignored(&fx), 0);
    assert_eq!(fx.task("t2").spec.route, spec.route);
    assert_eq!(fx.task("t2").route.model, "claude-opus-5-5");
}

/// Fix round 1 (controller ruling): an ignored route's values never refuse the batch,
/// even ones no route could hold: an unknown runtime, strength or effort.
#[test]
fn an_orchestrators_unknown_route_values_are_ignored_not_refused() {
    let mut fx = launched(false);
    let route = json!({"runtime": "openai", "strength": "ultra", "effort": "xhigh",
                       "model": "o9"});
    let effects = edit_plan(
        &mut fx,
        json!({"edits": [routed("t1", "a", route.clone())]}),
    );
    assert!(answer(&effects).0, "{effects:#?}");
    assert_eq!(ignored(&fx), 1);
    on_its_row(&fx, "t1");
    // An amend of it too, and a split's child.
    let amend = json!({"op": "amend_task", "task_id": "t1", "route": route.clone()});
    assert!(answer(&edit_plan(&mut fx, json!({"edits": [amend]}))).0);
    let mut child = add("t2", "a");
    child["task"]["route"] = route;
    let split = json!({"op": "split_task", "task_id": "t1", "into": [child["task"].clone()]});
    assert!(answer(&edit_plan(&mut fx, json!({"edits": [split]}))).0);
    assert_eq!(ignored(&fx), 3);
    on_its_row(&fx, "t2");
}

/// The length guard (M9.11 review finding 3) stays: an over-long model is refused.
#[test]
fn an_ignored_routes_model_is_still_bounded() {
    let mut fx = launched(false);
    let route = json!({"runtime": "openai", "model": "m".repeat(101)});
    let effects = edit_plan(&mut fx, json!({"edits": [routed("t1", "a", route)]}));
    let (ok, value) = answer(&effects);
    assert!(!ok, "{value}");
    assert_eq!(ignored(&fx), 0);
}
