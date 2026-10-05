//! Milestone 9.6 task M9.6.12, fix round 1: an op that cannot be built halts with
//! decision 23's text (ruling T12-3, m1). Ruling T12-2's round-1-only test is replaced
//! by task M9.6.15's round commits (`design_rounds_commit.rs`).

use proto::RunState;

use super::design_commit::{approve, commits, plan_gate_with};
use super::design_plan_fixture::covering;

/// Ruling T12-3 (m1): a commit that cannot be built (an approved version missing from
/// the index) halts with decision 23's exact text.
#[test]
fn an_unbuildable_commit_halts_with_decision_23s_text() {
    let mut fx = plan_gate_with(|_| {}, &[covering("t1", &["R1", "R2"])]);
    // The read-back stored the requirements under v1; the approval points elsewhere.
    fx.run_mut().orch.design.as_mut().unwrap().approved_spec = Some(99);
    let effects = approve(&mut fx);
    assert!(commits(&effects).is_empty());
    let run = fx.run();
    assert_eq!(run.state, RunState::Halted);
    let text =
        "design flow: could not commit the spec and plan: the approved spec has no stored version";
    assert_eq!(run.halted_reason.as_deref(), Some(text));
}
