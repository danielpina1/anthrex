//! Controller ruling C-21 on task M9.1.17: a run completes delivered only when every
//! merged task is in the highest stage (the branch `run accept` delivers). A sync task
//! is never given up by the `finish` edit and, cancelled, leaves its stage due again;
//! a red propagate holds a finishing run; `run cancel` still ends it. A sync claim
//! must keep its merge, and its reviewer sees only the resolution.

use proto::{AgentRole, PlanEdit, RunState, TaskState};

use super::dispatch::edit;
use super::fixture::*;
use super::full::{attention, later};
use super::kinds_integration::merge_real;
use super::merge::{commit, merge, pending, pending_one, to_queue, window_of};
use super::propagate::{TREE, conflicted, merge_t1, propagate, propagates, stages};
use crate::run::contract::sha7;
use crate::run::engine::stages::set_stage_head;
use crate::run::engine::{EventKind, OpKind, OpResult};
use crate::run::model::SyncCheck;

const UNDELIVERED: &str = "stage 1's merged work is not in stage 2 yet: t1; it cannot be delivered until it is (anthrex run cancel gives up)";

/// `t2` merged into stage 2 at `commit(2)`, then `t1` into stage 1 at `commit(1)`, whose
/// propagate into stage 2 comes back with `result`.
fn t1_propagated(result: OpResult) -> Fixture {
    let (mut fx, windows) = stages(false);
    to_queue(&mut fx, "t2", window_of(&windows, "t2"));
    merge(&mut fx, "t2", &commit(2));
    merge_t1(&mut fx, &windows);
    let (op, spec) = propagate(&fx);
    assert_eq!(spec.expected_to_head, commit(2));
    fx.done(op, result);
    fx
}

fn conflict() -> OpResult {
    OpResult::Conflict {
        files: vec!["docs/shared.md".into()],
        tree: Some(TREE.into()),
    }
}

fn red() -> OpResult {
    OpResult::CandidateRed {
        code: Some(1),
        timed_out: false,
        tail: String::new(),
        secs: 1,
        tier: None,
    }
}

/// The reviewer's scenario: `finish` while the sync task is open. It is not cancelled,
/// no `VerifyRefs` is issued without t1 in stage 2, and the run completes only after the
/// sync task merges.
#[test]
fn finish_with_an_open_sync_task_waits_for_it_to_merge() {
    let mut fx = t1_propagated(conflict());
    edit(&mut fx, vec![PlanEdit::Finish]);
    later(&mut fx, 1_000);
    assert!(
        !fx.task("fix1").state.is_finished(),
        "{:?}",
        fx.task("fix1").state
    );
    assert!(fx.ops("VerifyRefs").is_empty(), "{:#?}", fx.run().log);
    assert!(
        attention(&fx).iter().any(|l| l == UNDELIVERED),
        "{:?}",
        attention(&fx)
    );
    merge_real(&mut fx, "fix1", &commit(4));
    assert!(fx.run().stage(2).unwrap().tasks_in.contains("t1"));
    later(&mut fx, 1);
    let (op, _) = pending_one(&fx, "VerifyRefs", None);
    assert!(!attention(&fx).iter().any(|l| l == UNDELIVERED));
    fx.done(op, OpResult::RefsOk);
    for (op, _) in pending(&fx, "Check", None) {
        fx.done(op, super::gates::check_result(true));
    }
    assert_eq!(fx.run().state, RunState::Complete);
}

/// A sync task cancelled without merging leaves its stage due again.
#[test]
fn a_cancelled_sync_task_makes_its_stage_due_again() {
    let (mut fx, _) = conflicted();
    assert!(propagates(&fx).is_empty());
    edit(
        &mut fx,
        vec![PlanEdit::CancelTask {
            task_id: "fix1".into(),
        }],
    );
    assert_eq!(fx.task("fix1").state, TaskState::Cancelled);
    fx.tick();
    let (_, spec) = propagate(&fx);
    assert_eq!((spec.from, spec.to), (1, 2));
    assert_eq!(spec.from_head, commit(1));
}

/// Item 4: a red propagate after `finish` holds completion with the invariant's line.
#[test]
fn a_red_propagate_after_finish_holds_completion() {
    let mut fx = t1_propagated(red());
    edit(&mut fx, vec![PlanEdit::Finish]);
    later(&mut fx, 1_000);
    assert!(fx.ops("VerifyRefs").is_empty(), "{:#?}", fx.run().log);
    let lines = attention(&fx);
    assert!(lines.iter().any(|l| l == UNDELIVERED), "{lines:?}");
    let red = "propagate of stage 1 into stage 2 is red: the check failed";
    assert!(lines.iter().any(|l| l == red), "{lines:?}");
    assert_ne!(fx.run().state, RunState::Complete);
}

/// Item 1(d): `run cancel` still ends the run, and the report names what the branch
/// lacks.
#[test]
fn run_cancel_ends_an_undelivered_run_and_reports_it() {
    let mut fx = t1_propagated(red());
    let reply = fx.reply();
    fx.next(EventKind::Cancel {
        reply,
        run_id: RUN_ID.into(),
    });
    for _ in 0..4 {
        later(&mut fx, 10);
        for (op, _) in pending(&fx, "VerifyRefs", None) {
            fx.done(op, OpResult::RefsOk);
        }
        for (op, _) in pending(&fx, "Check", None) {
            fx.done(op, super::gates::check_result(true));
        }
    }
    assert_eq!(fx.run().state, RunState::Complete);
    let report = crate::run::report::render(fx.run(), fx.now);
    let line = "Not delivered: stage 1's merged work never reached stage 2: t1";
    assert!(report.contains(line), "{report}");
}

/// The sync task `fix1` of [`conflicted`], its merge handed back and its session
/// started; its window.
fn launched_sync() -> (Fixture, u32) {
    let (mut fx, _) = conflicted();
    let (op, _) = pending_one(&fx, "PrepareWorktree", Some("fix1"));
    fx.done(op, OpResult::Worktree { head: BASE.into() });
    let (op, _) = pending_one(&fx, "HandBack", Some("fix1"));
    fx.done(
        op,
        OpResult::HandedBack {
            files: vec!["docs/shared.md".into()],
            head: Some(BASE.into()),
            onto: Some(BASE.into()),
            merged: Vec::new(),
            merged_total: 0,
        },
    );
    let windows = fx.launch_all();
    let window = window_of(&windows, "fix1");
    (fx, window)
}

/// `fix1` claims done; its `VerifyDone` op and kind.
fn sync_claim(fx: &mut Fixture, window: u32) -> (u64, OpKind) {
    let args = serde_json::json!({"summary": "resolved"});
    let effects = fx.tool_as(AgentRole::Worker, window, "fix1", "task_done", args);
    ops_in(&effects, "VerifyDone")[0].clone()
}

/// Item 5: a sync claim whose head lost the merge (`merge --abort`, then a plain
/// commit) is refused.
#[test]
fn a_sync_claim_that_lost_its_merge_is_refused() {
    let (mut fx, window) = launched_sync();
    let (op, kind) = sync_claim(&mut fx, window);
    let OpKind::VerifyDone { sync, .. } = kind else {
        unreachable!()
    };
    assert_eq!(
        sync,
        Some(Box::new(SyncCheck {
            onto: commit(1),
            to_head: BASE.into(),
            upper: None,
        }))
    );
    let mut result = fx.clean_check("fix1");
    if let OpResult::DoneChecked { sync_kept, .. } = &mut result {
        *sync_kept = Some(false);
    }
    let effects = fx.done(op, result);
    let text = format!(
        "task_done rejected: sync task must keep stage 1's merge: its head does not contain {}",
        sha7(&commit(1))
    );
    assert_eq!(super::dispatch::replies(&effects), vec![Err(text)]);
    assert_eq!(fx.task("fix1").state, TaskState::Working);
}

/// Item 3: a hand-back into the sync task after its first (here the merge queue's) is
/// recorded, and its next claim's check names that head.
#[test]
fn a_later_hand_back_is_carried_to_the_sync_claim() {
    let (mut fx, window) = launched_sync();
    set_stage_head(fx.run_mut(), 2, &commit(9));
    let task = fx.task_mut("fix1");
    task.state = TaskState::MergeQueue;
    task.head = Some(HEAD.into());
    fx.run_mut().merge_queue.push("fix1".into());
    fx.tick();
    let (op, _) = pending_one(&fx, "MergeCandidate", Some("fix1"));
    let files = vec!["c.txt".to_string()];
    fx.done(op, OpResult::Conflict { files, tree: None });
    let (op, kind) = pending_one(&fx, "HandBack", Some("fix1"));
    let OpKind::HandBack { run_head, .. } = kind else {
        unreachable!()
    };
    assert_eq!(run_head, commit(9));
    fx.done(
        op,
        OpResult::HandedBack {
            files: vec!["c.txt".into()],
            head: Some(HEAD.into()),
            onto: Some(HEAD.into()),
            merged: Vec::new(),
            merged_total: 0,
        },
    );
    assert_eq!(
        fx.task("fix1").sync.as_ref().unwrap().handed,
        vec![commit(9)]
    );
    let (_, kind) = sync_claim(&mut fx, window);
    let OpKind::VerifyDone { sync, .. } = kind else {
        unreachable!()
    };
    assert_eq!(sync.unwrap().upper, Some(commit(9)));
}

/// Item 6: the sync task's reviewer reads the diff from the conflicted tree, and the
/// prompt says what to judge.
#[test]
fn the_sync_reviewer_sees_only_the_resolution() {
    let (mut fx, window) = launched_sync();
    let (op, _) = sync_claim(&mut fx, window);
    fx.done(op, fx.clean_check("fix1"));
    fx.turn_completed(window);
    let (op, _) = pending_one(&fx, "Check", Some("fix1"));
    fx.done(op, super::gates::check_result(true));
    let (op, kind) = pending_one(&fx, "PrepareReview", Some("fix1"));
    let OpKind::PrepareReview { base_tree, .. } = kind else {
        unreachable!()
    };
    assert_eq!(base_tree.as_deref(), Some(TREE));
    let effects = fx.done(
        op,
        OpResult::Review {
            base: TREE.into(),
            head: HEAD.into(),
            patch: "diff --git a/docs/shared.md b/docs/shared.md".into(),
        },
    );
    let windows = ops_in(&effects, "CreateWindow");
    let OpKind::CreateWindow { first_turn, .. } = &windows[0].1 else {
        unreachable!()
    };
    let line = "This is a sync task: judge only how the conflicts were resolved; stage 1's own changes were reviewed when they merged.";
    assert!(first_turn.contains(line), "{first_turn}");
    assert!(
        first_turn.contains(&format!("Diff ({}..", sha7(TREE))),
        "{first_turn}"
    );
}
