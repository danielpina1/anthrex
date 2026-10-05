//! Milestone 9.6 task M9.6.15, fix round 1 (ruling T15-6): round 2 of a design run
//! whose round 1 was already `Multi` (stages 1 and 2). The round's documents commit is
//! its first new stage, stage 3, made from stage 2's head; stages 1 and 2 never move.

use serde_json::json;

use super::design_commit::{approve, commits, committed, pending_commit};
use super::design_fixture::*;
use super::design_plan_fixture::{covering, plan_reviewed, read_back};
use super::design_rounds_commit::ROUND_DOCS;
use super::design_rounds_fixture::*;
use super::fixture::*;
use super::kinds_integration::{C1, C2, merge_real};
use super::orch::{ORCH, orch_tool};
use crate::run::engine::OpResult;
use crate::run::model::StageLayout;
use proto::{DocGateAction, DocGateKind, RunState};

/// Task `id` of stage `n` covering `covers`.
fn staged(id: &str, n: u16, covers: &[&str]) -> serde_json::Value {
    let mut edit = covering(id, covers);
    edit["task"]["stage"] = json!(n);
    edit
}

/// The pending stage creation, answered: its branch.
fn stage_created(fx: &mut Fixture) -> String {
    let (op, kind) = fx.op("CreateStageBranch");
    fx.done(op, OpResult::StageCreated);
    format!("{kind:?}")
}

/// A design run whose round 1 is `Multi` and complete: `t1` merged into stage 1 at
/// `C1`, then `t1b` (which needs it) into stage 2 at `C2`.
fn multi_complete() -> Fixture {
    let mut fx = at_spec_gate(false);
    act(&mut fx, DocGateKind::Spec, DocGateAction::Approve).unwrap();
    read_back(&mut fx, 1, SPEC);
    let mut t1b = staged("t1b", 2, &["R2"]);
    t1b["task"]["deps"] = json!(["t1"]);
    let edits = json!([staged("t1", 1, &["R1"]), t1b]);
    let effects = orch_tool(
        &mut fx,
        ORCH,
        "edit_plan",
        json!({"edits": edits, "submit": true}),
    );
    assert!(super::dispatch::replies(&effects)[0].is_ok(), "{effects:?}");
    plan_reviewed(&mut fx, json!([]));
    let effects = orch_tool(&mut fx, ORCH, "edit_plan", json!({"submit": true}));
    assert!(super::dispatch::replies(&effects)[0].is_ok(), "{effects:?}");
    approve(&mut fx);
    assert_eq!(fx.run().stage_layout, StageLayout::Multi);
    let (op, _) = pending_commit(&fx);
    committed(&mut fx, op);
    let stubbed = |p: &crate::run::model::PendingOp| {
        matches!(p.kind, crate::run::engine::OpKind::StartDesignAgent { .. })
    };
    fx.run_mut().pending_ops.retain(|_, p| !stubbed(p));
    assert!(stage_created(&mut fx).contains("stage-1"));
    merge_real(&mut fx, "t1", C1);
    assert!(stage_created(&mut fx).contains("stage-2"));
    merge_real(&mut fx, "t1b", C2);
    let (op, _) = fx.op("VerifyRefs");
    fx.done(op, OpResult::RefsOk);
    assert_eq!(fx.run().state, RunState::Complete, "{:#?}", fx.run().log);
    fx
}

/// Ruling T15-6: round 2 of a run whose round 1 had stages 1 and 2: its commit is stage
/// 3, from stage 2's head; recorded, stages 1 and 2 are where round 1 left them, and
/// the run head and `integration` follow stage 3.
#[test]
fn round_two_from_a_multi_round_one_commits_as_its_next_stage() {
    let fx = multi_complete();
    let (one, two) = {
        let run = fx.run();
        (
            run.stage(1).unwrap().head.clone(),
            run.stage(2).unwrap().head.clone(),
        )
    };
    assert_eq!((one.as_str(), two.as_str()), (C1, C2));
    let mut fx = plan_gate_of(fx, json!([staged("t2", 3, &["R2", "R3"])]));
    let effects = approve(&mut fx);
    let mut asked = commits(&effects);
    asked.extend(commits(&fx.tick()));
    assert_eq!(asked.len(), 1, "{effects:?}");
    let (op, spec) = asked.remove(0);
    let run = fx.run();
    assert_eq!(
        spec.stage_branch.as_deref(),
        Some(run.stage_branch(3).as_str())
    );
    assert_eq!(spec.expected_head, C2, "from stage 2's head");
    let result = OpResult::DocsCommitted {
        head: ROUND_DOCS.into(),
        spec: "docs/anthrex/specs/1970-01-01-password-reset.md".into(),
    };
    let mut effects = fx.done(op, result);
    effects.extend(fx.tick());
    let run = fx.run();
    let three = run.stage(3).expect("stage 3 recorded");
    assert_eq!((three.head.as_str(), three.round), (ROUND_DOCS, 2));
    assert_eq!(three.synced_from.as_deref(), Some(C2));
    assert_eq!(run.stage(1).unwrap().head, C1, "stage 1 never moves");
    assert_eq!(run.stage(2).unwrap().head, C2, "stage 2 never moves");
    assert_eq!(run.run_head, ROUND_DOCS);
    assert!(
        !ops_in(&effects, "PrepareWorktree").is_empty(),
        "{effects:?}"
    );
}
