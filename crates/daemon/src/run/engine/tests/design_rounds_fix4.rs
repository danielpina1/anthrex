//! Milestone 9.6 task M9.6.15, fix round 4 (rulings T15-15, T15-16 and T15-17): the
//! stage a round's documents commit created is the round record's own
//! (`committed_stage`), so a stage above it created at the same head is skipped as 9.2
//! skips it; and a cancelled round whose commit then fails is dropped, never halted.

use proto::{RunState, TaskState};
use serde_json::json;

use super::delivery_open::host_ops_in;
use super::design_commit::{approve, commits};
use super::design_rounds::round_task;
use super::design_rounds_commit::{ROUND_DOCS, approved_round, pr_complete};
use super::design_rounds_fix3::merged_fix;
use super::design_rounds_fixture::*;
use super::fixture::*;
use super::goal_rounds_start::reply;
use super::kinds_cancel::cancel;
use crate::run::delivery::ops::HostOp;
use crate::run::engine::{Effect, OpResult};

/// The stages of every `OpenPr` among `effects`.
fn opened(effects: &[Effect]) -> Vec<u16> {
    (host_ops_in(effects).into_iter())
        .filter_map(|op| match op {
            HostOp::OpenPr { stage, .. } => Some(stage),
            _ => None,
        })
        .collect()
}

/// Ruling T15-15 (R1): round 2 of a `pr` run with `t2` in stage 2 and an independent
/// `t3` in stage 3. The commit lands as stage 2; stage 3 is created at the same head.
/// The round's cancel skips stage 3 (no changes, 9.2's rule), and no PR is opened for
/// it; stage 2, holding the amendment, is kept and is the round record's stage.
#[test]
fn a_stage_above_the_amendment_at_its_head_is_skipped() {
    let mut t3 = round_task("t3", &["R3"]);
    t3["task"]["stage"] = json!(3);
    let edits = json!([round_task("t2", &["R2", "R3"]), t3]);
    let mut fx = plan_gate_of(pr_complete(), edits);
    approve(&mut fx);
    let (op, _) = fx.op("CreateStageBranch");
    let effects = fx.done(op, OpResult::StageCreated);
    let (op, _) = commits(&effects).remove(0);
    let result = OpResult::DocsCommitted {
        head: ROUND_DOCS.into(),
        spec: "docs/anthrex/specs/1970-01-01-password-reset.md".into(),
    };
    let mut all = fx.done(op, result);
    all.extend(fx.tick());
    let (op, kind) = fx.op("CreateStageBranch");
    assert!(format!("{kind:?}").contains("stage-3"), "{kind:?}");
    all.extend(fx.done(op, OpResult::StageCreated));
    let run = fx.run();
    assert_eq!(
        run.stage(3).unwrap().created_from,
        ROUND_DOCS,
        "the same head"
    );
    assert_eq!(run.rounds[1].committed_stage, Some(2));
    assert!(reply(&cancel(&mut fx)).is_ok());
    for _ in 0..3 {
        all.extend(fx.tick());
    }
    let skipped = |n: u16| fx.run().delivery.stage(n).is_some_and(|s| s.skipped);
    assert!(skipped(3), "{:#?}", fx.run().log);
    assert!(!skipped(2), "{:#?}", fx.run().log);
    assert!(!opened(&all).contains(&3), "{all:?}");
}

/// The failed reply of a round's documents commit.
fn failed(fx: &mut Fixture, op: u64) -> Vec<Effect> {
    let message = "git commit-tree: fatal: bad object".to_string();
    fx.done(op, OpResult::Failed { message })
}

/// Ruling T15-16 (R2): round 2 cancelled with its commit in flight, then the commit
/// fails. Nothing landed, so the round is dropped as a reject drops it: no halt, the
/// commit no longer due, the design state restored, the round record dropped; and a
/// merge runs after.
#[test]
fn a_cancelled_rounds_failed_commit_drops_the_round() {
    let (mut fx, op, _) = approved_round(design_complete());
    let round = fx
        .run()
        .orch
        .design
        .as_ref()
        .unwrap()
        .round
        .clone()
        .unwrap();
    assert!(reply(&cancel(&mut fx)).is_ok());
    failed(&mut fx, op);
    let run = fx.run();
    assert_ne!(run.state, RunState::Halted, "{:#?}", run.log);
    let design = run.orch.design.as_ref().unwrap();
    assert!(!design.commit_due);
    assert_eq!(design.approved_spec, round.spec_before);
    assert_eq!(design.requirements, round.base);
    assert!(run.rounds[1].dropped);
    assert_eq!(
        (run.rounds[1].approved_at, run.rounds[1].committed_stage),
        (None, None)
    );
    let line = "round 2's documents commit failed after its cancel: git commit-tree: fatal: bad object; the round is dropped";
    assert!(run.log.iter().any(|l| l.text == line), "{:#?}", run.log);
    merged_fix(&mut fx);
    assert_eq!(fx.task("t2").state, TaskState::Cancelled);
}

/// Decision 23, unchanged by ruling T15-16: a round that was not cancelled halts on its
/// commit's failure, retryably, the commit still due.
#[test]
fn an_open_rounds_failed_commit_still_halts() {
    let (mut fx, op, _) = approved_round(design_complete());
    failed(&mut fx, op);
    let run = fx.run();
    assert_eq!(run.state, RunState::Halted);
    assert!(run.halt_retryable);
    assert_eq!(
        run.halted_reason.as_deref(),
        Some("design flow: could not commit the spec and plan: git commit-tree: fatal: bad object")
    );
    assert!(run.orch.design.as_ref().unwrap().commit_due);
    assert!(!run.rounds[1].dropped);
}

/// The final fix wave's FW-16 (re-review m2): ruling T15-15 through 9.2's `open::empty`
/// directly, with no round cancel. Round 2's independent `t3` (stage 3, created at the
/// amendment's head) is cancelled on its own; at the next ticks stage 3 is skipped as
/// empty, with no `OpenPr`, and stage 2, holding the amendment, is not.
#[test]
fn an_emptied_stage_above_the_amendment_is_skipped_without_a_cancel() {
    let mut t3 = round_task("t3", &["R3"]);
    t3["task"]["stage"] = json!(3);
    let edits = json!([round_task("t2", &["R2", "R3"]), t3]);
    let mut fx = plan_gate_of(pr_complete(), edits);
    approve(&mut fx);
    let (op, _) = fx.op("CreateStageBranch");
    let effects = fx.done(op, OpResult::StageCreated);
    let (op, _) = commits(&effects).remove(0);
    let result = OpResult::DocsCommitted {
        head: ROUND_DOCS.into(),
        spec: "docs/anthrex/specs/1970-01-01-password-reset.md".into(),
    };
    let mut all = fx.done(op, result);
    all.extend(fx.tick());
    let (op, _) = fx.op("CreateStageBranch");
    all.extend(fx.done(op, OpResult::StageCreated));
    let cancel = proto::PlanEdit::CancelTask {
        task_id: "t3".into(),
    };
    assert!(reply(&super::dispatch::edit(&mut fx, vec![cancel])).is_ok());
    assert_eq!(fx.task("t3").state, TaskState::Cancelled);
    assert!(!fx.run().rounds[1].dropped, "no round was cancelled");
    // Stage 2's PR open at its head (as `pr_complete` opens stage 1's), so stage 3 is
    // ready: its one task is finished and the stage below has its PR.
    let head = fx.run().stage_head(2).unwrap().to_string();
    let mut pr = super::delivery_land::pr_record(13, proto::PrState::Open);
    pr.pushed_head = head;
    let run = fx.run_mut();
    run.delivery.stages.resize(2, Default::default());
    run.delivery.stages[1].pr = Some(pr);
    for _ in 0..3 {
        all.extend(fx.tick());
    }
    let skipped = |n: u16| fx.run().delivery.stage(n).is_some_and(|s| s.skipped);
    assert!(skipped(3), "{:#?}", fx.run().log);
    assert!(!skipped(2), "{:#?}", fx.run().log);
    assert!(!opened(&all).contains(&3), "{all:?}");
    let line = "stage 3: skipped (no changes)";
    assert!(
        fx.run().log.iter().any(|l| l.text == line),
        "{:#?}",
        fx.run().log
    );
}
