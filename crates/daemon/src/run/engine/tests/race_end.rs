//! Milestone 9.5 task M9.5.17b: how a race ends (decisions 20–22, rulings RR-1, RR-2).
//! The first lane past every gate wins and the other is lost at once; a lane that left
//! the race is stopped through `KillWindow` and salvaged only once its ops have returned
//! and its racer has exited, or kept when the racer does not exit; the last lane left
//! is adopted, crowned at its last head, and the ladder applies to the task; a cancel
//! stops and salvages both lanes. A racing task's requests are `race_end_requests.rs`.

use proto::{AgentRole, BlockReason, LaneState, PlanEdit, RaceLane, Size, TaskState};
use serde_json::json;

use super::dispatch::{edit, replies};
use super::fixture::*;
use super::gates::{check_result, only_op};
use super::kinds::approve;
use super::race::{RACING, claim, lane, lane_check, lane_path, launched, racing, tdd, window};
use super::race_crown::crowned;
use super::race_lanes::{HEAD_B, passes, proof, submit, to_review};
use super::turns::killed_exit;
use crate::run::engine::race_end::LANE_EXIT_WAIT_SECS;
use crate::run::engine::{Effect, EventKind, OpKind, OpResult};
use crate::run::model::{OpId, task_branch};

const A: RaceLane = RaceLane::A;
const B: RaceLane = RaceLane::B;

/// The windows `effects` kill, in order.
pub(super) fn kills(effects: &[Effect]) -> Vec<u32> {
    (effects.iter())
        .filter_map(|e| match e {
            Effect::KillWindow { window_id } => Some(*window_id),
            _ => None,
        })
        .collect()
}

pub(super) fn sorted(mut windows: Vec<u32>) -> Vec<u32> {
    windows.sort();
    windows
}

/// `t1`'s salvage ref number `n`.
pub(super) fn salvage(n: u32) -> String {
    format!("refs/anthrex/salvage/{RUN_ID}/t1/{n}")
}

/// A lane's salvage and removal (decision 22).
pub(super) fn removal(lane: RaceLane, n: u32, kept: bool) -> OpKind {
    OpKind::RemoveWorktree {
        root: "/tmp/x".into(),
        path: lane_path(lane),
        salvage_ref: salvage(n),
        keep_head: true,
        clear_locks: !kept,
        keep_path: kept,
    }
}

#[test]
fn the_first_lane_to_pass_every_gate_wins() {
    let (mut fx, a, _) = racing();
    let reviewer_a = to_review(&mut fx, A, HEAD);
    let effects = passes(&mut fx, B, HEAD_B);
    match only_op(&effects, "CrownRacer").1 {
        OpKind::CrownRacer {
            lane_head, adopt, ..
        } => assert_eq!((lane_head.as_str(), adopt), (HEAD_B, false)),
        other => panic!("{other:?}"),
    }
    let race = fx.task("t1").race.clone().expect("a race");
    assert_eq!(race.winner, Some(B));
    assert_eq!(
        (lane(&fx, A).state, lane(&fx, B).state),
        (LaneState::Lost, LaneState::Won)
    );
    assert_eq!(sorted(kills(&effects)), sorted(vec![a, reviewer_a]));
    assert!(lane(&fx, A).kill_sent_at.is_some());
    let line = "race t1: racer a lost: racer b won";
    assert!(
        fx.run().log.iter().any(|l| l.text == line),
        "{:#?}",
        fx.run().log
    );
}

/// Task 17a's review, m1: a lane whose review approves after the other lane won is
/// lost already; it is never reviewed again.
#[test]
fn a_lane_that_passes_after_the_win_gets_no_new_review_round() {
    let (mut fx, _, _) = racing();
    let reviewer_a = to_review(&mut fx, A, HEAD);
    let effects = passes(&mut fx, B, HEAD_B);
    let (op, _) = only_op(&effects, "CrownRacer");
    crowned(&mut fx, op, HEAD_B);
    let from = fx.log.len();
    submit(&mut fx, reviewer_a, approve());
    fx.tick();
    fx.tick();
    assert_eq!(lane(&fx, A).state, LaneState::Lost);
    let review_a = super::dispatch::task_path("t1.a.review");
    let later = &fx.log[from..];
    for (_, kind) in ops_in(later, "PrepareReview") {
        assert!(!format!("{kind:?}").contains(&review_a.display().to_string()));
    }
    let a_rounds = (fx.task("t1").rounds.iter())
        .filter(|r| r.role == AgentRole::Reviewer && r.lane == Some(A))
        .count();
    assert_eq!(a_rounds, 1, "lane a has its one reviewer round");
}

/// An unreviewed racing `t1` (S): each lane's last gate is its check.
pub(super) fn unreviewed() -> (Fixture, u32, u32) {
    let config = config::Orchestrator {
        review_small: false,
        ..Default::default()
    };
    let fx = launched(PROFILE, &[task("t1", "S", "a", RACING)], config);
    assert_eq!(fx.task("t1").review_level, None);
    let (a, b) = (window(&fx, A), window(&fx, B));
    (fx, a, b)
}

/// Lane `l` claims at `head` and passes its proof: its check's op.
pub(super) fn to_check(fx: &mut Fixture, l: RaceLane, window: u32, head: &str) -> OpId {
    let effects = claim(fx, l, window, head);
    let (op, _) = only_op(&effects, "Proof");
    let effects = fx.done(op, proof(true));
    only_op(&effects, "Check").0
}

/// Review focus 3: both lanes pass in one step (a new daemon replays both checks), and
/// exactly one is crowned (a pinning test: the reducer takes one event at a time).
#[test]
fn both_racers_passing_in_one_step_crowns_exactly_one() {
    let (mut fx, a, b) = unreviewed();
    let check_a = to_check(&mut fx, A, a, HEAD);
    let check_b = to_check(&mut fx, B, b, HEAD_B);
    let replay = vec![(check_b, check_result(true)), (check_a, check_result(true))];
    let effects = super::control_restore::restart(&mut fx, replay);
    let crowns = ops_in(&effects, "CrownRacer");
    assert_eq!(crowns.len(), 1, "{effects:#?}");
    assert!(matches!(&crowns[0].1, OpKind::CrownRacer { lane_head, .. } if lane_head == HEAD_B));
    assert_eq!(fx.task("t1").race.as_ref().and_then(|r| r.winner), Some(B));
    assert_eq!(lane(&fx, A).state, LaneState::Lost);
}

/// The pending-crown variant: lane a passes while lane b's crown is in flight.
#[test]
fn a_pass_while_the_crown_is_pending_crowns_nothing() {
    let (mut fx, a, b) = unreviewed();
    let check_a = to_check(&mut fx, A, a, HEAD);
    let check_b = to_check(&mut fx, B, b, HEAD_B);
    let effects = fx.done(check_b, check_result(true));
    only_op(&effects, "CrownRacer");
    let effects = fx.done(check_a, check_result(true));
    assert!(ops_in(&effects, "CrownRacer").is_empty());
    assert_eq!(fx.ops("CrownRacer").len(), 1);
    assert_eq!(lane(&fx, A).state, LaneState::Lost);
}

/// The `RefMoved` variant: the winner's crown finds the task branch moved and halts
/// the run; lane a's pass then crowns nothing, and a resume crowns lane b again.
#[test]
fn a_pass_after_a_moved_branch_crowns_nothing_and_a_resume_crowns_the_winner() {
    let (mut fx, a, b) = unreviewed();
    let check_a = to_check(&mut fx, A, a, HEAD);
    let check_b = to_check(&mut fx, B, b, HEAD_B);
    let effects = fx.done(check_b, check_result(true));
    let (crown, _) = only_op(&effects, "CrownRacer");
    let moved = OpResult::RefMoved {
        reason: "anthrex/x/t1 moved".into(),
    };
    fx.done(crown, moved);
    assert_eq!(fx.run().state, proto::RunState::Halted);
    let effects = fx.done(check_a, check_result(true));
    assert!(ops_in(&effects, "CrownRacer").is_empty());
    let reply = fx.reply();
    let mut effects = fx.next(EventKind::Resume {
        reply,
        run_id: RUN_ID.into(),
        rebaseline: Some((BASE.to_string(), BASE.to_string()).into()),
    });
    effects.extend(fx.tick());
    let crowns = ops_in(&effects, "CrownRacer");
    assert_eq!(crowns.len(), 1, "{effects:#?}");
    assert!(
        matches!(&crowns[0].1, OpKind::CrownRacer { checkout, .. } if *checkout == lane_path(B))
    );
}

/// Lane b wins while lane a's check is in flight: the fixture, lane a's racer window
/// and its check's op.
fn lost_with_a_pending_check() -> (Fixture, u32, OpId) {
    let (mut fx, a, _) = racing();
    let effects = claim(&mut fx, A, a, HEAD);
    let (op, _) = only_op(&effects, "Proof");
    let effects = fx.done(op, proof(true));
    let (check, _) = only_op(&effects, "Check");
    let effects = passes(&mut fx, B, HEAD_B);
    assert_eq!(lane(&fx, A).state, LaneState::Lost);
    assert_eq!(kills(&effects), [a]);
    assert!(ops_in(&effects, "RemoveWorktree").is_empty());
    (fx, a, check)
}

/// Ruling RR-2: the loser's checkout is salvaged and removed only once its pending op
/// has returned and its racer has exited, in either order; the removal's result lands
/// on the lane.
#[test]
fn the_loser_is_stopped_then_salvaged_after_its_ops_and_exit() {
    for exit_first in [false, true] {
        let (mut fx, a, check) = lost_with_a_pending_check();
        let effects = match exit_first {
            true => {
                let effects = killed_exit(&mut fx, a);
                assert!(ops_in(&effects, "RemoveWorktree").is_empty(), "its op");
                fx.done(check, check_result(true))
            }
            false => {
                let effects = fx.done(check, check_result(true));
                assert!(ops_in(&effects, "RemoveWorktree").is_empty(), "its exit");
                killed_exit(&mut fx, a)
            }
        };
        let (op, kind) = only_op(&effects, "RemoveWorktree");
        assert_eq!(kind, removal(A, 1, false));
        assert!(ops_in(&fx.tick(), "RemoveWorktree").is_empty(), "once");
        let removed = OpResult::Removed {
            salvage_ref: Some(salvage(1)),
            cleared_locks: vec!["index.lock".into()],
        };
        fx.done(op, removed);
        let la = lane(&fx, A);
        assert_eq!(la.salvage_ref, Some(salvage(1)));
        assert_eq!(la.cleared_locks, ["index.lock"]);
        assert!(la.removed && !la.kept);
        let t1 = fx.task("t1");
        assert_eq!(t1.salvage_refs, [salvage(1)]);
        let line = "racer a: removed a stale index.lock left by the stopped racer";
        assert!(
            t1.history.iter().any(|e| e.text == line),
            "{:#?}",
            t1.history
        );
    }
}

/// Ruling RR-2: a racer that does not exit within `LANE_EXIT_WAIT_SECS` of its
/// `KillWindow` keeps its checkout; salvage still runs, and no lock is cleared.
#[test]
fn a_loser_that_does_not_exit_keeps_its_checkout() {
    let (mut fx, _, _) = racing();
    passes(&mut fx, B, HEAD_B);
    let sent = lane(&fx, A).kill_sent_at.expect("lane a was stopped");
    let effects = fx.send(sent + LANE_EXIT_WAIT_SECS - 1, EventKind::Tick);
    assert!(ops_in(&effects, "RemoveWorktree").is_empty());
    let effects = fx.send(sent + LANE_EXIT_WAIT_SECS, EventKind::Tick);
    let (op, kind) = only_op(&effects, "RemoveWorktree");
    assert_eq!(kind, removal(A, 1, true));
    let line = "race t1: kept a checkout: its racer did not exit";
    assert!(
        fx.run().log.iter().any(|l| l.text == line),
        "{:#?}",
        fx.run().log
    );
    assert!(lane(&fx, A).kept);
    let removed = OpResult::Removed {
        salvage_ref: Some(salvage(1)),
        cleared_locks: Vec::new(),
    };
    fx.done(op, removed);
    let la = lane(&fx, A);
    assert_eq!((la.salvage_ref, la.removed), (Some(salvage(1)), false));
    assert!(ops_in(&fx.tick(), "RemoveWorktree").is_empty());
}

/// Decisions 21 and 23: once crowned, the task is an ordinary task in lane b's
/// checkout: a conflict hands back into it and lane b's session takes the conflict,
/// and the merge removes that checkout.
#[test]
fn after_the_crown_the_task_is_an_ordinary_task() {
    let (mut fx, _, b) = racing();
    let effects = passes(&mut fx, B, HEAD_B);
    let (op, _) = only_op(&effects, "CrownRacer");
    crowned(&mut fx, op, HEAD_B);
    let t1 = fx.task("t1");
    assert_eq!(t1.checkout_name(), "t1.b");
    assert_eq!(t1.branch, task_branch(RUN_ID, "t1"));
    assert_eq!(t1.state, TaskState::MergeQueue);
    let (op, head) = super::merge::candidate(&fx, "t1");
    assert_eq!(head, HEAD_B);
    let merged = OpResult::Merged {
        commit: "c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1".into(),
        tier: None,
    };
    let effects = fx.done(op, merged);
    let paths: Vec<String> = (ops_in(&effects, "RemoveWorktree").into_iter())
        .map(|(_, kind)| match kind {
            OpKind::RemoveWorktree { path, .. } => path.display().to_string(),
            other => panic!("{other:?}"),
        })
        .collect();
    assert!(
        paths.contains(&lane_path(B).display().to_string()),
        "{paths:?}"
    );
    // The conflict variant, on a second race.
    let (mut fx, _, _) = racing();
    let effects = passes(&mut fx, B, HEAD_B);
    let (op, _) = only_op(&effects, "CrownRacer");
    crowned(&mut fx, op, HEAD_B);
    let from = fx.log.len();
    super::race_crown::handed_back(&mut fx);
    let later = &fx.log[from..];
    let (_, kind) = ops_in(later, "HandBack")[0].clone();
    assert!(matches!(kind, OpKind::HandBack { worktree, .. } if worktree == lane_path(B)));
    let to_b = later.iter().any(|e| match e {
        Effect::Deliver { window_id, .. } => *window_id == b,
        Effect::Op {
            kind: OpKind::ResumeSession { window_id, .. },
            ..
        } => *window_id == b,
        _ => false,
    });
    assert!(to_b, "lane b's session takes the conflict: {later:#?}");
}

#[test]
fn a_losers_tool_call_is_refused_with_the_winner() {
    let (mut fx, a, _) = racing();
    passes(&mut fx, B, HEAD_B);
    let text = "the race for task t1 is over: racer b won. Stop now.".to_string();
    let effects = fx.tool_as(AgentRole::Racer, a, "t1", "task_done", tdd());
    assert_eq!(replies(&effects), vec![Err(text.clone())]);
    let args = json!({"kind": "question", "reason": "which API?"});
    let effects = fx.tool_as(AgentRole::Racer, a, "t1", "task_blocked", args);
    assert_eq!(replies(&effects), vec![Err(text)]);
}

/// Lane b out first (its racer asked a question), lane a racing on.
pub(super) fn b_out() -> (Fixture, u32, u32) {
    let (mut fx, a, b) = racing();
    let args = json!({"kind": "question", "reason": "which API?"});
    fx.tool_as(AgentRole::Racer, b, "t1", "task_blocked", args);
    assert_eq!(lane(&fx, B).state, LaneState::Out);
    (fx, a, b)
}

/// The adoption's crown among `effects`: its op and head.
fn adopted_crown(effects: &[Effect]) -> (OpId, String) {
    match only_op(effects, "CrownRacer") {
        (
            op,
            OpKind::CrownRacer {
                lane_head,
                adopt,
                checkout,
                ..
            },
        ) => {
            assert!(adopt);
            assert_eq!(checkout, lane_path(A));
            (op, lane_head)
        }
        (_, other) => panic!("{other:?}"),
    }
}

/// Decision 20, a second failure: lane a, the last lane left, is adopted at its last
/// imported head, and after the crown rung 2 starts a fresh session in its checkout.
#[test]
fn the_last_lane_out_after_a_second_failure_is_adopted_at_rung_2() {
    let (mut fx, a, _) = b_out();
    let mut last = Vec::new();
    for _ in 0..2 {
        let effects = claim(&mut fx, A, a, HEAD);
        let (op, _) = only_op(&effects, "Proof");
        last = fx.done(op, proof(false));
    }
    let (op, head) = adopted_crown(&last);
    assert_eq!(head, HEAD, "its last imported head");
    let race = fx.task("t1").race.clone().expect("a race");
    assert_eq!((race.winner, race.adopted), (Some(A), true));
    assert_eq!(lane(&fx, A).state, LaneState::Adopted);
    // Whole-branch review B, M6: Interfaces' exact words, as REPORT.md has them.
    let line = "race t1: racer a adopted after racer b went out";
    assert!(
        fx.run().log.iter().any(|l| l.text == line),
        "{:#?}",
        fx.run().log
    );
    crowned(&mut fx, op, HEAD);
    let t1 = fx.task("t1");
    assert_eq!((t1.state, t1.rung), (TaskState::Working, 2));
    assert_eq!(t1.worktree, lane_path(A));
    assert!(t1.fresh_session.is_some());
    let effects = killed_exit(&mut fx, a);
    let (op, kind) = only_op(&effects, "DiffSoFar");
    assert!(matches!(kind, OpKind::DiffSoFar { worktree, .. } if worktree == lane_path(A)));
    let diff = OpResult::Diff {
        stat: "1 file".into(),
        patch: String::new(),
    };
    let effects = fx.done(op, diff);
    match only_op(&effects, "CreateWindow").1 {
        OpKind::CreateWindow { worktree, spec, .. } => {
            assert_eq!(worktree, lane_path(A));
            assert_eq!(spec.mcp.map(|m| m.role), Some(AgentRole::Worker));
        }
        other => panic!("{other:?}"),
    }
}

/// Lane b out, then lane a's racer asks a question: lane a is adopted at the stage
/// head its checkout was prepared from (it has no imported head), its session kept.
pub(super) fn adopted_on_a_question() -> (Fixture, u32) {
    let (mut fx, a, _) = b_out();
    let args = json!({"kind": "question", "reason": "which schema?"});
    let effects = fx.tool_as(AgentRole::Racer, a, "t1", "task_blocked", args);
    assert!(
        !kills(&fx.log).contains(&a),
        "an adopted racer keeps its session"
    );
    let (op, head) = adopted_crown(&effects);
    assert_eq!(
        head, BASE,
        "no imported head: the stage head it was prepared from"
    );
    crowned(&mut fx, op, BASE);
    (fx, a)
}

/// Decision 20, a question: the adopted task is `blocked(question)`, and an answer
/// reaches lane a's racer.
#[test]
fn the_last_lane_out_on_a_question_is_adopted_and_blocked_on_it() {
    let (mut fx, a) = adopted_on_a_question();
    let t1 = fx.task("t1");
    assert_eq!(t1.state, TaskState::Blocked);
    let block = t1.block.clone().expect("a block");
    assert_eq!(
        (block.reason, block.text.as_str()),
        (BlockReason::Question, "which schema?")
    );
    fx.turn_completed(a);
    let from = fx.log.len();
    let effects = edit(&mut fx, vec![super::holds::answer("use v2")]);
    assert!(replies(&effects)[0].is_ok(), "{effects:#?}");
    fx.tick();
    let to_a = fx.log[from..].iter().any(|e| match e {
        Effect::Deliver { window_id, .. } => *window_id == a,
        Effect::Op {
            kind: OpKind::ResumeSession { window_id, .. },
            ..
        } => *window_id == a,
        _ => false,
    });
    assert!(to_a, "the answer resumes lane a: {:#?}", &fx.log[from..]);
}

/// Decision 20, a spill: rung 3 for the adopted task.
#[test]
fn the_last_lane_out_on_a_spill_is_adopted_at_rung_3() {
    let (mut fx, a, _) = b_out();
    let effects = fx.tool_as(AgentRole::Racer, a, "t1", "task_done", tdd());
    let (op, _) = only_op(&effects, "VerifyDone");
    let mut check = lane_check(&fx, A, HEAD);
    if let OpResult::DoneChecked { outside_owns, .. } = &mut check {
        *outside_owns = vec!["crates/b/src/x.rs".into()];
    }
    let effects = fx.done(op, check);
    let (op, head) = adopted_crown(&effects);
    assert_eq!(head, BASE);
    crowned(&mut fx, op, BASE);
    let t1 = fx.task("t1");
    assert_eq!(
        (t1.state, t1.rung, t1.size),
        (TaskState::Blocked, 3, Size::L)
    );
    let block = t1.block.clone().expect("a block");
    assert_eq!(block.reason, BlockReason::MisSized);
    assert!(
        block.text.starts_with("changed files outside owns"),
        "{}",
        block.text
    );
}

/// A racing `t1` whose two lane checkouts could not be prepared.
pub(super) fn never_prepared() -> Fixture {
    let mut fx = Fixture::new(&plan_with(PROFILE, &[task("t1", "M", "a", RACING)]));
    fx.ready(true);
    for (op, _) in fx.ops("PrepareWorktree") {
        let failed = OpResult::Failed {
            message: "disk full".into(),
        };
        fx.done(op, failed);
    }
    fx
}

/// Decision 20: a last lane whose checkout was never prepared is not crowned; the task
/// is `blocked(environment)` with the lane's reason.
#[test]
fn a_last_lane_never_prepared_is_not_crowned() {
    let fx = never_prepared();
    let t1 = fx.task("t1");
    assert_eq!(t1.state, TaskState::Blocked);
    let block = t1.block.clone().expect("a block");
    assert_eq!(block.reason, BlockReason::Environment);
    assert_eq!(block.text, "could not prepare the worktree: disk full");
    assert!(fx.ops("CrownRacer").is_empty());
    let race = t1.race.clone().expect("a race");
    assert_eq!(race.winner, None);
    assert!(race.lanes.iter().all(|l| l.state == LaneState::Out));
}

/// Task 17a's review, m8: the crown never carries an empty head. An adopted lane with
/// no imported head is crowned at its start commit; one with neither is not crowned.
#[test]
fn the_crown_never_carries_an_empty_head() {
    let (mut fx, _, _) = racing();
    let t1 = fx.task_mut("t1");
    let mut race = crate::run::test_support::race_of(t1, [LaneState::Adopted, LaneState::Out]);
    race.crowned = false;
    race.lanes[0].start_commit = Some(BASE.into());
    t1.race = Some(race);
    let (_, head) = adopted_crown(&fx.tick());
    assert_eq!(head, BASE);
    let (mut fx, _, _) = racing();
    let t1 = fx.task_mut("t1");
    let mut race = crate::run::test_support::race_of(t1, [LaneState::Adopted, LaneState::Out]);
    race.crowned = false;
    t1.race = Some(race);
    let effects = fx.tick();
    assert!(ops_in(&effects, "CrownRacer").is_empty(), "{effects:#?}");
    let t1 = fx.task("t1");
    assert_eq!(t1.state, TaskState::Blocked);
    let block = t1.block.clone().expect("a block");
    assert_eq!(block.reason, BlockReason::Environment);
    assert!(
        fx.ops("CrownRacer").is_empty(),
        "the next pass sends none either"
    );
    fx.tick();
    assert!(fx.ops("CrownRacer").is_empty());
}

/// Decision 23: a cancel stops both lanes' racers and salvages both checkouts, each
/// once its racer has exited.
#[test]
fn cancel_stops_and_salvages_both_lanes() {
    let (mut fx, a, b) = racing();
    let cancel = PlanEdit::CancelTask {
        task_id: "t1".into(),
    };
    let effects = edit(&mut fx, vec![cancel]);
    assert_eq!(fx.task("t1").state, TaskState::Cancelled);
    assert_eq!(sorted(kills(&effects)), sorted(vec![a, b]));
    assert!(ops_in(&effects, "RemoveWorktree").is_empty());
    let first = killed_exit(&mut fx, a);
    assert_eq!(only_op(&first, "RemoveWorktree").1, removal(A, 1, false));
    let second = killed_exit(&mut fx, b);
    assert_eq!(only_op(&second, "RemoveWorktree").1, removal(B, 2, false));
    assert_eq!(
        (lane(&fx, A).state, lane(&fx, B).state),
        (LaneState::Out, LaneState::Out)
    );
    assert_eq!(
        fx.ops("RemoveWorktree").len(),
        2,
        "the task's own was never prepared"
    );
}
