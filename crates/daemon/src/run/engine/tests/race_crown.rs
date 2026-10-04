//! Milestone 9.5 task M9.5.17a, part 2's crown and part 3 after it (split from
//! `race_lanes.rs` for size): the first lane past its last gate is `Won`, then crowned,
//! and the crown makes the task the lane's (decision 21, ruling T1-3); a moved task
//! branch halts the run and a resume crowns again; and once a lane is crowned the
//! task's worker checks see its racer alone (decision 23, ruling RR-9).

use proto::{AgentRole, LaneState, RaceLane, Runtime, TaskState};

use super::dispatch::{replies, task_path};
use super::fixture::*;
use super::gates::only_op;
use super::race::{lane, lane_path, racing};
use super::race_lanes::{HEAD_B, passes};
use crate::run::engine::{Effect, OpKind, OpResult};
use crate::run::model::{OpId, task_branch};

const A: RaceLane = RaceLane::A;
const B: RaceLane = RaceLane::B;

/// Decision 21 (task M9.5.15's review): `CrownRacer` is sent only once the lane is
/// `Won` (the restart pin and reconcile's crown row read the lane's state).
#[test]
fn the_crown_is_sent_only_after_the_lane_is_won() {
    let (mut fx, _, _) = racing();
    let effects = passes(&mut fx, B, HEAD_B);
    let (op, _) = only_op(&effects, "CrownRacer");
    let race = fx.task("t1").race.clone().expect("a race");
    assert_eq!((race.winner, race.crowned), (Some(B), false));
    assert_eq!(lane(&fx, B).state, LaneState::Won);
    assert!(fx.run().pending_ops.contains_key(&op));
    let line = "race t1: racer b won";
    assert_eq!(fx.run().log.iter().filter(|l| l.text == line).count(), 1);
}

/// The crown result: the task is the lane's (ruling T1-3).
fn crowned(fx: &mut Fixture, op: OpId, head: &str) -> Vec<Effect> {
    fx.done(op, OpResult::Crowned { head: head.into() })
}

#[test]
fn the_crown_moves_the_tasks_worktree_and_branch() {
    let (mut fx, _, b) = racing();
    let effects = passes(&mut fx, B, HEAD_B);
    let (op, _) = only_op(&effects, "CrownRacer");
    crowned(&mut fx, op, HEAD_B);
    let t1 = fx.task("t1");
    assert_eq!(t1.worktree, lane_path(B));
    assert_eq!(t1.branch, task_branch(RUN_ID, "t1"));
    assert_eq!(t1.checkout_name(), "t1.b");
    assert_eq!(
        (t1.state, t1.head.as_deref()),
        (TaskState::MergeQueue, Some(HEAD_B))
    );
    assert_eq!(t1.route.runtime, Runtime::Codex, "the winner's route");
    assert!(t1.race.as_ref().is_some_and(|r| r.crowned));
    // The merge carries the lane's head; a conflict hands back into its checkout.
    let (op, kind) = super::merge::pending_one(&fx, "MergeCandidate", Some("t1"));
    match kind {
        OpKind::MergeCandidate { task_head, .. } => assert_eq!(task_head, HEAD_B),
        other => panic!("{other:?}"),
    }
    let files = vec!["crates/a/x.rs".to_string()];
    let effects = fx.done(op, OpResult::Conflict { files, tree: None });
    let (op, kind) = only_op(&effects, "HandBack");
    match kind {
        OpKind::HandBack {
            worktree,
            task_head,
            ..
        } => {
            assert_eq!(worktree, lane_path(B));
            assert_eq!(task_head.as_deref(), Some(HEAD_B));
        }
        other => panic!("{other:?}"),
    }
    let handed = OpResult::HandedBack {
        files: vec!["crates/a/x.rs".to_string()],
        head: None,
        onto: None,
        merged: Vec::new(),
        merged_total: 0,
    };
    fx.done(op, handed);
    // The winner's racer is the task's worker now (decision 23), and its next claim
    // (minor m3) is checked in lane b's checkout, on the task branch.
    assert_eq!(super::race::window(&fx, B), b);
    let worker = crate::run::engine::ladder::worker_round(fx.task("t1"));
    assert_eq!(worker.map(|r| fx.task("t1").rounds[r].lane), Some(Some(B)));
    let effects = fx.tool_as(AgentRole::Racer, b, "t1", "task_done", super::race::tdd());
    let (op, kind) = only_op(&effects, "VerifyDone");
    match kind {
        OpKind::VerifyDone { worktree, .. } => assert_eq!(worktree, lane_path(B)),
        other => panic!("{other:?}"),
    }
    let mut check = fx.clean_check("t1");
    if let OpResult::DoneChecked {
        head, head_branch, ..
    } = &mut check
    {
        *head = HEAD_B.to_string();
        *head_branch = Some(task_branch(RUN_ID, "t1"));
    }
    let effects = fx.done(op, check);
    assert!(matches!(&replies(&effects)[..], [Ok(_)]), "{effects:#?}");
}

/// The adoption case: lane a `Adopted` (task M9.5.17b's), its crown makes the task
/// lane a's the same way.
#[test]
fn an_adopted_lanes_crown_moves_the_worktree_too() {
    let (mut fx, _, _) = racing();
    let t1 = fx.task_mut("t1");
    let mut race = crate::run::test_support::race_of(t1, [LaneState::Adopted, LaneState::Out]);
    race.crowned = false;
    race.lanes[0].head = Some(HEAD.into());
    t1.race = Some(race);
    let effects = fx.tick();
    let (op, kind) = only_op(&effects, "CrownRacer");
    match kind {
        OpKind::CrownRacer {
            adopt, checkout, ..
        } => {
            assert!(adopt);
            assert_eq!(checkout, lane_path(A));
        }
        other => panic!("{other:?}"),
    }
    crowned(&mut fx, op, HEAD);
    let t1 = fx.task("t1");
    assert_eq!(
        (t1.worktree.clone(), t1.branch.clone()),
        (lane_path(A), task_branch(RUN_ID, "t1"))
    );
    assert_eq!(t1.state, TaskState::MergeQueue);
}

/// Task M9.5.15's review: a crown that finds the task branch moved halts the run, and
/// nothing imports through the won lane meanwhile: its racer is refused, and a resume
/// sends only the crown again (a compare-and-swap).
#[test]
fn a_moved_task_branch_halts_and_nothing_imports_before_the_crown() {
    let (mut fx, _, b) = racing();
    let effects = passes(&mut fx, B, HEAD_B);
    let (op, _) = only_op(&effects, "CrownRacer");
    fx.done(
        op,
        OpResult::RefMoved {
            reason: "anthrex/x/t1 moved".into(),
        },
    );
    assert_eq!(fx.run().state, proto::RunState::Halted);
    assert!(!fx.task("t1").race.as_ref().is_some_and(|r| r.crowned));
    assert_eq!(fx.task("t1").worktree, task_path("t1"));
    let effects = fx.tool_as(AgentRole::Racer, b, "t1", "task_done", super::race::tdd());
    assert!(matches!(&replies(&effects)[..], [Err(_)]));
    assert!(ops_in(&effects, "VerifyDone").is_empty());
    // A new daemon, then the user's resume (minor m2): the crown is sent again, once,
    // and nothing imports through the won lane.
    let mut effects = super::control_restore::restart(&mut fx, Vec::new());
    assert_eq!(fx.run().state, proto::RunState::Halted);
    let reply = fx.reply();
    effects.extend(fx.next(crate::run::engine::EventKind::Resume {
        reply,
        run_id: RUN_ID.into(),
        rebaseline: Some((BASE.to_string(), BASE.to_string()).into()),
    }));
    assert_eq!(fx.run().state, proto::RunState::Running);
    effects.extend(fx.tick());
    assert_eq!(ops_in(&effects, "CrownRacer").len(), 1, "{effects:#?}");
    for name in [
        "VerifyDone",
        "CountCommits",
        "HandBack",
        "DiffSoFar",
        "MergeCandidate",
    ] {
        assert!(ops_in(&effects, name).is_empty(), "no {name}");
    }
    let won = format!("{}\"", lane_path(B).display());
    for (_, kind) in (effects.iter()).filter_map(|e| match e {
        Effect::Op { op, kind, .. } => Some((op, kind)),
        _ => None,
    }) {
        let text = format!("{kind:?}");
        let crown = matches!(kind, OpKind::CrownRacer { .. });
        assert!(crown || !text.contains(&won), "{text}");
    }
}

/// Task M9.5.15's review: a lane's salvage ref is the task's, so the next salvage
/// number moves on.
#[test]
fn a_lanes_salvage_ref_is_the_tasks() {
    let (mut fx, _, _) = racing();
    let reference = format!("refs/anthrex/salvage/{RUN_ID}/t1/1");
    let op = fx.run().next_op;
    let kind = OpKind::RemoveWorktree {
        root: "/tmp/x".into(),
        path: lane_path(A),
        salvage_ref: reference.clone(),
        keep_head: true,
        clear_locks: true,
        keep_path: false,
    };
    let run = fx.run_mut();
    run.next_op += 1;
    let pending = crate::run::model::PendingOp {
        op,
        task_id: Some("t1".into()),
        kind,
        lane: Some(A),
    };
    run.pending_ops.insert(op, pending);
    fx.done(
        op,
        OpResult::Removed {
            salvage_ref: Some(reference.clone()),
            cleared_locks: Vec::new(),
        },
    );
    let t1 = fx.task("t1");
    assert_eq!(t1.salvage_refs, [reference]);
    assert_eq!(crate::run::engine::merge::next_salvage_seq(t1), 2);
}

/// The crowned lane b's conflict, handed back into its checkout: the task `working`.
fn handed_back(fx: &mut Fixture) {
    let (op, _) = super::merge::pending_one(fx, "MergeCandidate", Some("t1"));
    let conflict = OpResult::Conflict {
        files: vec!["crates/a/x.rs".to_string()],
        tree: None,
    };
    let effects = fx.done(op, conflict);
    let (op, _) = only_op(&effects, "HandBack");
    let handed = OpResult::HandedBack {
        files: vec!["crates/a/x.rs".to_string()],
        head: None,
        onto: None,
        merged: Vec::new(),
        merged_total: 0,
    };
    fx.done(op, handed);
    assert_eq!(fx.task("t1").state, TaskState::Working);
}

/// Part 3 after the crown (minor m4), through the reducer: lane b passes and is crowned
/// while lane a's racer still runs (task M9.5.17b stops it). The task's worker checks
/// see lane b's racer alone: `worker_round`, `has_live_worker`, the budget and rung 4's
/// epoch spend; a retry stops lane b's racer only, and its fresh session starts in lane
/// b's checkout once that racer exits, whatever lane a's still does.
#[test]
fn worker_checks_see_the_crowned_lane() {
    let (mut fx, a, b) = racing();
    let effects = passes(&mut fx, B, HEAD_B);
    let (op, _) = only_op(&effects, "CrownRacer");
    crowned(&mut fx, op, HEAD_B);
    handed_back(&mut fx);
    let now = fx.now + 300;
    fx.send(now, crate::run::engine::EventKind::Tick);
    let t1 = fx.task("t1");
    let racer = |l: RaceLane| {
        (t1.rounds.iter()).any(|r| r.role == AgentRole::Racer && r.lane == Some(l) && !r.ended)
    };
    assert!(racer(A) && racer(B), "both racers run");
    let worker = crate::run::engine::ladder::worker_round(t1).map(|r| t1.rounds[r].lane);
    assert_eq!(worker, Some(Some(B)));
    assert!(crate::run::edits_state::has_live_worker(t1));
    let b_secs: u64 = (t1.rounds.iter())
        .filter(|r| r.role == AgentRole::Racer && r.lane == Some(B))
        .map(|r| crate::run::engine::ladder::round_spend(r, t1.clock.stopped, now).secs)
        .sum();
    assert!(b_secs > 0);
    let spent = crate::run::engine::ladder::total_spend(t1, now);
    assert_eq!(spent.secs, b_secs, "lane a's racer counts for nothing");
    let epoch = crate::run::engine::clock::epoch_spend(t1, now);
    assert_eq!(epoch.secs, b_secs);
    // Blocked, then retried: lane b's racer is the one stopped, and the fresh session
    // waits for it alone.
    let args = serde_json::json!({"kind": "question", "reason": "which API?"});
    fx.tool_as(AgentRole::Racer, b, "t1", "task_blocked", args);
    assert_eq!(fx.task("t1").state, TaskState::Blocked);
    let effects = super::control::retry(&mut fx, "t1");
    let kills: Vec<u32> = (effects.iter())
        .filter_map(|e| match e {
            Effect::KillWindow { window_id } => Some(*window_id),
            _ => None,
        })
        .collect();
    assert_eq!(kills, [b], "{effects:#?}");
    let effects = super::turns::killed_exit(&mut fx, b);
    let (op, kind) = only_op(&effects, "DiffSoFar");
    match kind {
        OpKind::DiffSoFar { worktree, .. } => assert_eq!(worktree, lane_path(B)),
        other => panic!("{other:?}"),
    }
    let diff = OpResult::Diff {
        stat: "1 file".into(),
        patch: String::new(),
    };
    let effects = fx.done(op, diff);
    let (_, kind) = only_op(&effects, "CreateWindow");
    match kind {
        OpKind::CreateWindow { worktree, spec, .. } => {
            assert_eq!((worktree, spec.cwd), (lane_path(B), lane_path(B)));
            assert_eq!(spec.mcp.map(|m| m.role), Some(AgentRole::Worker));
        }
        other => panic!("{other:?}"),
    }
    assert!(
        !(fx.log.iter()).any(|e| matches!(e, Effect::KillWindow { window_id } if *window_id == a))
    );
}
