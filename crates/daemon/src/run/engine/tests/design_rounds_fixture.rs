//! Milestone 9.6 task M9.6.15's fixtures: a design run whose round 1 is complete (its
//! spec and plan approved, its documents committed, its one task merged), a round of it
//! started with a chosen design, and the round's spec amendment and plan.

use proto::{DocGateAction, DocGateKind, RoundDesign, RunState};
use serde_json::{Value, json};

use super::design_agents::{launches, started};
use super::design_commit::{approve, committed, pending_commit};
use super::design_fixture::*;
use super::design_plan_fixture::read_back;
use super::design_review_fixture::{outcome, submit_findings};
use super::fixture::*;
use super::goal_rounds_start::reply;
use super::kinds_integration::{C1, merge_real};
use super::orch::{ORCH, orch_tool};
use crate::run::engine::{Effect, EventKind, OpKind, OpResult};
use crate::run::model::PendingOp;

/// Round 2's spec amendment: R2 changed, R3 new (decision 14).
pub(super) const AMENDMENT: &str = "\
# Password reset on mobile

## Goal and success criteria
Users reset their password from the app too.

## Non-goals
SSO.

## Approach
Stored tokens.

## Design
A deep link.

## Requirements
R2 Links are single use, in the app too. Check: a reuse test.
R3 The app opens reset links. Check: a deep-link test.

## Interfaces
`open(link)`.

## Errors and edge cases
Expired links.

## Testing
A reuse test and a deep-link test.

## Risks
Store review delays.

## Open questions
";

/// The round plan reviewer's window in these tests.
pub(super) const ROUND_REVIEWER: u32 = 912;

/// A design run whose round 1 is `complete`: spec v1 (R1, R2) and plan v1 (`t1`)
/// approved, the documents committed at `DOCS`, `t1` merged at `C1`.
pub(super) fn design_complete() -> Fixture {
    design_complete_with(false)
}

/// [`design_complete`], started with `--yes` when `yes`.
pub(super) fn design_complete_with(yes: bool) -> Fixture {
    let mut fx = at_plan_gate(yes);
    approve(&mut fx);
    let (op, _) = pending_commit(&fx);
    committed(&mut fx, op);
    // The brainstormers' starts the fixture stubbed are never answered (set in place).
    let stubbed = |p: &PendingOp| matches!(p.kind, OpKind::StartDesignAgent { .. });
    fx.run_mut().pending_ops.retain(|_, p| !stubbed(p));
    merge_real(&mut fx, "t1", C1);
    let (op, _) = fx.op("VerifyRefs");
    fx.done(op, OpResult::RefsOk);
    assert_eq!(fx.run().state, RunState::Complete, "{:#?}", fx.run().log);
    fx
}

/// The user's `run iterate` with `design`: its reply.
pub(super) fn iterate_with(
    fx: &mut Fixture,
    design: Option<RoundDesign>,
) -> Result<String, String> {
    let reply_id = fx.reply();
    let effects = fx.next(EventKind::Iterate {
        reply: reply_id,
        run_id: RUN_ID.into(),
        goal: "Add the app".into(),
        design,
    });
    reply(&effects)
}

/// The orchestrator's spec amendment, `ready: true`: its answer.
pub(super) fn submit_amendment(fx: &mut Fixture, text: &str) -> Result<Value, String> {
    let args = json!({"kind": "spec", "text": text, "ready": true, "amend": true});
    outcome(&orch_tool(fx, ORCH, "submit_doc", args))
}

/// `fx` iterated into round 2 (`amend`), its amendment at the spec gate as v2.
pub(super) fn amended(mut fx: Fixture) -> Fixture {
    iterate_with(&mut fx, None).unwrap();
    assert_eq!(fx.run().state, RunState::Specifying);
    let answer = submit_amendment(&mut fx, AMENDMENT).unwrap();
    assert_eq!(answer["version"], 2, "{answer}");
    fx
}

/// [`design_complete`] [`amended`].
pub(super) fn amendment_at_gate() -> Fixture {
    amended(design_complete())
}

/// [`amended`] `fx`, approved and read back: round 2 plans.
pub(super) fn planned(fx: Fixture) -> Fixture {
    let mut fx = amended(fx);
    act(&mut fx, DocGateKind::Spec, DocGateAction::APPROVE).unwrap();
    read_back(&mut fx, 2, AMENDMENT);
    assert_eq!(fx.run().state, RunState::Planning);
    fx
}

/// [`design_complete`] [`planned`].
pub(super) fn round_planning() -> Fixture {
    planned(design_complete())
}

/// The orchestrator's plan submit of `edits`: its answer.
pub(super) fn round_submit(fx: &mut Fixture, edits: Value) -> Result<Value, String> {
    let args = json!({"edits": edits, "submit": true});
    outcome(&orch_tool(fx, ORCH, "edit_plan", args))
}

/// The round's plan review, launched by its first passing submit, in with no findings;
/// then the submit that opens the round's plan gate.
pub(super) fn round_reviewed(fx: &mut Fixture) -> Vec<Effect> {
    round_reviewed_as(fx, "plan-r2")
}

/// [`round_reviewed`], its reviewer's label `label` (ruling T15-9: the numbers run on
/// across rounds, a dropped round's included).
pub(super) fn round_reviewed_as(fx: &mut Fixture, label: &str) -> Vec<Effect> {
    assert_eq!(launches(fx).last().unwrap().1.kind.label(), label);
    started(fx, label, ROUND_REVIEWER);
    outcome(&submit_findings(fx, ROUND_REVIEWER, json!([]))).unwrap();
    let effects = orch_tool(fx, ORCH, "edit_plan", json!({"submit": true}));
    outcome(&effects).unwrap();
    assert_eq!(gate(fx).map(|g| g.0), Some(DocGateKind::Plan));
    effects
}

/// [`planned`] `fx`, `edits` planned and reviewed: round 2 at its plan gate.
pub(super) fn plan_gate_of(fx: Fixture, edits: Value) -> Fixture {
    let mut fx = planned(fx);
    round_submit(&mut fx, edits).unwrap();
    round_reviewed(&mut fx);
    fx
}

/// [`design_complete`] [`plan_gate_of`].
pub(super) fn round_plan_gate(edits: Value) -> Fixture {
    plan_gate_of(design_complete(), edits)
}
