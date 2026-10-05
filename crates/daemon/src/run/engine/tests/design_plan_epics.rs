//! Milestone 9.6 task M9.6.11, fix round 1 (ruling T11-1, decision 22; ruling T11-3,
//! m4): in a design run with requirements, `spawn_subplanner` names the requirement
//! ids its epic owns (`covers`), and the sub-planner's first turn carries exactly those
//! R-lines, then the spec's Goal and Interfaces; a re-plan gets them with the ones its
//! epic's tasks cover. Coverage is still checked over the whole plan at submit. A run
//! without the flow spawns as in 9.5.

use proto::DocGateAction;
use proto::DocGateKind;
use serde_json::{Value, json};

use super::design_fixture::*;
use super::design_plan_fixture::*;
use super::design_review_fixture::outcome;
use super::fixture::*;
use super::orch::{ORCH, launched, orch_tool};
use super::planners::{PLANNER, planner_started, spawn, spawn_args, submit_epic};
use crate::run::engine::OpKind;

const NEEDS: &str =
    "in a design run, spawn_subplanner needs covers: the requirement ids this epic owns";

/// `spawn_subplanner` of `epic` over `crates/<epic>/**`, with `covers` when given: its
/// answer.
fn spawn_owning(fx: &mut Fixture, epic: &str, covers: Option<Value>) -> Result<Value, String> {
    let area = format!("crates/{epic}/**");
    let mut args = spawn_args(epic, &[&area], &format!("Plan {epic}"));
    if let Some(covers) = covers {
        args["covers"] = covers;
    }
    outcome(&orch_tool(fx, ORCH, "spawn_subplanner", args))
}

/// The first turn of `epic`'s latest sub-planner launch.
fn planner_turn(fx: &Fixture, epic: &str) -> String {
    let found = (fx.ops("StartPlanner").into_iter().rev()).find_map(|(_, kind)| match kind {
        OpKind::StartPlanner { spec } if spec.epic == epic => Some(spec.first_turn),
        _ => None,
    });
    found.unwrap_or_else(|| panic!("no sub-planner of {epic} launched"))
}

fn block(lines: &[&str]) -> String {
    let mut out = String::from("\n\nSpec requirements this epic delivers:\n");
    for line in lines {
        out.push_str(line);
        out.push('\n');
    }
    out.push_str("Goal (from the spec): Users reset their password.\n");
    out.push_str("Interfaces (from the spec):\n`reset(token)`.");
    out
}

const R1: &str = "R1  Tokens expire after an hour. Check: a clock test.";
const R2: &str = "R2  Links are single use. Check: a reuse test.";

/// Ruling T11-1: a spawn without `covers`, with none, or with an id the approved spec
/// does not have is refused and records no epic; with them, the first turn carries
/// exactly those R-lines, and the snapshot shows them.
#[test]
fn a_first_spawn_gets_exactly_the_requirements_it_names() {
    let mut fx = planning();
    assert_eq!(spawn_owning(&mut fx, "mail", None).unwrap_err(), NEEDS);
    assert_eq!(
        spawn_owning(&mut fx, "mail", Some(json!([]))).unwrap_err(),
        NEEDS
    );
    let unknown = spawn_owning(&mut fx, "mail", Some(json!(["R2", "R9"])));
    assert_eq!(
        unknown.unwrap_err(),
        "epic mail covers R9, which the spec does not have"
    );
    assert!(fx.run().orch.epics.is_empty());
    spawn_owning(&mut fx, "mail", Some(json!(["R2"]))).unwrap();
    let turn = planner_turn(&fx, "mail");
    assert!(turn.ends_with(&block(&[R2])), "{turn}");
    let info = crate::run::snapshot::snapshot(&fx.state, fx.now).runs[0].clone();
    assert_eq!(info.planners[0].covers, ["R2"]);
}

/// Ruling T11-1: a re-plan's first turn carries the union of its spawn's `covers` and
/// those of its epic's tasks, in the spec's order.
#[test]
fn a_re_plan_gets_its_spawns_and_its_tasks_requirements() {
    let mut fx = planning();
    spawn_owning(&mut fx, "mail", Some(json!(["R2"]))).unwrap();
    planner_started(&mut fx, PLANNER);
    let effects = submit_epic(&mut fx, json!([covering("mail", &["R1"])]));
    assert!(super::dispatch::replies(&effects)[0].is_ok(), "{effects:?}");
    spawn_owning(&mut fx, "mail", Some(json!(["R2"]))).unwrap();
    let turn = planner_turn(&fx, "mail");
    assert!(turn.ends_with(&block(&[R1, R2])), "{turn}");
}

/// Ruling T11-3 (m4): a sub-planner spawned before the approved spec is read back would
/// get no requirements, so the spawn waits for it.
#[test]
fn a_spawn_waits_for_the_approved_specs_read_back() {
    let mut fx = at_spec_gate(false);
    act(&mut fx, DocGateKind::Spec, DocGateAction::APPROVE).unwrap();
    let early = spawn_owning(&mut fx, "mail", Some(json!(["R1"])));
    assert_eq!(
        early.unwrap_err(),
        "the approved spec is still being read back; spawn the sub-planner again in a moment"
    );
    assert!(fx.run().orch.epics.is_empty());
    read_back(&mut fx, 1, SPEC);
    spawn_owning(&mut fx, "mail", Some(json!(["R1"]))).unwrap();
}

/// Ruling T11-1: a run without the design flow spawns without `covers`, as in 9.5, and
/// its snapshot's planner has no `covers` key.
#[test]
fn a_run_without_the_flow_spawns_as_in_9_5() {
    let mut fx = launched(false);
    spawn(&mut fx, "mail");
    let info = crate::run::snapshot::snapshot(&fx.state, fx.now).runs[0].clone();
    let json = serde_json::to_value(&info.planners[0]).unwrap();
    assert!(json.get("covers").is_none(), "{json}");
    assert!(!planner_turn(&fx, "mail").contains("Spec requirements"));
}
