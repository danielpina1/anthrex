//! Milestone 9.6 task M9.6.11's fixtures: a design run in planning (its spec approved
//! and read back), tasks that cover the spec's requirements, the orchestrator's plan
//! submits, and the plan reviewer's session, stubbed by its op's result and its
//! `submit_findings`.

use proto::{DocGateAction, DocGateKind, DocKind, RunState};
use serde_json::{Value, json};

use super::design_agents::{launches, started};
use super::design_fixture::*;
use super::design_review_fixture::{outcome, submit_findings};
use super::fixture::*;
use super::orch::{ORCH, orch_tool};
use crate::run::engine::{DocChecked, Effect, EventKind};

/// The plan reviewer's window in these tests.
pub(super) const PLAN_REVIEWER: u32 = 902;

/// The driver read spec v`n` back as `text` (`Effect::ReadBack`'s answer).
pub(super) fn read_back(fx: &mut Fixture, n: u32, text: &str) -> Vec<Effect> {
    let checked = vec![DocChecked {
        kind: DocKind::Spec,
        n,
        read: Ok(Some(text.to_string())),
    }];
    fx.next(EventKind::DesignChecked {
        run_id: RUN_ID.into(),
        checked,
    })
}

/// A design run in planning: its spec v1 approved and read back, so its requirements
/// (R1, R2) are stored.
pub(super) fn planning() -> Fixture {
    let mut fx = at_spec_gate(false);
    act(&mut fx, DocGateKind::Spec, DocGateAction::Approve).unwrap();
    read_back(&mut fx, 1, SPEC);
    assert_eq!(fx.run().state, RunState::Planning);
    let design = fx.run().orch.design.as_ref().unwrap();
    assert_eq!(design.requirements.len(), 2, "{design:?}");
    fx
}

/// [`add_task`] of `id`, covering `covers`.
pub(super) fn covering(id: &str, covers: &[&str]) -> Value {
    let mut edit = add_task(id);
    edit["task"]["covers"] = json!(covers);
    edit
}

/// The orchestrator's `edit_plan` with `edits`, `submit: true` and, when given,
/// `responses`: its answer.
pub(super) fn plan_submit(
    fx: &mut Fixture,
    edits: Value,
    responses: Value,
) -> Result<Value, String> {
    let mut args = json!({"edits": edits, "submit": true});
    if !responses.is_null() {
        args["responses"] = responses;
    }
    outcome(&orch_tool(fx, ORCH, "edit_plan", args))
}

/// The plan review's reviewer, launched by the first passing submit, live in
/// [`PLAN_REVIEWER`] with `findings` in.
pub(super) fn plan_reviewed(fx: &mut Fixture, findings: Value) {
    let label = launches(fx).last().unwrap().1.kind.label();
    assert_eq!(label, "plan-r1");
    started(fx, &label, PLAN_REVIEWER);
    outcome(&submit_findings(fx, PLAN_REVIEWER, findings)).unwrap();
}

/// Ruling T11-1: `spawn_subplanner` of `epic` over `crates/<epic>/**` in a design run,
/// owning `covers`, which must be accepted.
pub(super) fn spawn_covering(fx: &mut Fixture, epic: &str, covers: &[&str]) {
    let area = format!("crates/{epic}/**");
    let mut args = super::planners::spawn_args(epic, &[&area], &format!("Plan {epic}"));
    args["covers"] = json!(covers);
    let effects = orch_tool(fx, ORCH, "spawn_subplanner", args);
    outcome(&effects).unwrap_or_else(|e| panic!("the spawn was refused: {e}"));
}
