//! Ruling T7-10 (the final fix wave's FW-5): brief ruling BD-2's cap counts the gate
//! versions that follow a user action (changes, back, edit), plus v1. A version opened
//! by a read-back resubmit or by an engine update takes the next number but does not
//! count, and opening one is never refused by the cap.

use proto::{DocGateAction, DocGateKind, DocKind, PlanEdit, Size};
use serde_json::json;

use super::design_fixture::*;
use super::dispatch::edit;
use super::fixture::*;
use super::orch::{ORCH, orch_tool};
use crate::run::engine::{DocChecked, EventKind};

fn changes() -> DocGateAction {
    DocGateAction::Changes {
        note: "again".into(),
        review: false,
    }
}

const SPEC_FULL: &str = "the spec has had its 6 versions; approve, go back or reject";
const PLAN_FULL: &str = "the plan has had its 6 versions; approve, go back or reject";

/// The spec, its text made `n`'s own.
fn spec(n: u32) -> String {
    SPEC.replace("Mail delays.", &format!("Mail delays {n}."))
}

/// The user's changes to the open spec gate, then the orchestrator's next version.
fn spec_cycle(fx: &mut Fixture, n: u32) {
    act(fx, DocGateKind::Spec, changes()).unwrap();
    assert_eq!(submitted(fx, "spec", &spec(n))["version"], n);
}

/// A restore could not read spec v`n` back: the gate reopens revising, and the
/// orchestrator submits it again as the next version.
fn spec_read_back(fx: &mut Fixture, n: u32) {
    fx.next(EventKind::DesignChecked {
        run_id: RUN_ID.into(),
        checked: vec![DocChecked {
            kind: DocKind::Spec,
            n,
            read: Err("its file is missing".into()),
        }],
    });
    assert_eq!(gate(fx).map(|g| g.2.is_some()), Some(true), "revising");
    assert_eq!(submitted(fx, "spec", &spec(n + 1))["version"], n + 1);
}

#[test]
fn a_read_back_version_does_not_count_toward_the_cap() {
    let mut fx = at_spec_gate(false);
    for n in 2..=4 {
        spec_cycle(&mut fx, n);
    }
    spec_read_back(&mut fx, 4);
    spec_cycle(&mut fx, 6);
    // v6, but only five versions followed a user action (v1 to v4 and v6).
    spec_cycle(&mut fx, 7);
    assert_eq!(
        act(&mut fx, DocGateKind::Spec, changes()),
        Err(SPEC_FULL.to_string())
    );
    // At the full cap, a read-back still opens its version.
    spec_read_back(&mut fx, 7);
    assert_eq!(gate(&fx).map(|g| g.1), Some(8));
}

/// The plan's next version through changes and the orchestrator's submit.
fn plan_cycle(fx: &mut Fixture, n: u32) {
    act(fx, DocGateKind::Plan, changes()).unwrap();
    let args = json!({"edits": [add_task(&format!("c{n}"))], "submit": true});
    orch_tool(fx, ORCH, "edit_plan", args);
    assert_eq!(gate(fx).map(|g| g.1), Some(n));
}

/// The engine's update at the open plan gate (a size a decider set): the next version.
fn engine_update(fx: &mut Fixture, n: u32, size: Size) {
    let t1 = (fx.run_mut().tasks.iter_mut()).find(|t| t.id() == "t1");
    t1.unwrap().size = size;
    let now = fx.now + 1;
    fx.send(now, EventKind::Tick);
    assert_eq!(gate(fx).map(|g| g.1), Some(n));
}

#[test]
fn an_engine_version_does_not_count_toward_the_cap() {
    let mut fx = at_plan_gate(false);
    for n in 2..=5 {
        plan_cycle(&mut fx, n);
    }
    engine_update(&mut fx, 6, Size::M);
    plan_cycle(&mut fx, 7);
    assert_eq!(
        act(&mut fx, DocGateKind::Plan, changes()),
        Err(PLAN_FULL.to_string())
    );
    // At the full cap, the engine's update still opens its version.
    engine_update(&mut fx, 8, Size::L);
}

/// A user's `run edit` at the open plan gate is a user action: its version counts.
#[test]
fn a_user_edit_version_counts_toward_the_cap() {
    let mut fx = at_plan_gate(false);
    for (n, size) in (2..=6).zip(["M", "S", "M", "S", "M"]) {
        let amend = json!({"op": "amend_task", "task_id": "t1", "size": size});
        let amend: PlanEdit = serde_json::from_value(amend).unwrap();
        edit(&mut fx, vec![amend]);
        assert_eq!(gate(&fx).map(|g| g.1), Some(n));
    }
    assert_eq!(
        act(&mut fx, DocGateKind::Plan, changes()),
        Err(PLAN_FULL.to_string())
    );
}
