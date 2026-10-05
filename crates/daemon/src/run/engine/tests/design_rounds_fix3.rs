//! Milestone 9.6 task M9.6.15, fix round 3 (rulings T15-13 and T15-14, N7): a round
//! cancelled after its plan's approval but before its documents commit was sent is
//! dropped, its commit no longer due; a `pr` run's stage that the round's commit is
//! creating is never skipped by the cancel; and a dropped round's plan review number
//! is never taken again.

use proto::{DocGateAction, DocGateKind, RunState};
use serde_json::json;

use super::design_commit::{approve, commits};
use super::design_fixture::*;
use super::design_plan_fixture::read_back;
use super::design_review_fixture::written;
use super::design_rounds::round_task;
use super::design_rounds_commit::{ROUND_DOCS, pr_round_approved};
use super::design_rounds_fixture::*;
use super::fixture::*;
use super::goal_rounds_cancel::settle;
use super::goal_rounds_start::reply;
use super::kinds_cancel::cancel;
use crate::run::engine::OpResult;

/// Round 3 (`amend`) of a run back at `complete`: its amendment (round 2's text, from
/// the restored base) approved and read back as spec `v`, [`t3`] planned and reviewed
/// by `reviewer`.
fn round_three_at_plan_gate(fx: &mut Fixture, v: u32, reviewer: &str) {
    iterate_with(fx, None).unwrap();
    let answer = submit_amendment(fx, AMENDMENT).unwrap();
    assert_eq!(answer["version"], v, "{answer}");
    act(fx, DocGateKind::Spec, DocGateAction::APPROVE).unwrap();
    read_back(fx, v, AMENDMENT);
    round_submit(fx, json!([t3()])).unwrap();
    round_reviewed_as(fx, reviewer);
}

/// Ruling T15-13 (N5): round 2 approved, cancelled while stage 1 is being created and
/// before its commit is sent. The round is dropped as a reject drops it: the commit is
/// no longer due, nothing is committed, the run completes, and round 3's commit holds
/// only round 3's amendment.
#[test]
fn a_round_cancelled_before_its_commit_is_sent_is_dropped() {
    let mut fx = round_plan_gate(json!([round_task("t2", &["R2", "R3"])]));
    let round = fx
        .run()
        .orch
        .design
        .as_ref()
        .unwrap()
        .round
        .clone()
        .unwrap();
    assert!(commits(&approve(&mut fx)).is_empty(), "stage 1 comes first");
    let (op, _) = fx.op("CreateStageBranch");
    assert!(reply(&cancel(&mut fx)).is_ok());
    fx.done(op, OpResult::StageCreated);
    // Ruling T15-17 (R3): a merge runs after the cancel (round 1's fix on stage 1).
    merged_fix(&mut fx);
    settle(&mut fx);
    let run = fx.run();
    assert_eq!(run.state, RunState::Complete, "{:#?}", run.log);
    // Ruling T15-17: the dropped round's record says so, and its approval is gone.
    assert!(run.rounds[1].dropped);
    assert_eq!(run.rounds[1].approved_at, None);
    let design = run.orch.design.as_ref().unwrap();
    assert!(!design.commit_due, "nothing is left due");
    assert!(!crate::run::engine::design_commit::due(run));
    assert_eq!(design.approved_spec, round.spec_before);
    assert_eq!(design.requirements, round.base);
    assert_eq!(design.committed_round, 1);
    let sent = |p: &crate::run::model::PendingOp| {
        matches!(p.kind, crate::run::engine::OpKind::CommitDesignDocs(_))
    };
    assert!(
        !fx.run().pending_ops.values().any(sent),
        "no commit was sent"
    );
    round_three_at_plan_gate(&mut fx, 3, "plan-r3");
    // Round 2's stage 2, never created, is created empty first (stages stay contiguous).
    assert!(commits(&approve(&mut fx)).is_empty());
    let (op, kind) = fx.op("CreateStageBranch");
    assert!(format!("{kind:?}").contains("stage-2"), "{kind:?}");
    let mut effects = fx.done(op, OpResult::StageCreated);
    effects.extend(fx.tick());
    let asked = commits(&effects);
    assert_eq!(asked.len(), 1, "{effects:?}");
    let spec = &asked[0].1;
    let amended: Vec<(&str, Option<&str>)> = (spec.files.iter())
        .filter(|f| f.folder == "specs")
        .map(|f| (f.what.as_str(), f.append.as_deref()))
        .collect();
    assert_eq!(amended, [("spec v3", Some("## Round 3 amendment"))]);
    assert!(
        spec.message.starts_with("docs: round 3 spec amendment"),
        "{}",
        spec.message
    );
}

/// Ruling T15-14 (N6): in a `pr` run, round 2 cancelled with its commit in flight: the
/// stage the commit is creating is not skipped, and once recorded holds the amendment,
/// unskipped.
#[test]
fn a_pr_rounds_cancel_never_skips_the_stage_its_commit_creates() {
    let (mut fx, op, _) = pr_round_approved();
    assert!(reply(&cancel(&mut fx)).is_ok());
    let skipped = |fx: &Fixture| (fx.run().delivery.stage(2)).is_some_and(|s| s.skipped);
    assert!(!skipped(&fx), "{:#?}", fx.run().log);
    let result = OpResult::DocsCommitted {
        head: ROUND_DOCS.into(),
        spec: "docs/anthrex/specs/1970-01-01-password-reset.md".into(),
    };
    fx.done(op, result);
    fx.tick();
    let run = fx.run();
    assert_eq!(run.stage(2).map(|s| s.head.as_str()), Some(ROUND_DOCS));
    assert!(!skipped(&fx), "{:#?}", run.log);
}

/// N7 (ruling T15-9, the plan's side): round 2 rejected at its plan gate after its plan
/// review (`plan-r2`); round 3's plan review is `plan-r3`, with its own draft file.
#[test]
fn a_dropped_rounds_plan_review_number_is_never_reused() {
    let mut fx = round_plan_gate(json!([round_task("t2", &["R2", "R3"])]));
    act(&mut fx, DocGateKind::Plan, DocGateAction::Reject).unwrap();
    assert_eq!(fx.run().state, RunState::Complete);
    iterate_with(&mut fx, None).unwrap();
    submit_amendment(&mut fx, AMENDMENT).unwrap();
    act(&mut fx, DocGateKind::Spec, DocGateAction::APPROVE).unwrap();
    read_back(&mut fx, 3, AMENDMENT);
    let effects = round_submit_effects(&mut fx);
    assert!(
        written(&effects).contains(&"plan-draft-r3.md".to_string()),
        "{:?}",
        written(&effects)
    );
    round_reviewed_as(&mut fx, "plan-r3");
    let design = fx.run().orch.design.as_ref().unwrap();
    let plans: Vec<(u32, bool)> = (design.reviews.iter())
        .filter(|r| r.doc == proto::DocKind::Plan)
        .map(|r| (r.n, r.dropped))
        .collect();
    assert_eq!(plans, [(1, false), (2, true), (3, false)]);
}

/// Round 3's task `t3` in stage 3 (round 2's stage 2 is that round's, done or not).
fn t3() -> serde_json::Value {
    let mut t3 = round_task("t3", &["R2", "R3"]);
    t3["task"]["stage"] = json!(3);
    t3
}

/// Round 3's plan submit of [`t3`]: its effects.
fn round_submit_effects(fx: &mut Fixture) -> Vec<crate::run::engine::Effect> {
    let args = json!({"edits": [t3()], "submit": true});
    super::orch::orch_tool(fx, super::orch::ORCH, "edit_plan", args)
}

/// An engine-made fix of round 1 on stage 1, through the real merge queue: its merge
/// candidate is sent (`start_merge` is not held) and merges.
pub(super) fn merged_fix(fx: &mut Fixture) {
    let spec = super::fixes::spec(
        proto::TaskOrigin::Bisect,
        "crates/fix/**",
        Default::default(),
    );
    let now = fx.now;
    let id = crate::run::engine::fixes::add_fix(fx.run_mut(), spec, now, &mut Vec::new());
    let id = id.expect("added");
    super::kinds_integration::merge_real(fx, &id, super::kinds_integration::C3);
    assert_eq!(
        fx.task(&id).state,
        proto::TaskState::Merged,
        "{:#?}",
        fx.run().log
    );
}
