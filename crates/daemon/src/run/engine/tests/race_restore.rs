//! The final fix wave's A-I1: a daemon restart resets each lane's gate state exactly as
//! it resets the task's. A lane's gate op the journal did not replay is issued again,
//! a lane's claim in flight is dropped (its racer's next `task_done` is checked), and
//! the downtime is never racer time.

use proto::{AgentRole, LaneState, RaceLane, TaskState};

use super::control::resume;
use super::control_restore::{restart, resumes};
use super::fixture::*;
use super::gates::{check_result, only_op};
use super::merge::{commit, merge, pending, pending_one};
use super::race::{claim, lane, racing, tdd};
use super::race_crown::crowned;
use super::race_end::unreviewed;
use super::race_end_requests::with_codex_session;
use super::race_lanes::{HEAD_B, proof};
use super::turns::killed_exit;
use crate::run::engine::{Effect, EngineState, EventKind, OpResult};
use crate::run::model::OpId;

const A: RaceLane = RaceLane::A;
const B: RaceLane = RaceLane::B;

/// Every pending `ResumeSession` answered `Resumed`.
fn all_resumed(fx: &mut Fixture) {
    for (op, _) in pending(fx, "ResumeSession", Some("t1")) {
        fx.done(op, OpResult::Resumed);
    }
}

/// The lane a pending op was sent for.
fn op_lane(fx: &Fixture, op: OpId) -> Option<RaceLane> {
    fx.run().pending_ops.get(&op).and_then(|p| p.lane)
}

/// A-I1 (a): both lanes are in their proof when the daemon restarts, and the journal
/// replays neither. Each lane's proof is sent again, both lanes go on, the race is
/// crowned and the run completes.
#[test]
fn a_restart_while_both_lanes_are_in_proof_sends_each_proof_again() {
    let (mut fx, a, b) = unreviewed();
    with_codex_session(&mut fx);
    for (l, window, head) in [(A, a, HEAD), (B, b, HEAD_B)] {
        let effects = claim(&mut fx, l, window, head);
        only_op(&effects, "Proof");
        assert_eq!(lane(&fx, l).state, LaneState::Proof);
    }
    restart(&mut fx, Vec::new());
    let mut effects = resume(&mut fx);
    effects.extend(fx.tick());
    all_resumed(&mut fx);
    let proofs = ops_in(&effects, "Proof");
    let mut lanes: Vec<Option<RaceLane>> = proofs.iter().map(|(op, _)| op_lane(&fx, *op)).collect();
    lanes.sort();
    assert_eq!(lanes, [Some(A), Some(B)], "{effects:#?}");
    // Lane b passes first and wins; lane a is lost.
    let proof_b = proofs.iter().find(|(op, _)| op_lane(&fx, *op) == Some(B));
    let effects = fx.done(proof_b.expect("lane b's proof").0, proof(true));
    let (op, _) = only_op(&effects, "Check");
    let effects = fx.done(op, check_result(true));
    let (crown, _) = only_op(&effects, "CrownRacer");
    assert_eq!(lane(&fx, A).state, LaneState::Lost);
    let proof_a = proofs.iter().find(|(op, _)| op_lane(&fx, *op) == Some(A));
    fx.done(proof_a.expect("lane a's proof").0, proof(true));
    killed_exit(&mut fx, a);
    crowned(&mut fx, crown, HEAD_B);
    assert_eq!(fx.task("t1").state, TaskState::MergeQueue);
    merge(&mut fx, "t1", &commit(1));
    assert_eq!(fx.task("t1").state, TaskState::Merged);
    for (op, _) in pending(&fx, "RemoveWorktree", Some("t1")) {
        let removed = OpResult::Removed {
            salvage_ref: None,
            cleared_locks: Vec::new(),
        };
        fx.done(op, removed);
    }
    fx.tick();
    pending_one(&fx, "VerifyRefs", None);
}

/// A-I1 (b): a lane's claim in flight when the daemon restarts is dropped with its
/// `VerifyDone`. The racer's next `task_done` is checked, not refused as already being
/// checked.
#[test]
fn a_lanes_claim_in_flight_at_a_restart_is_dropped() {
    let (mut fx, a, _) = racing();
    with_codex_session(&mut fx);
    let effects = fx.tool_as(AgentRole::Racer, a, "t1", "task_done", tdd());
    only_op(&effects, "VerifyDone");
    restart(&mut fx, Vec::new());
    assert!(
        lane(&fx, A).gates.claim.is_none(),
        "no claim outlives its session"
    );
    resume(&mut fx);
    all_resumed(&mut fx);
    fx.tick();
    let effects = fx.tool_as(AgentRole::Racer, a, "t1", "task_done", tdd());
    let (op, _) = only_op(&effects, "VerifyDone");
    assert_eq!(op_lane(&fx, op), Some(A));
    assert!(
        !format!("{effects:?}").contains("already being checked"),
        "{effects:#?}"
    );
}

/// A-I1 (c): two hours of downtime against the racers' budget. Each lane's clock stops
/// at the restore at its racer's last sign of life, so on resume neither racer is
/// charged the downtime, and neither lane breaches its budget.
#[test]
fn the_downtime_is_not_charged_to_the_racers() {
    let (mut fx, a, b) = racing();
    with_codex_session(&mut fx);
    let last = fx.now;
    let run = fx.run().clone();
    fx.state = EngineState::default();
    let back = last + 2 * 3_600;
    fx.send(
        back,
        EventKind::Restore {
            held: Vec::new(),
            runs: vec![run],
            replay: Vec::new(),
        },
    );
    for l in [A, B] {
        let stopped = lane(&fx, l).gates.clock.stopped;
        assert!(stopped.is_some_and(|at| at <= last), "{l:?}: {stopped:?}");
    }
    let effects = resume(&mut fx);
    let windows: Vec<u32> = resumes(&effects).into_iter().map(|(w, _, _)| w).collect();
    assert!(windows.contains(&a) && windows.contains(&b), "{effects:#?}");
    all_resumed(&mut fx);
    let effects = fx.tick();
    assert!(
        !effects
            .iter()
            .any(|e| matches!(e, Effect::KillWindow { .. })),
        "{effects:#?}"
    );
    for l in [A, B] {
        assert_eq!(lane(&fx, l).state, LaneState::Working, "{l:?}");
    }
    let t1 = fx.task("t1");
    for l in [A, B] {
        let racer = (t1.rounds.iter())
            .rfind(|r| r.role == AgentRole::Racer && r.lane == Some(l))
            .expect("a racer");
        let clock = lane(&fx, l).gates.clock;
        let secs = crate::run::engine::ladder::round_spend(racer, clock.stopped, fx.now).secs;
        assert!(secs < 600, "{l:?} charged {secs} s");
    }
}
