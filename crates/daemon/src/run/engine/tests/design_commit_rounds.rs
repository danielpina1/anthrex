//! Milestone 9.6 task M9.6.12, fix round 1: the documents commit is round 1's only
//! (ruling T12-2, until M9.6.15 places a later round's), and an op that cannot be
//! built halts with decision 23's text (ruling T12-3, m1).

use proto::{DocGateKind, RunState};
use serde_json::json;

use super::design_commit::{DOCS, approve, commits, committed, pending_commit, plan_gate_with};
use super::design_plan_fixture::covering;
use crate::run::design::state::{DocGate, Revision};
use crate::run::engine::OpResult;
use crate::run::model::StageLayout;

/// Ruling T12-2: a `Multi` design run whose round 1 committed its documents and
/// created stage 1 from the commit, then at round 2's plan gate: approving it commits
/// nothing, and the stages and `run_head` stay as they were.
#[test]
fn round_two_of_a_multi_run_commits_nothing_and_keeps_the_stages() {
    let mut t2 = covering("t2", &["R2"]);
    t2["task"]["stage"] = json!(2);
    t2["task"]["deps"] = json!(["t1"]);
    let mut fx = plan_gate_with(|_| {}, &[covering("t1", &["R1"]), t2]);
    approve(&mut fx);
    let (op, _) = pending_commit(&fx);
    committed(&mut fx, op);
    let (op, _) = fx.op("CreateStageBranch");
    fx.done(op, OpResult::StageCreated);
    assert_eq!(fx.run().stage_layout, StageLayout::Multi);
    assert_eq!(fx.run().stage(1).map(|s| s.head.as_str()), Some(DOCS));

    // Round 2 at its plan gate (`goal_rounds::iterate`, then a plan gate the design
    // flow opens): the round's record and the open gate, as they would stand.
    let run = fx.run_mut();
    let mut round = run.rounds.last().cloned().unwrap();
    round.n = 2;
    round.first_stage = 3;
    run.rounds.push(round);
    run.state = RunState::AwaitingApproval;
    let design = run.orch.design.as_mut().unwrap();
    let version = design.gate_versions(proto::DocKind::Plan);
    design.gate = Some(DocGate {
        kind: DocGateKind::Plan,
        version,
        opened_at: 0,
        revising: None,
        review: false,
        cause: Revision::Changes,
    });
    let (stages, head) = (fx.run().stages.clone(), fx.run().run_head.clone());

    let mut effects = approve(&mut fx);
    effects.extend(fx.tick());
    assert!(commits(&effects).is_empty(), "{effects:?}");
    let run = fx.run();
    let design = run.orch.design.as_ref().unwrap();
    assert!(!design.commit_due);
    assert_eq!(design.committed.as_deref(), Some(DOCS), "round 1's commit");
    assert_eq!(run.stages, stages);
    assert_eq!(run.run_head, head);
    assert_eq!(run.state, RunState::Running);
}

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
