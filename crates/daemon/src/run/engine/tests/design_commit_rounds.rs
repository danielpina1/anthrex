//! Milestone 9.6 task M9.6.12, fix round 1: an op that cannot be built halts with
//! decision 23's text (ruling T12-3, m1). Ruling T12-2's round-1-only test is replaced
//! by task M9.6.15's round commits (`design_rounds_commit.rs`).

use proto::RunState;

use super::design_commit::{DOCS, approve, commits, pending_commit, plan_gate_with};
use super::design_fixture::at_plan_gate;
use super::design_plan_fixture::covering;
use super::design_rounds_commit::approved_round;
use super::fixture::Fixture;
use super::kinds_integration::{C1, merge_real};
use crate::run::delivery::body::pr_body;
use crate::run::engine::{OpKind, OpResult};

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

/// The W2 re-review's N4: a documents commit that took the `-2` names (ruling WB-B-I2)
/// is the path the stage's PR body covers, and the one round 2 appends its amendment
/// to, its plan named beside it.
#[test]
fn a_suffixed_spec_path_reaches_the_pr_body_and_round_2() {
    const SUFFIXED: &str = "docs/anthrex/specs/1970-01-01-password-reset-2.md";
    let mut fx: Fixture = at_plan_gate(false);
    approve(&mut fx);
    let (op, _) = pending_commit(&fx);
    let result = OpResult::DocsCommitted {
        head: DOCS.into(),
        spec: SUFFIXED.into(),
    };
    fx.done(op, result);
    let body = pr_body(fx.run(), 1);
    let covers = format!("Covers R1, R2 (spec: {SUFFIXED})\n");
    assert!(body.contains(&covers), "{body}");
    // Round 1 completes, as `design_complete` does.
    let started =
        |p: &crate::run::model::PendingOp| matches!(p.kind, OpKind::StartDesignAgent { .. });
    fx.run_mut().pending_ops.retain(|_, p| !started(p));
    merge_real(&mut fx, "t1", C1);
    let (op, _) = fx.op("VerifyRefs");
    fx.done(op, OpResult::RefsOk);
    assert_eq!(fx.run().state, RunState::Complete, "{:#?}", fx.run().log);
    let (_, _, spec) = approved_round(fx);
    let paths: Vec<(Option<&str>, Option<&str>)> = (spec.files.iter())
        .map(|f| (f.repo_path.as_deref(), f.append.as_deref()))
        .collect();
    let plan = "docs/anthrex/plans/1970-01-01-password-reset-2-round2.md";
    assert_eq!(
        paths,
        [
            (Some(SUFFIXED), Some("## Round 2 amendment")),
            (Some(plan), None)
        ]
    );
}
