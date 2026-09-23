//! M8a.11 fix rounds 2 and 3 (rulings T11-N1..N3, re-review 2 M1 and M2): the
//! conflicts of a held task's hand-back. A conflict during a hold is undone
//! (`AbortMerge`); one the worker was told of is kept; one it was never told of is
//! undone when the task is held again. Split from `holds.rs` to keep both under
//! AGENTS.md rule 8's ~600 lines.

use proto::{BlockReason, PlanEdit, TaskState};

use super::dispatch::{edit, replies};
use super::fixture::*;
use super::holds::{
    add_dep, answer, assert_held, blocked_t1, delivers, hand_back_in_flight, joined, plan_task,
};
use crate::run::contract::{answer_message, conflict_message};
use crate::run::engine::{OpKind, OpResult};

fn amend_brief(text: &str) -> PlanEdit {
    PlanEdit::AmendTask {
        task_id: "t1".into(),
        brief: Some(text.into()),
        acceptance: None,
        route: None,
        test_mode: None,
        test_mode_reason: None,
        priority: None,
        size: None,
    }
}

fn add_t3_as_dep(fx: &mut Fixture) {
    let effects = edit(
        fx,
        vec![
            PlanEdit::AddTask {
                task: plan_task("t3", "[\"crates/c/**\"]"),
            },
            add_dep("t1", "t3"),
        ],
    );
    assert!(replies(&effects)[0].is_ok(), "{effects:#?}");
}

fn conflict_messages(fx: &Fixture, files: &[String]) -> usize {
    let text = conflict_message(files);
    fx.run().outbox.iter().filter(|m| m.text == text).count()
}

/// Re-review N1(b), probe q1: a hand-back that conflicts while another dependency is
/// still unfinished is aborted, not handed to the worker. No second hand-back runs
/// while the abort does. The hand-back after the last dependency merges produces the
/// one conflict message.
#[test]
fn a_conflict_while_a_dependency_is_unfinished_is_aborted() {
    let mut fx = blocked_t1();
    let first = hand_back_in_flight(&mut fx);
    add_t3_as_dep(&mut fx);
    let files = vec!["crates/a/x.rs".to_string()];
    let effects = fx.done(
        first,
        OpResult::HandedBack {
            files: files.clone(),
        },
    );
    let aborts = ops_in(&effects, "AbortMerge");
    assert_eq!(aborts.len(), 1, "{effects:#?}");
    assert_eq!(
        aborts[0].1,
        OpKind::AbortMerge {
            worktree: fx.task("t1").worktree.clone()
        }
    );
    assert_held(&fx);
    assert!(delivers(&effects).is_empty(), "{effects:#?}");
    assert_eq!(conflict_messages(&fx, &files), 0, "the conflict is dropped");

    fx.launch_all();
    let effects = fx.merge("t3", &"c3".repeat(20));
    assert!(
        ops_in(&effects, "HandBack").is_empty(),
        "not while the abort runs"
    );
    assert_held(&fx);
    let effects = fx.done(aborts[0].0, OpResult::MergeAborted);
    let hand_backs = ops_in(&effects, "HandBack");
    assert_eq!(hand_backs.len(), 1, "{effects:#?}");
    let effects = fx.done(
        hand_backs[0].0,
        OpResult::HandedBack {
            files: files.clone(),
        },
    );
    assert_eq!(fx.task("t1").state, TaskState::Working);
    assert_eq!(
        delivers(&effects),
        vec![joined(&[answer_message("A"), conflict_message(&files)])]
    );
    assert_eq!(conflict_messages(&fx, &files), 1, "one per hand-back");
}

/// Ruling T11-N1(b): an abort that fails leaves the worktree mid-merge; the task is
/// blocked on its environment and no hand-back follows.
#[test]
fn a_failed_abort_blocks_the_task_on_its_environment() {
    let mut fx = blocked_t1();
    let first = hand_back_in_flight(&mut fx);
    add_t3_as_dep(&mut fx);
    let files = vec!["crates/a/x.rs".to_string()];
    let effects = fx.done(first, OpResult::HandedBack { files });
    let abort = ops_in(&effects, "AbortMerge")[0].0;
    fx.done(
        abort,
        OpResult::Failed {
            message: "index.lock exists".into(),
        },
    );
    let t1 = fx.task("t1");
    assert_eq!(t1.state, TaskState::Blocked);
    let block = t1.block.as_ref().unwrap();
    assert_eq!(block.reason, BlockReason::Environment);
    assert!(block.text.contains("index.lock exists"), "{}", block.text);
    fx.launch_all();
    let effects = fx.merge("t3", &"c3".repeat(20));
    assert!(ops_in(&effects, "HandBack").is_empty(), "{effects:#?}");
}

/// Re-review N2, probe q7: an amendment to a held task whose question is unanswered
/// does not resume it. After the hand-back it is back on its question, and the
/// amendment goes out with the eventual answer.
#[test]
fn an_amendment_to_an_unanswered_held_task_waits_for_the_answer() {
    let mut fx = blocked_t1();
    let question = fx.task("t1").block.clone();
    edit(&mut fx, vec![add_dep("t1", "t2")]);
    fx.launch_all();
    fx.merge("t2", &"c2".repeat(20));
    let effects = edit(&mut fx, vec![amend_brief("new brief")]);
    assert!(replies(&effects)[0].is_ok(), "{effects:#?}");
    let amendment: Vec<String> = fx
        .run()
        .outbox
        .iter()
        .filter(|m| m.task_id == "t1" && m.delivered_at.is_none())
        .map(|m| m.text.clone())
        .collect();
    assert_eq!(amendment.len(), 1, "the amendment is queued");
    let hand_backs = ops_in(&effects, "HandBack");
    assert_eq!(hand_backs.len(), 1, "{effects:#?}");

    let effects = fx.done(hand_backs[0].0, OpResult::HandedBack { files: vec![] });
    let t1 = fx.task("t1");
    assert_eq!(t1.state, TaskState::Blocked);
    assert_eq!(t1.block, question, "back on its own question");
    assert!(!t1.awaiting_deps);
    assert!(delivers(&effects).is_empty(), "{effects:#?}");
    assert!(delivers(&fx.tick()).is_empty());

    let effects = edit(&mut fx, vec![answer("users")]);
    assert_eq!(fx.task("t1").state, TaskState::Working);
    assert_eq!(
        delivers(&effects),
        vec![joined(&[amendment[0].clone(), answer_message("users")])]
    );
}

/// Re-review N3, probe q2b: a conflict is processed even when the dependency that held
/// the task was cancelled while the hand-back ran. The worker gets the conflict message
/// once it can take messages again.
#[test]
fn a_conflict_is_kept_after_the_holding_dependency_is_cancelled() {
    let mut fx = blocked_t1();
    let first = hand_back_in_flight(&mut fx);
    add_t3_as_dep(&mut fx);
    edit(
        &mut fx,
        vec![PlanEdit::CancelTask {
            task_id: "t3".into(),
        }],
    );
    assert_eq!(
        fx.task("t1").block.as_ref().unwrap().reason,
        BlockReason::DepCancelled
    );
    let files = vec!["crates/a/x.rs".to_string()];
    let effects = fx.done(
        first,
        OpResult::HandedBack {
            files: files.clone(),
        },
    );
    assert!(ops_in(&effects, "AbortMerge").is_empty(), "{effects:#?}");
    assert_eq!(conflict_messages(&fx, &files), 1);
    let t1 = fx.task("t1");
    assert_eq!(t1.block.as_ref().unwrap().reason, BlockReason::DepCancelled);
}

/// A failed hand-back for a task whose hold was replaced by `dep_cancelled` meanwhile
/// leaves that block in place: the merge never happened.
#[test]
fn a_failed_hand_back_keeps_a_dep_cancelled_block() {
    let mut fx = blocked_t1();
    let first = hand_back_in_flight(&mut fx);
    add_t3_as_dep(&mut fx);
    edit(
        &mut fx,
        vec![PlanEdit::CancelTask {
            task_id: "t3".into(),
        }],
    );
    fx.done(
        first,
        OpResult::Failed {
            message: "untracked files in the way".into(),
        },
    );
    let t1 = fx.task("t1");
    assert_eq!(t1.block.as_ref().unwrap().reason, BlockReason::DepCancelled);
}

/// A task cancelled while its hand-back runs takes no conflict message.
#[test]
fn a_conflict_for_a_cancelled_task_is_dropped() {
    let mut fx = blocked_t1();
    let first = hand_back_in_flight(&mut fx);
    edit(
        &mut fx,
        vec![PlanEdit::CancelTask {
            task_id: "t1".into(),
        }],
    );
    let files = vec!["crates/a/x.rs".to_string()];
    fx.done(
        first,
        OpResult::HandedBack {
            files: files.clone(),
        },
    );
    assert_eq!(fx.task("t1").state, TaskState::Cancelled);
    assert_eq!(conflict_messages(&fx, &files), 0);
}

/// Ruling T11-N1(b): a task cancelled while its abort runs stays cancelled whatever the
/// abort returns.
#[test]
fn an_abort_result_for_a_cancelled_task_is_ignored() {
    let mut fx = blocked_t1();
    let first = hand_back_in_flight(&mut fx);
    add_t3_as_dep(&mut fx);
    let files = vec!["crates/a/x.rs".to_string()];
    let effects = fx.done(first, OpResult::HandedBack { files });
    let abort = ops_in(&effects, "AbortMerge")[0].0;
    edit(
        &mut fx,
        vec![PlanEdit::CancelTask {
            task_id: "t1".into(),
        }],
    );
    assert_eq!(fx.task("t1").state, TaskState::Cancelled);
    fx.done(
        abort,
        OpResult::Failed {
            message: "index.lock exists".into(),
        },
    );
    assert_eq!(fx.task("t1").state, TaskState::Cancelled);
}

/// Re-review 2 M1, probe p4: an abort that fails after the holding dependency was
/// cancelled keeps the `dep_cancelled` block (only its text records the leftover
/// merge), and the task is not held again on the cancelled dependency.
#[test]
fn a_failed_abort_keeps_a_dep_cancelled_block() {
    let mut fx = blocked_t1();
    let first = hand_back_in_flight(&mut fx);
    add_t3_as_dep(&mut fx);
    let files = vec!["crates/a/x.rs".to_string()];
    let effects = fx.done(first, OpResult::HandedBack { files });
    let abort = ops_in(&effects, "AbortMerge")[0].0;
    edit(
        &mut fx,
        vec![PlanEdit::CancelTask {
            task_id: "t3".into(),
        }],
    );
    fx.done(
        abort,
        OpResult::Failed {
            message: "index.lock exists".into(),
        },
    );
    fx.tick();
    let t1 = fx.task("t1");
    let block = t1.block.as_ref().unwrap();
    assert_eq!(block.reason, BlockReason::DepCancelled);
    assert!(block.text.contains("index.lock exists"), "{}", block.text);
    assert!(!t1.awaiting_deps, "not held on a cancelled dependency");
}

/// Re-review 2 M2, probe p6: an unanswered task's conflict the worker was never told of
/// is undone when the task is held again, and its message dropped; the hand-back after
/// the new dependency merges brings the conflict again, once.
#[test]
fn an_untold_conflict_is_aborted_when_the_task_is_held_again() {
    let mut fx = blocked_t1();
    edit(&mut fx, vec![add_dep("t1", "t2")]);
    fx.launch_all();
    fx.merge("t2", &"c2".repeat(20));
    let effects = edit(&mut fx, vec![amend_brief("new brief")]);
    let first = ops_in(&effects, "HandBack")[0].0;
    let files = vec!["crates/a/x.rs".to_string()];
    fx.done(
        first,
        OpResult::HandedBack {
            files: files.clone(),
        },
    );
    assert_eq!(conflict_messages(&fx, &files), 1);
    assert!(!fx.task("t1").awaiting_deps);

    let effects = edit(
        &mut fx,
        vec![
            PlanEdit::AddTask {
                task: plan_task("t3", "[\"crates/c/**\"]"),
            },
            add_dep("t1", "t3"),
        ],
    );
    assert!(replies(&effects)[0].is_ok(), "{effects:#?}");
    let aborts = ops_in(&effects, "AbortMerge");
    assert_eq!(aborts.len(), 1, "{effects:#?}");
    assert_eq!(
        conflict_messages(&fx, &files),
        0,
        "the untold conflict is dropped"
    );
    assert!(fx.task("t1").awaiting_deps);

    fx.done(aborts[0].0, OpResult::MergeAborted);
    fx.launch_all();
    let effects = fx.merge("t3", &"c3".repeat(20));
    let hand_backs = ops_in(&effects, "HandBack");
    assert_eq!(
        hand_backs.len(),
        1,
        "the amendment still waits: {effects:#?}"
    );
    fx.done(
        hand_backs[0].0,
        OpResult::HandedBack {
            files: files.clone(),
        },
    );
    assert_eq!(conflict_messages(&fx, &files), 1);
    let t1 = fx.task("t1");
    assert_eq!(t1.state, TaskState::Blocked);
    assert_eq!(t1.block.as_ref().unwrap().text, "which table?");
}
