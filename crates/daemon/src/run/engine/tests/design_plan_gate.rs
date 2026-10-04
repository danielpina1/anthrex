//! Milestone 9.6 task M9.6.7, fix round 1: the plan gate of a design run. The action
//! menu's approve agrees with the handler while the orchestrator revises (ruling
//! T7-3); a plan the orchestrator changes at the open gate plans again, so the user
//! approves exactly the version the gate shows (ruling T7-4); and the plan tools'
//! admission by phase (ruling T1-O4, review m4).

use proto::{ActionKind, DocGateAction, DocGateKind, DocKind, RunState};
use serde_json::json;

use super::design_fixture::*;
use super::dispatch::replies;
use super::fixture::*;
use super::orch::{ORCH, orch_tool};
use super::planners::{PLANNER, planner_started, spawn, submit_epic};
use crate::run::engine::actions::{self, ActionNode};

fn plan_version(fx: &Fixture, n: u32) -> String {
    let design = fx.run().orch.design.as_ref().unwrap();
    let version = design.find(DocKind::Plan, Some(n)).unwrap();
    version.reason.clone()
}

fn plan_submitted(fx: &Fixture) -> bool {
    fx.run().orch.orchestrator.as_ref().unwrap().plan_submitted
}

/// Ruling T7-3: at a plan gate the orchestrator revises, the menu's approve is refused
/// with the handler's own text (milestone 9.0.6's invariant).
#[test]
fn approve_at_a_revising_plan_gate_is_refused_by_its_check_too() {
    let mut fx = at_plan_gate(false);
    let changes = DocGateAction::Changes {
        note: "Split t1.".into(),
        review: false,
    };
    act(&mut fx, DocGateKind::Plan, changes).unwrap();
    let text = "the orchestrator is revising plan v1; wait for it".to_string();
    let check = actions::check(fx.run(), &ActionNode::Run, &ActionKind::Approve);
    assert_eq!(check, Err(text.clone()));
    assert_eq!(replies(&fx.approve()), vec![Err(text)]);
}

/// Ruling T7-4: a sub-planner started at the open plan gate returns the run to
/// planning, closes the gate and starts the planning clock; the next submit opens the
/// plan's v2, and the user's approve is exactly that version's.
#[test]
fn a_sub_planner_at_the_plan_gate_makes_the_resubmit_a_new_version() {
    let mut fx = at_plan_gate(false);
    assert_eq!(gate(&fx), Some((DocGateKind::Plan, 1, None)));
    spawn(&mut fx, "mail");
    assert_eq!(fx.run().state, RunState::Planning);
    assert_eq!(gate(&fx), None);
    assert!(!plan_submitted(&fx));
    let started = fx.run().orch.design.as_ref().unwrap().phase_started;
    assert_eq!(started, Some(fx.now), "the planning clock runs");
    planner_started(&mut fx, PLANNER);
    let effects = submit_epic(&mut fx, json!([add_task("mail")]));
    assert!(replies(&effects)[0].is_ok(), "{effects:?}");
    let effects = orch_tool(&mut fx, ORCH, "edit_plan", json!({"submit": true}));
    assert!(replies(&effects)[0].is_ok(), "{effects:?}");
    assert_eq!(gate(&fx), Some((DocGateKind::Plan, 2, None)));
    assert_eq!(plan_version(&fx, 2), "submitted");
    assert_eq!(
        replies(&fx.approve()),
        vec![Ok(format!("run {RUN_ID} approved"))]
    );
    assert!(notes(&fx).contains(&"the user approved the plan v2".to_string()));
    assert_eq!(fx.run().state, RunState::Running);
}

/// Ruling T7-4: the orchestrator's `edit_plan` edits at the open plan gate plan again
/// (logged), and its submit in the same call opens v2; a submit alone changes nothing.
#[test]
fn orchestrator_edits_at_the_open_plan_gate_plan_again() {
    let mut fx = at_plan_gate(false);
    let effects = orch_tool(&mut fx, ORCH, "edit_plan", json!({"submit": true}));
    assert!(replies(&effects)[0].is_ok());
    assert_eq!(gate(&fx), Some((DocGateKind::Plan, 1, None)));
    let args = json!({"edits": [add_task("t2")]});
    assert!(replies(&orch_tool(&mut fx, ORCH, "edit_plan", args))[0].is_ok());
    assert_eq!(fx.run().state, RunState::Planning);
    assert_eq!(gate(&fx), None);
    let line = "planning again: the orchestrator changed the plan at the gate".to_string();
    assert!(log_lines(&fx).contains(&line), "{:?}", log_lines(&fx));
    let args = json!({"edits": [add_task("t3")], "submit": true});
    assert!(replies(&orch_tool(&mut fx, ORCH, "edit_plan", args))[0].is_ok());
    assert_eq!(gate(&fx), Some((DocGateKind::Plan, 2, None)));
    let reply = fx.approve();
    assert_eq!(replies(&reply), vec![Ok(format!("run {RUN_ID} approved"))]);
    assert!(notes(&fx).contains(&"the user approved the plan v2".to_string()));
}

/// Review m4 (ruling T1-O4): `edit_plan` in specifying is refused exactly, and
/// `submit_epic` is never the orchestrator's (its role refuses it before any phase
/// check; a sub-planner exists only from planning on).
#[test]
fn plan_tools_before_planning_are_refused() {
    let mut fx = at_brainstorm_gate(false);
    act(&mut fx, DocGateKind::Brainstorm, DocGateAction::Approve).unwrap();
    assert_eq!(fx.run().state, RunState::Specifying);
    let args = json!({"edits": [add_task("t1")]});
    let effects = orch_tool(&mut fx, ORCH, "edit_plan", args);
    assert_eq!(
        refused(&effects),
        "edit_plan is for the planning phase; this run is specifying"
    );
    assert!(fx.run().task("t1").is_none());
    let args = json!({"edits": [add_task("t1")]});
    let effects = orch_tool(&mut fx, ORCH, "submit_epic", args);
    assert_eq!(
        refused(&effects),
        "tool submit_epic is not available to the orchestrator role"
    );
}
