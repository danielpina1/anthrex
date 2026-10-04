//! Milestone 9.5 task M9.5.17a, parts 2 and 3: each lane runs every pre-merge gate on
//! its own (ruling RR-3), bounces at rung 1 on its own and leaves the race past it
//! (decision 20); the first lane past its last gate is `Won`, then crowned, and the
//! crown makes the task the lane's (decision 21, ruling T1-3); and every worker check
//! sees the writing lane (decision 23, ruling RR-9).

use proto::{AgentRole, LaneState, RaceLane, Runtime, TaskState};
use serde_json::json;

use super::dispatch::{replies, task_path};
use super::fixture::*;
use super::gates::{check_result, only_op};
use super::kinds::approve;
use super::race::{
    RACING, all_ops, claim, lane, lane_branch, lane_of, lane_path, launched, racing, window,
};
use super::tiers::{outcome, tiered};
use crate::run::engine::{AgentSignal, Effect, OpKind, OpResult};
use crate::run::model::task_branch;

const A: RaceLane = RaceLane::A;
const B: RaceLane = RaceLane::B;
/// Lane b's head, distinct from lane a's (`HEAD`).
pub(super) const HEAD_B: &str = "e2e2e2e2e2e2e2e2e2e2e2e2e2e2e2e2e2e2e2e2";

pub(super) fn proof(ok: bool) -> OpResult {
    OpResult::Proof {
        red_failed: true,
        head_passed: ok,
        matched: ok,
        red_tail: "red".into(),
        head_tail: "head".into(),
    }
}

/// The path of an op that names one.
fn op_path(kind: &OpKind) -> std::path::PathBuf {
    match kind {
        OpKind::Proof { path, .. } | OpKind::PrepareReview { path, .. } => path.clone(),
        OpKind::Check { dir, .. } => dir.clone(),
        OpKind::Tier(spec) => spec.dir.clone(),
        other => panic!("no path in {other:?}"),
    }
}

/// Lane `l` of `t1`, at `head`, from its claim through proof and check to its
/// reviewer: the reviewer's window. Every op on the way is the lane's.
fn to_review(fx: &mut Fixture, l: RaceLane, head: &str) -> u32 {
    let window = window(fx, l);
    let effects = claim(fx, l, window, head);
    let (op, kind) = only_op(&effects, "Proof");
    assert_eq!((lane_of(fx, op), op_path(&kind)), (Some(l), proof_path(l)));
    let effects = fx.done(op, proof(true));
    let (op, kind) = only_op(&effects, "Check");
    assert_eq!((lane_of(fx, op), op_path(&kind)), (Some(l), proof_path(l)));
    let effects = fx.done(op, check_result(true));
    assert_eq!(lane(fx, l).state, LaneState::Review);
    let (op, kind) = only_op(&effects, "PrepareReview");
    assert_eq!((lane_of(fx, op), op_path(&kind)), (Some(l), review_path(l)));
    let review = OpResult::Review {
        base: BASE.into(),
        head: head.into(),
        patch: "diff --git a/x b/x".into(),
    };
    let effects = fx.done(op, review);
    let (op, kind) = only_op(&effects, "CreateWindow");
    assert_eq!(lane_of(fx, op), Some(l));
    match kind {
        OpKind::CreateWindow { name, spec, .. } => {
            assert_eq!(name, format!("{H4}/t1.{}r1", l.label()));
            assert_eq!(spec.run_ref.and_then(|r| r.lane), Some(l));
        }
        other => panic!("{other:?}"),
    }
    let windows = fx.complete_windows();
    windows[0].1
}

fn proof_path(l: RaceLane) -> std::path::PathBuf {
    task_path(&format!("t1.{}.proof", l.label()))
}

fn review_path(l: RaceLane) -> std::path::PathBuf {
    task_path(&format!("t1.{}.review", l.label()))
}

pub(super) fn submit(fx: &mut Fixture, rwindow: u32, args: serde_json::Value) -> Vec<Effect> {
    fx.tool_as(AgentRole::Reviewer, rwindow, "t1", "submit_review", args)
}

/// Lane `l` passes every gate, its reviewer approving: the step's effects.
pub(super) fn passes(fx: &mut Fixture, l: RaceLane, head: &str) -> Vec<Effect> {
    let rwindow = to_review(fx, l, head);
    submit(fx, rwindow, approve())
}

#[test]
fn each_lane_runs_every_pre_merge_gate() {
    let (mut fx, _, _) = racing();
    let effects = passes(&mut fx, A, HEAD);
    assert_eq!(lane(&fx, B).state, LaneState::Working, "lane b races on");
    // No tier 2 (the candidate) before a crown: the lane is `Won`, then crowned.
    assert!(fx.ops("MergeCandidate").is_empty());
    assert!(fx.run().merge_queue.is_empty());
    assert_eq!(lane(&fx, A).state, LaneState::Won);
    let (op, kind) = only_op(&effects, "CrownRacer");
    assert_eq!(lane_of(&fx, op), Some(A));
    assert_eq!(
        kind,
        OpKind::CrownRacer {
            root: "/tmp/x".into(),
            task_branch: task_branch(RUN_ID, "t1"),
            lane_head: HEAD.into(),
            adopt: false,
            checkout: lane_path(A),
        }
    );
    // Each lane's records are its own.
    let t1 = fx.task("t1");
    assert!(t1.proofs.iter().all(|p| p.lane == Some(A)) && t1.proofs.len() == 1);
    assert!(t1.checks.iter().all(|c| c.lane == Some(A)) && t1.checks.len() == 1);
    assert!(t1.reviews.iter().all(|r| r.lane == Some(A)) && t1.reviews.len() == 1);
}

/// The same on a tiered profile: tier 1 is the lane's check, and its claim carries the
/// test-weakening signals' spec, each in the lane's checkouts.
#[test]
fn each_lane_runs_tier_1_and_the_signals_on_a_tiered_profile() {
    let profile = tiered().replace("setup = ", "test_paths = [\"tests/**\"]\nsetup = ");
    let mut fx = launched(
        &profile,
        &[task("t1", "M", "a", RACING)],
        config::Orchestrator::default(),
    );
    let window = window(&fx, B);
    let effects = fx.tool_as(
        AgentRole::Racer,
        window,
        "t1",
        "task_done",
        super::race::tdd(),
    );
    let (op, kind) = only_op(&effects, "VerifyDone");
    match &kind {
        OpKind::VerifyDone {
            worktree, signals, ..
        } => {
            assert_eq!(worktree, &lane_path(B));
            assert!(
                signals.is_some(),
                "the weakening signals are read: {kind:?}"
            );
        }
        other => panic!("{other:?}"),
    }
    let effects = fx.done(op, super::race::lane_check(&fx, B, HEAD_B));
    let (op, _) = only_op(&effects, "Proof");
    let effects = fx.done(op, proof(true));
    let (op, kind) = only_op(&effects, "Tier");
    assert_eq!((lane_of(&fx, op), op_path(&kind)), (Some(B), proof_path(B)));
    match &kind {
        OpKind::Tier(spec) => {
            assert_eq!(spec.tier, 1);
            assert_eq!(
                spec.scratch.as_ref().map(|s| s.commit.as_str()),
                Some(HEAD_B)
            );
        }
        other => panic!("{other:?}"),
    }
    let effects = fx.done(op, OpResult::Tier(Box::new(outcome(1, true, false))));
    assert_eq!(lane(&fx, B).state, LaneState::Review);
    let (op, _) = only_op(&effects, "PrepareReview");
    assert_eq!(lane_of(&fx, op), Some(B));
    assert!(
        !all_ops(&fx)
            .iter()
            .any(|k| matches!(k, OpKind::Tier(spec) if spec.tier == 2)),
        "no tier 2 before a crown"
    );
}

#[test]
fn a_lane_bounces_at_rung_1_on_its_own() {
    let (mut fx, a, b) = racing();
    let effects = claim(&mut fx, A, a, HEAD);
    let (op, _) = only_op(&effects, "Proof");
    fx.done(op, proof(false));
    let la = lane(&fx, A);
    assert_eq!(
        (la.state, la.failures, la.bounces.proof),
        (LaneState::Working, 1, 1)
    );
    assert_eq!(la.gates.rung, 1);
    let lb = lane(&fx, B);
    assert_eq!((lb.state, lb.failures), (LaneState::Working, 0));
    let t1 = fx.task("t1");
    assert_eq!((t1.state, t1.failures, t1.rung), (TaskState::Working, 0, 0));
    // The failure goes to lane a's racer, and only to it.
    let delivered: Vec<u32> = (fx.log.iter())
        .filter_map(|e| match e {
            Effect::Deliver {
                window_id, text, ..
            } if text.contains("proof") => Some(*window_id),
            _ => None,
        })
        .collect();
    assert_eq!(delivered, [a]);
    assert!(fx.run().outbox.iter().all(|m| m.window_id != b));
}

/// A lane's second gate failure: rung 2 for a single worker, so the lane is out while
/// the other races on. Nothing of the task's own changes: no fresh session, no new
/// route, no size raised.
#[test]
fn a_second_failure_eliminates_a_lane_while_the_other_lives() {
    let (mut fx, a, b) = racing();
    let route = fx.task("t1").route.clone();
    for _ in 0..2 {
        let effects = claim(&mut fx, A, a, HEAD);
        let (op, _) = only_op(&effects, "Proof");
        fx.done(op, proof(false));
    }
    let la = lane(&fx, A);
    assert_eq!(la.state, LaneState::Out);
    let reason = la.reason.clone().expect("a reason");
    assert!(
        reason.starts_with("the proof gate failed again"),
        "{reason}"
    );
    assert_eq!(la.gates.rung, 2);
    assert_eq!(
        la.gates.block, None,
        "minor m7: the rung says why, not a block reason"
    );
    let kills: Vec<u32> = (fx.log.iter())
        .filter_map(|e| match e {
            Effect::KillWindow { window_id } => Some(*window_id),
            _ => None,
        })
        .collect();
    assert_eq!(kills, [a]);
    let line = format!("race t1: racer a out: {reason}");
    assert_eq!(fx.run().log.iter().filter(|l| l.text == line).count(), 1);
    assert_eq!(lane(&fx, B).state, LaneState::Working);
    let t1 = fx.task("t1");
    assert_eq!(
        (t1.state, t1.size, &t1.route),
        (TaskState::Working, proto::Size::M, &route)
    );
    assert!(t1.fresh_session.is_none() && fx.ops("DiffSoFar").is_empty());
    // Lane b still works: its claim goes through.
    claim(&mut fx, B, b, HEAD_B);
    assert_eq!(lane(&fx, B).state, LaneState::Proof);
    // Lane a's racer is refused from now on.
    let effects = fx.tool_as(AgentRole::Racer, a, "t1", "task_done", super::race::tdd());
    let text = format!("racer a of task t1 is out: {reason}. Stop now.");
    assert_eq!(replies(&effects), vec![Err(text)]);
}

#[test]
fn task_blocked_in_a_lane_eliminates_it() {
    let (mut fx, a, b) = racing();
    let args = json!({"kind": "question", "reason": "which API?"});
    fx.tool_as(AgentRole::Racer, b, "t1", "task_blocked", args);
    let lb = lane(&fx, B);
    assert_eq!(
        (lb.state, lb.reason.as_deref()),
        (LaneState::Out, Some("which API?"))
    );
    assert_eq!(
        lb.gates.block.as_ref().map(|b| b.reason),
        Some(proto::BlockReason::Question)
    );
    assert!(
        fx.log
            .iter()
            .any(|e| matches!(e, Effect::KillWindow { window_id } if *window_id == b))
    );
    let t1 = fx.task("t1");
    assert_eq!((t1.state, t1.block.is_none()), (TaskState::Working, true));
    assert_eq!(lane(&fx, A).state, LaneState::Working);
    assert!(
        !fx.log
            .iter()
            .any(|e| matches!(e, Effect::KillWindow { window_id } if *window_id == a))
    );
}

/// Decision 16 and ruling RC-1, the racer case of task M9.5.13's counting test.
#[test]
fn a_lanes_rate_limit_counts_for_its_runtime() {
    let (mut fx, _, b) = racing();
    fx.signal(
        b,
        AgentSignal::ApiRetry {
            error: "rate_limit".into(),
            delay_ms: 1_000,
        },
    );
    let run = fx.run();
    assert_eq!(run.rate_limits.get("codex"), Some(&1));
    assert_eq!(run.rate_limits.get("claude"), None);
    assert_eq!(run.concurrency.get("codex").map(|c| c.cap), Some(1));
    assert_eq!(
        crate::run::engine::concurrency::cap(run, Runtime::Claude),
        3
    );
}

#[test]
fn race_rounds_have_unique_keys() {
    let (mut fx, _, _) = racing();
    to_review(&mut fx, A, HEAD);
    to_review(&mut fx, B, HEAD_B);
    let t1 = fx.task("t1");
    let keys: Vec<_> = (t1.rounds.iter())
        .map(|r| (r.role, r.session, r.round, r.lane))
        .collect();
    let mut unique = keys.clone();
    unique.dedup();
    unique.sort_by_key(|k| format!("{k:?}"));
    unique.dedup();
    assert_eq!(unique.len(), keys.len(), "{keys:?}");
    let names: Vec<String> = (fx.ops("CreateWindow").into_iter())
        .map(|(_, k)| match k {
            OpKind::CreateWindow { name, .. } => name,
            other => panic!("{other:?}"),
        })
        .collect();
    assert_eq!(
        names,
        ["t1.aw1", "t1.bw2", "t1.ar1", "t1.br1"].map(|n| format!("{H4}/{n}"))
    );
}

/// Ruling T1-3: every op of lane b names lane b's checkout (or its proof and review
/// checkouts) and branch; no op of either lane names the task's own before the crown.
#[test]
fn lane_ops_name_the_lanes_checkout() {
    let (mut fx, _, _) = racing();
    to_review(&mut fx, B, HEAD_B);
    let (own_path, own_branch) = (task_path("t1"), task_branch(RUN_ID, "t1"));
    let b = lane_path(B).display().to_string();
    for kind in all_ops(&fx) {
        let text = format!("{kind:?}");
        match &kind {
            OpKind::PrepareWorktree { path, branch, .. } if path == &lane_path(B) => {
                assert_eq!(branch, &lane_branch(B))
            }
            OpKind::CreateWindow { worktree, spec, .. } if worktree.starts_with(&b) => {
                assert!(spec.cwd.starts_with(&b), "{text}")
            }
            OpKind::VerifyDone { worktree, .. } => assert_eq!(worktree, &lane_path(B)),
            OpKind::Proof { .. } | OpKind::Check { .. } => {
                assert_eq!(op_path(&kind), proof_path(B))
            }
            OpKind::PrepareReview { .. } => assert_eq!(op_path(&kind), review_path(B)),
            _ => {}
        }
        assert!(!text.contains(&format!("{own_branch}\"")), "{text}");
        assert!(
            !text.contains(&format!("{}\"", own_path.display())),
            "{text}"
        );
    }
}

/// Part 3, the `AgentRole::Worker` audit (decision 23, ruling RR-9): each worker check
/// acts on the racer of the lane whose view it runs in. Once a lane is crowned, on that
/// lane's racer alone: `race_crown.rs`'s `worker_checks_see_the_crowned_lane`.
#[test]
fn worker_checks_see_the_writing_lane() {
    let (mut fx, a, b) = racing();
    // The stall clock: lane a's silence nudges lane a's racer only.
    let (stall, t0) = (fx.run().limits.stall_after_secs, fx.now);
    let activity = crate::run::engine::EventKind::Signal {
        window_id: b,
        signal: AgentSignal::Activity,
    };
    fx.send(t0 + stall - 5, activity);
    let effects = fx.send(t0 + stall + 2, crate::run::engine::EventKind::Tick);
    let interrupted: Vec<u32> = (effects.iter())
        .filter_map(|e| match e {
            Effect::Interrupt { window_id } => Some(*window_id),
            _ => None,
        })
        .collect();
    assert_eq!(interrupted, [a], "{effects:#?}");
    // A tool call from lane b is lane b's: its claim checks lane b's checkout.
    let effects = fx.tool_as(AgentRole::Racer, b, "t1", "task_done", super::race::tdd());
    let (op, _) = only_op(&effects, "VerifyDone");
    assert_eq!(lane_of(&fx, op), Some(B));
}
