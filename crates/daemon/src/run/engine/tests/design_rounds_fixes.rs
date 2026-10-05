//! Milestone 9.6 task M9.6.15, fix round 1 (rulings T15-1 and T15-2): a later round's
//! pack carries every approved spec of the run, round 1's then each amendment; and a
//! cancelled round drops itself as a rejected one does, its plan review included.

use proto::{DocGateAction, DocGateKind, RoundDesign, RoundOutcome, RunState};
use serde_json::json;

use super::design_fixture::*;
use super::design_rounds::round_task;
use super::design_rounds_commit::{approved_round, assert_committed};
use super::design_rounds_fixture::*;
use super::fixture::*;
use super::goal_rounds_cancel::settle;
use super::goal_rounds_start::{reply, started};
use super::kinds_cancel::cancel;
use super::kinds_integration::{C1, C2, merge_real};
use super::orch_restore::{restart, resume};
use crate::run::engine::OpResult;

/// 9.3's answer to a round's cancel.
const CANCELLED: &str = "run 3f9a round 2 cancelled; it ends once its sessions have ended";

/// Round 2 (`amend`) approved, committed, its task merged: the run is complete again.
fn round_two_complete() -> Fixture {
    let (mut fx, op, _) = approved_round(design_complete());
    assert_committed(&mut fx, op, C1);
    merge_real(&mut fx, "t2", C2);
    let (op, _) = fx.op("VerifyRefs");
    fx.done(op, OpResult::RefsOk);
    assert_eq!(fx.run().state, RunState::Complete, "{:#?}", fx.run().log);
    fx
}

/// Ruling T15-1: round 3 brainstorms (`full`); its pack's earlier spec is this run's
/// round 1 spec, then round 2's approved amendment, never the amendment alone.
#[test]
fn a_full_rounds_pack_carries_round_ones_spec_and_every_amendment() {
    let mut fx = round_two_complete();
    assert_eq!(iterate_with(&mut fx, Some(RoundDesign::Full)), started(3));
    start_brainstorm(&mut fx);
    let design = fx.run().orch.design.as_ref().unwrap();
    let earlier = (design.pack.as_ref())
        .and_then(|p| p.earlier.clone())
        .expect("an earlier spec");
    assert_eq!(earlier.run, RUN_ID);
    assert_eq!(earlier.version.n, 1, "round 1's spec first");
    let amendments: Vec<(u32, u32)> = (earlier.amendments.iter())
        .map(|a| (a.round, a.version.n))
        .collect();
    assert_eq!(amendments, vec![(2, 2)]);
    assert!(earlier.path.ends_with("design/spec-v1.md"), "{earlier:?}");
    assert!(earlier.amendments[0].path.ends_with("design/spec-v2.md"));
}

/// Ruling T15-1: a rejected round adds no amendment: round 3's pack, after round 3's
/// own amendment was rejected and round 4 brainstorms, still holds round 2's only.
#[test]
fn a_rejected_rounds_amendment_is_never_carried() {
    let mut fx = round_two_complete();
    iterate_with(&mut fx, None).unwrap();
    let amendment = AMENDMENT.replace("R3 The app opens", "R4 The app opens");
    let amendment = amendment.replace("R2 Links are", "R3 Links are");
    submit_amendment(&mut fx, &amendment).unwrap();
    act(&mut fx, DocGateKind::Spec, DocGateAction::Reject).unwrap();
    assert_eq!(fx.run().state, RunState::Complete);
    assert_eq!(iterate_with(&mut fx, Some(RoundDesign::Full)), started(4));
    start_brainstorm(&mut fx);
    let design = fx.run().orch.design.as_ref().unwrap();
    let earlier = (design.pack.as_ref()).and_then(|p| p.earlier.clone());
    let earlier = earlier.expect("an earlier spec");
    let amendments: Vec<(u32, u32)> = (earlier.amendments.iter())
        .map(|a| (a.round, a.version.n))
        .collect();
    assert_eq!((earlier.version.n, amendments), (1, vec![(2, 2)]));
}

/// Ruling T15-2 and m7: the design state a dropped round leaves, as the round's start
/// recorded it, with none of the round's reviews.
fn assert_dropped(fx: &Fixture, before: &crate::run::design::round::DesignRound) {
    let design = fx.run().orch.design.as_ref().unwrap();
    assert_eq!(design.requirements, before.base);
    assert_eq!(design.approved_spec, before.spec_before);
    assert!(design.gate.is_none(), "{:?}", design.gate);
    assert!(design.round.is_none());
    assert!(design.phase_started.is_none(), "no clock");
    assert!(!design.commit_due);
    assert_eq!(design.reviews.len(), before.reviews_before, "m7");
    let live = (design.brainstormers.iter().chain(&design.reviewer))
        .filter(|a| a.state == crate::run::design::state::DesignAgentState::Running)
        .count();
    assert_eq!(live, 0, "the round's agents stop");
}

/// The round's start, as recorded.
fn round_of(fx: &Fixture) -> crate::run::design::round::DesignRound {
    let design = fx.run().orch.design.as_ref().unwrap();
    design.round.clone().expect("a round")
}

/// Ruling T15-2: `run cancel` of round 2 paused while it specifies (a daemon restart;
/// 9.3's cancel takes a running, paused or halted run, so a round is cancelled in its
/// phase this way) restores the design state as a reject does; the round ends
/// cancelled and the run is complete again.
#[test]
fn cancelling_a_round_while_it_specifies_drops_the_round() {
    let mut fx = design_complete();
    iterate_with(&mut fx, None).unwrap();
    let before = round_of(&fx);
    restart(&mut fx);
    assert_eq!(fx.run().state, RunState::Paused);
    assert_eq!(fx.run().paused_from, Some(RunState::Specifying));
    assert_eq!(reply(&cancel(&mut fx)), Ok(CANCELLED.into()));
    assert_dropped(&fx, &before);
    settle(&mut fx);
    let run = fx.run();
    assert_eq!(run.rounds[1].outcome, Some(RoundOutcome::Cancelled));
    assert_eq!(run.state, RunState::Complete, "{:#?}", run.log);
}

/// Ruling T15-2 and m7: `run cancel` of round 2 halted at its plan gate, its plan review
/// taken, restores the design state and drops the round's review, so round 3's digest
/// and checks never read it; the halt no longer returns to the dropped gate, and the
/// resume completes the run.
#[test]
fn cancelling_a_round_at_its_plan_gate_drops_the_round_and_its_review() {
    let mut fx = round_plan_gate(json!([round_task("t2", &["R2", "R3"])]));
    let before = round_of(&fx);
    assert!(fx.run().orch.design.as_ref().unwrap().reviews.len() > before.reviews_before);
    let now = fx.now;
    crate::run::engine::merge::halt(fx.run_mut(), "remote stage branch moved".into(), now);
    fx.run_mut().halt_retryable = true;
    assert_eq!(reply(&cancel(&mut fx)), Ok(CANCELLED.into()));
    assert_dropped(&fx, &before);
    let design = fx.run().orch.design.as_ref().unwrap();
    assert_eq!(design.halted_from, None);
    assert!(reply(&resume(&mut fx)).is_ok());
    settle(&mut fx);
    let run = fx.run();
    assert_eq!(run.rounds[1].outcome, Some(RoundOutcome::Cancelled));
    assert_eq!(run.state, RunState::Complete, "{:#?}", run.log);
}
