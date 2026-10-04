//! Task M9.5.15, part 1 (ruling RR-1): once a lane of a task's race is crowned, every
//! engine site that names one of the task's checkouts names the lane's: the proof and
//! tier-1 scratch checkout `<task>.<lane>.proof`, the review checkout
//! `<task>.<lane>.review`, the merge's clean-up and the run's accept or discard list,
//! which also carries every lane checkout of a race. The launch-side sites are
//! `run/checkout_name_tests.rs`'s. An unraced task is pinned by the existing path tests.

use std::path::PathBuf;

use proto::{FinishAction, LaneState, TaskState};

use super::dispatch::task_path;
use super::fixture::*;
use super::gates::{accepted, check_result, only_op, tdd_args, working_on};
use super::gates_review::reviewer;
use super::merge::{candidate, commit};
use crate::run::engine::{EventKind, OpKind, OpResult};
use crate::run::test_support::race_of;

/// `t1`'s lane b won and the reducer moved the task into its checkout (decision 21;
/// ruling T1-3).
fn crown_b(fx: &mut Fixture) {
    let task = fx.task_mut("t1");
    task.race = Some(race_of(task, [LaneState::Lost, LaneState::Won]));
    task.worktree = task_path("t1.b");
}

fn proof_passed() -> OpResult {
    OpResult::Proof {
        red_failed: true,
        head_passed: true,
        matched: true,
        red_tail: String::new(),
        head_tail: String::new(),
    }
}

#[test]
fn every_gate_checkout_follows_checkout_name() {
    let (mut fx, window) = working_on(PROFILE, "");
    crown_b(&mut fx);
    let effects = accepted(&mut fx, window, tdd_args());
    let (op, kind) = only_op(&effects, "Proof");
    let OpKind::Proof { path, env, .. } = kind else {
        unreachable!()
    };
    assert_eq!(path, task_path("t1.b.proof"));
    let target = format!("{}/target", task_path("t1.b.proof").display());
    assert_eq!(env, vec![("TARGET".to_string(), target)]);

    let effects = fx.done(op, proof_passed());
    let (op, kind) = only_op(&effects, "Check");
    let OpKind::Check { dir, .. } = kind else {
        unreachable!()
    };
    assert_eq!(dir, task_path("t1.b.proof"));

    let effects = fx.done(op, check_result(true));
    assert_eq!(fx.task("t1").state, TaskState::Review);
    let (op, kind) = only_op(&effects, "PrepareReview");
    let OpKind::PrepareReview { path, .. } = kind else {
        unreachable!()
    };
    assert_eq!(path, task_path("t1.b.review"));
    let (_, kind) = reviewer(&mut fx, op, "diff --git a/x b/x");
    let OpKind::CreateWindow { spec, .. } = kind else {
        unreachable!()
    };
    assert_eq!(spec.cwd, task_path("t1.b.review"));
}

#[test]
fn tier_1_runs_in_the_lanes_proof_checkout() {
    let (mut fx, window) = super::tiers::working();
    crown_b(&mut fx);
    let effects = super::tiers::proved(&mut fx, window);
    let (_, kind) = only_op(&effects, "Tier");
    let OpKind::Tier(spec) = kind else {
        unreachable!()
    };
    assert_eq!(spec.dir, task_path("t1.b.proof"));
}

#[test]
fn the_merge_removes_the_lanes_checkouts() {
    let plan = plan_with(PROFILE, &[super::merge::doc_task("t1", "")]);
    let mut fx = Fixture::with_config(&plan, super::merge::config());
    fx.ready(true);
    let window = fx.launch_all()[0].1;
    crown_b(&mut fx);
    super::merge::to_queue(&mut fx, "t1", window);
    let (op, _) = candidate(&fx, "t1");
    let merged = OpResult::Merged {
        commit: commit(1),
        tier: None,
    };
    let effects = fx.done(op, merged);
    let salvage = |n: u32| format!("refs/anthrex/salvage/{RUN_ID}/t1/{n}");
    let removals: Vec<_> = ops_in(&effects, "RemoveWorktree")
        .into_iter()
        .map(|(_, kind)| match kind {
            OpKind::RemoveWorktree {
                path, salvage_ref, ..
            } => (path, salvage_ref),
            _ => unreachable!(),
        })
        .collect();
    assert_eq!(
        removals,
        vec![
            (task_path("t1.b"), salvage(1)),
            (task_path("t1.b.review"), salvage(2)),
            (task_path("t1.b.proof"), salvage(3)),
        ]
    );
}

/// The run's accept or discard list (decision 22: a kept loser's checkout goes with the
/// run's other checkouts) for a complete run whose `t1` raced in `states`: the task's
/// own checkouts by `checkout_name`, then every lane checkout not already listed.
fn discarded_worktrees(states: Option<[LaneState; 2]>, crowned: bool) -> Vec<PathBuf> {
    finished_worktrees(FinishAction::Discard, states, crowned, |_| {})
}

/// [`discarded_worktrees`] for `action`, with `lanes` applied to the race's lanes.
fn finished_worktrees(
    action: FinishAction,
    states: Option<[LaneState; 2]>,
    crowned: bool,
    lanes: impl Fn(&mut [crate::run::model::Lane]),
) -> Vec<PathBuf> {
    let mut fx = super::actions_fixtures::complete();
    let task = fx.task_mut("t1");
    task.race = states.map(|states| race_of(task, states));
    if let Some(race) = task.race.as_mut() {
        lanes(&mut race.lanes);
    }
    if crowned {
        task.worktree = task_path("t1.b");
    }
    let reply = fx.reply();
    fx.next(EventKind::Finish {
        reply,
        run_id: RUN_ID.into(),
        action,
    });
    let worktrees = match action {
        FinishAction::Discard => match fx.op("Discard").1 {
            OpKind::Discard { worktrees, .. } => worktrees,
            other => panic!("{other:?}"),
        },
        _ => match fx.op("Accept").1 {
            OpKind::Accept { worktrees, .. } => worktrees,
            other => panic!("{other:?}"),
        },
    };
    worktrees.into_iter().map(|(path, _)| path).collect()
}

/// Task 17b's fix round 1: accept and discard alike remove a kept loser's checkout and
/// every lane's review and proof checkouts (decision 22).
#[test]
fn accept_and_discard_remove_a_kept_lanes_checkouts() {
    let keep_a = |lanes: &mut [crate::run::model::Lane]| {
        lanes[0].kept = true;
        lanes[0].salvage_ref = Some(format!("refs/anthrex/salvage/{RUN_ID}/t1/1"));
    };
    for action in [FinishAction::Accept, FinishAction::Discard] {
        let states = Some([LaneState::Lost, LaneState::Won]);
        let paths = finished_worktrees(action, states, true, keep_a);
        let mut names = vec!["t1.b", "t1.b.review", "t1.b.proof"];
        names.extend(["t1.a", "t1.a.review", "t1.a.proof"]);
        assert_eq!(paths, listed(&names), "{action:?}");
    }
}

/// `names`' checkouts, then `t2`'s, the integration worktree and the tier-3 checkout.
fn listed(names: &[&str]) -> Vec<PathBuf> {
    let rest = ["t2", "t2.review", "t2.proof", "integration", ".full"];
    names
        .iter()
        .chain(&rest)
        .map(|name| task_path(name))
        .collect()
}

#[test]
fn the_runs_clean_up_lists_every_lane_checkout() {
    let racing = discarded_worktrees(Some([LaneState::Working, LaneState::Review]), false);
    let mut names = vec!["t1", "t1.review", "t1.proof"];
    names.extend(["t1.a", "t1.a.review", "t1.a.proof"]);
    names.extend(["t1.b", "t1.b.review", "t1.b.proof"]);
    assert_eq!(racing, listed(&names));

    let crowned = discarded_worktrees(Some([LaneState::Lost, LaneState::Won]), true);
    let mut names = vec!["t1.b", "t1.b.review", "t1.b.proof"];
    names.extend(["t1.a", "t1.a.review", "t1.a.proof"]);
    assert_eq!(crowned, listed(&names));
}

/// Pinning: an unraced task's list is as before.
#[test]
fn an_unraced_tasks_clean_up_is_unchanged() {
    let paths = discarded_worktrees(None, false);
    assert_eq!(paths, listed(&["t1", "t1.review", "t1.proof"]));
}

/// Decision 27: a crown that reconcile found not started (the task branch absent) is
/// sent again under a new id, as every other idempotent git op is: it is a
/// compare-and-swap.
#[test]
fn a_lost_crown_is_sent_again() {
    let (mut fx, _) = working_on(PROFILE, "");
    let kind = OpKind::CrownRacer {
        root: "/tmp/x".into(),
        task_branch: format!("anthrex/{RUN_ID}/t1"),
        lane_head: HEAD.into(),
        adopt: false,
        checkout: task_path("t1.b"),
    };
    let op = fx.run().next_op;
    let pending = crate::run::model::PendingOp {
        op,
        task_id: Some("t1".into()),
        kind: kind.clone(),
        lane: Some(proto::RaceLane::B),
    };
    let run = fx.run_mut();
    run.pending_ops.insert(op, pending);
    run.next_op = op + 1;
    let effects = super::control_restore::restart(&mut fx, Vec::new());
    let crowns = ops_in(&effects, "CrownRacer");
    assert_eq!(crowns.len(), 1, "{effects:#?}");
    assert_ne!(crowns[0].0, op, "under a new id");
    assert_eq!(crowns[0].1, kind);
    // Review m1: still the lane's op.
    let again = &fx.run().pending_ops[&crowns[0].0];
    assert_eq!(again.lane, Some(proto::RaceLane::B));
    assert_eq!(again.task_id.as_deref(), Some("t1"));
}
