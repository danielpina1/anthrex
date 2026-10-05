//! Milestone 9.6 task M9.6.7, fix rounds 1 and 2: the plan gate of a design run. The
//! action menu's approve agrees with the handler while the orchestrator revises (ruling
//! T7-3); the orchestrator changes the plan at its gate only while it revises (ruling
//! T7-8, which replaced T7-4's scope), so the user approves exactly the version the gate
//! shows and every version follows a user action; and the plan tools' admission by
//! phase (ruling T1-O4, review m4).

use proto::{ActionKind, DocGateAction, DocGateKind, DocKind, RunState};
use serde_json::json;

use super::design_fixture::*;
use super::dispatch::replies;
use super::fixture::*;
use super::orch::{ORCH, orch_tool};
use super::planners::{PLANNER, planner_started, submit_epic};
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

const LOCKED: &str = "the plan v1 is waiting for the user; change it after they ask for changes";

fn changes(note: &str) -> DocGateAction {
    DocGateAction::Changes {
        note: note.into(),
        review: false,
    }
}

/// Ruling T7-8 (replacing T7-4's scope): at an open plan gate the orchestrator does not
/// revise, its plan changes (`edit_plan` edits, with or without a submit, and
/// `spawn_subplanner`) are refused exactly and change nothing; a submit alone changes
/// nothing either.
#[test]
fn orchestrator_plan_changes_at_the_open_plan_gate_are_refused() {
    let mut fx = at_plan_gate(false);
    let before = fx.run().clone();
    for args in [
        json!({"edits": [add_task("t2")]}),
        json!({"edits": [add_task("t2")], "submit": true}),
    ] {
        let effects = orch_tool(&mut fx, ORCH, "edit_plan", args);
        assert_eq!(refused(&effects), LOCKED);
    }
    let args = super::planners::spawn_args("mail", &["crates/mail/**"], "Plan mail");
    let effects = orch_tool(&mut fx, ORCH, "spawn_subplanner", args);
    assert_eq!(refused(&effects), LOCKED);
    assert_eq!(fx.run().state, RunState::AwaitingApproval);
    assert_eq!(gate(&fx), Some((DocGateKind::Plan, 1, None)));
    assert!(fx.run().task("t2").is_none());
    assert_eq!(fx.run().tasks, before.tasks);
    assert!(fx.run().orch.epics.is_empty());
    let effects = orch_tool(&mut fx, ORCH, "edit_plan", json!({"submit": true}));
    assert!(replies(&effects)[0].is_ok());
    assert_eq!(gate(&fx), Some((DocGateKind::Plan, 1, None)));
}

/// Ruling T7-8: once the user asks for changes, the orchestrator's edits are taken,
/// and its next submit opens v2, which is the version the user approves.
#[test]
fn changes_then_edits_then_a_submit_make_the_next_version() {
    let mut fx = at_plan_gate(false);
    act(&mut fx, DocGateKind::Plan, changes("Split t1.")).unwrap();
    let args = json!({"edits": [add_task("t2")]});
    assert!(replies(&orch_tool(&mut fx, ORCH, "edit_plan", args))[0].is_ok());
    assert!(fx.run().task("t2").is_some());
    let revising = Some("Split t1.".to_string());
    assert_eq!(gate(&fx), Some((DocGateKind::Plan, 1, revising)));
    let effects = orch_tool(&mut fx, ORCH, "edit_plan", json!({"submit": true}));
    assert!(replies(&effects)[0].is_ok(), "{effects:?}");
    assert_eq!(gate(&fx), Some((DocGateKind::Plan, 2, None)));
    assert_eq!(plan_version(&fx, 2), "revised: Split t1.");
    let reply = fx.approve();
    assert_eq!(replies(&reply), vec![Ok(format!("run {RUN_ID} approved"))]);
    assert!(notes(&fx).contains(&"the user approved the plan v2".to_string()));
}

/// Ruling T7-8: a sub-planner is the orchestrator's change too: started while the
/// orchestrator revises, its epic's tasks are in the next version.
#[test]
fn a_sub_planner_while_revising_is_in_the_next_version() {
    let mut fx = at_plan_gate(false);
    act(&mut fx, DocGateKind::Plan, changes("Add mail.")).unwrap();
    // Ruling T11-1: a design run's spawn names the requirements its epic owns.
    super::design_plan_fixture::spawn_covering(&mut fx, "mail", &["R2"]);
    assert_eq!(fx.run().state, RunState::Planning);
    assert!(!plan_submitted(&fx));
    planner_started(&mut fx, PLANNER);
    let effects = submit_epic(&mut fx, json!([add_task("mail")]));
    assert!(replies(&effects)[0].is_ok(), "{effects:?}");
    let effects = orch_tool(&mut fx, ORCH, "edit_plan", json!({"submit": true}));
    assert!(replies(&effects)[0].is_ok(), "{effects:?}");
    assert_eq!(gate(&fx), Some((DocGateKind::Plan, 2, None)));
    assert_eq!(plan_version(&fx, 2), "revised: Add mail.");
    assert_eq!(
        replies(&fx.approve()),
        vec![Ok(format!("run {RUN_ID} approved"))]
    );
    assert!(notes(&fx).contains(&"the user approved the plan v2".to_string()));
}

/// Ruling T7-8 and BD-2: the orchestrator's churn at the open gate makes no version, so
/// the user's revisions are all left.
#[test]
fn orchestrator_churn_does_not_use_up_the_plan_cap() {
    let mut fx = at_plan_gate(false);
    for n in 2..=10 {
        let args = json!({"edits": [add_task(&format!("c{n}"))], "submit": true});
        assert_eq!(
            refused(&orch_tool(&mut fx, ORCH, "edit_plan", args)),
            LOCKED
        );
        let effects = orch_tool(&mut fx, ORCH, "edit_plan", json!({"submit": true}));
        assert!(replies(&effects)[0].is_ok());
    }
    let design = fx.run().orch.design.as_ref().unwrap();
    assert_eq!(design.gate_versions(DocKind::Plan), 1);
    for n in 2..=6 {
        act(&mut fx, DocGateKind::Plan, changes("again")).unwrap();
        let args = json!({"edits": [add_task(&format!("t{n}"))], "submit": true});
        assert!(replies(&orch_tool(&mut fx, ORCH, "edit_plan", args))[0].is_ok());
        assert_eq!(gate(&fx), Some((DocGateKind::Plan, n, None)));
    }
}

/// Review m4 (ruling T1-O4): `edit_plan` in specifying is refused exactly, and
/// `submit_epic` is never the orchestrator's (its role refuses it before any phase
/// check; a sub-planner exists only from planning on).
#[test]
fn plan_tools_before_planning_are_refused() {
    let mut fx = at_brainstorm_gate(false);
    act(&mut fx, DocGateKind::Brainstorm, DocGateAction::APPROVE).unwrap();
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
