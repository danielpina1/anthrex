//! M8a.14: the merge queue and M8a.6 ruling N5 — a task blocked by its second conflict
//! that gains a dependency (carry T11-RR), a handed-back task that is then held, and
//! the mail a merged task leaves behind. Every sequence ends with the liveness check.

use proto::{AgentRole, BlockReason, PlanEdit, TaskState};
use serde_json::json;

use super::dispatch::{edit, replies, task_path};
use super::fixture::*;
use super::merge::{
    candidate, claim, commit, doc_task, head_of, merge, pending, pending_one, start, to_queue,
    window_of,
};
use super::turns_fixes::assert_alive;
use crate::run::engine::{OpKind, OpResult};

/// Carry T11-RR, the part M8a.14 reaches: a started task blocked for a reason other
/// than a question that gains a dependency keeps its own block when the dependency
/// merges; it is not handed back and not turned into `blocked(question)`.
#[test]
fn a_conflict_blocked_task_that_gains_a_dependency_keeps_its_block() {
    let (mut fx, windows) = start(&[doc_task("t1", ""), doc_task("t2", "")]);
    to_queue(&mut fx, "t1", window_of(&windows, "t1"));
    let (op, _) = candidate(&fx, "t1");
    fx.done(
        op,
        OpResult::Conflict {
            files: vec!["docs/t1/a.md".into()],
        },
    );
    let (hand_back, _) = pending_one(&fx, "HandBack", Some("t1"));
    fx.done(
        hand_back,
        OpResult::HandedBack {
            files: vec![],
            head: Some(head_of("t1m")),
        },
    );
    let (op, _) = candidate(&fx, "t1");
    fx.done(
        op,
        OpResult::Conflict {
            files: vec!["docs/t1/a.md".into()],
        },
    );
    assert_eq!(fx.task("t1").state, TaskState::Blocked);
    let effects = edit(
        &mut fx,
        vec![PlanEdit::AddDep {
            task_id: "t1".into(),
            dep: "t2".into(),
        }],
    );
    assert!(replies(&effects)[0].is_ok(), "{effects:#?}");
    assert!(fx.task("t1").awaiting_deps);
    to_queue(&mut fx, "t2", window_of(&windows, "t2"));
    let effects = merge(&mut fx, "t2", &commit(2));
    assert!(ops_in(&effects, "HandBack").is_empty(), "{effects:#?}");
    let block = fx.task("t1").block.clone().unwrap();
    assert_eq!(block.reason, BlockReason::Conflict);
    assert_eq!(fx.task("t1").state, TaskState::Blocked);
    assert_alive(&fx);
}

/// Ruling T6-N5 on the hand-back path: a task handed back with conflicts that then
/// blocks on a question and gains a dependency is held; the dependency's merge brings
/// the N5 hand-back (not the merge queue's), and its next `task_done` still goes
/// straight to the merge queue.
#[test]
fn a_handed_back_task_held_by_a_new_dependency_is_handed_back_again_first() {
    let (mut fx, windows) = start(&[doc_task("t1", ""), doc_task("t2", "")]);
    let window = window_of(&windows, "t1");
    to_queue(&mut fx, "t1", window);
    let (op, _) = candidate(&fx, "t1");
    let files = vec!["docs/t1/a.md".to_string()];
    fx.done(
        op,
        OpResult::Conflict {
            files: files.clone(),
        },
    );
    let (hand_back, _) = pending_one(&fx, "HandBack", Some("t1"));
    fx.done(hand_back, OpResult::HandedBack { files, head: None });
    assert_eq!(fx.task("t1").state, TaskState::Working);
    let args = json!({"kind": "question", "reason": "which table?"});
    fx.tool_as(AgentRole::Worker, window, "t1", "task_blocked", args);
    edit(
        &mut fx,
        vec![PlanEdit::AddDep {
            task_id: "t1".into(),
            dep: "t2".into(),
        }],
    );
    edit(
        &mut fx,
        vec![PlanEdit::Answer {
            task_id: "t1".into(),
            text: "the users table".into(),
        }],
    );
    assert!(fx.task("t1").awaiting_deps);
    assert_eq!(fx.task("t1").state, TaskState::Blocked);
    assert!(pending(&fx, "HandBack", Some("t1")).is_empty());
    to_queue(&mut fx, "t2", window_of(&windows, "t2"));
    merge(&mut fx, "t2", &commit(2));
    let (hand_back, kind) = pending_one(&fx, "HandBack", Some("t1"));
    assert_eq!(
        kind,
        OpKind::HandBack {
            worktree: task_path("t1"),
            run_head: commit(2),
        }
    );
    assert_eq!(
        fx.task("t1").merge_op,
        None,
        "the N5 hand-back, not the queue's"
    );
    fx.done(
        hand_back,
        OpResult::HandedBack {
            files: vec![],
            head: None,
        },
    );
    assert_eq!(fx.task("t1").state, TaskState::Working);
    assert!(!fx.task("t1").awaiting_deps);
    let effects = claim(&mut fx, "t1", window, &head_of("t1m"));
    assert!(ops_in(&effects, "Check").is_empty(), "{effects:#?}");
    assert_eq!(fx.task("t1").state, TaskState::MergeQueue);
    assert_eq!(candidate(&fx, "t1").1, head_of("t1m"));
    assert_alive(&fx);
}

/// A merged task's worker gets no more mail: whatever was held for it while it was in
/// the merge queue (an amendment, say) is dropped with the merge.
#[test]
fn a_merged_task_leaves_no_mail_behind() {
    let (mut fx, windows) = start(&[doc_task("t1", "")]);
    to_queue(&mut fx, "t1", window_of(&windows, "t1"));
    let effects = edit(
        &mut fx,
        vec![PlanEdit::AmendTask {
            task_id: "t1".into(),
            brief: Some("a new brief".into()),
            acceptance: None,
            route: None,
            test_mode: None,
            test_mode_reason: None,
            priority: None,
            size: None,
        }],
    );
    assert!(replies(&effects)[0].is_ok(), "{effects:#?}");
    assert!(
        fx.run().outbox.iter().any(|m| m.task_id == "t1"),
        "held in the queue"
    );
    merge(&mut fx, "t1", &commit(1));
    assert!(fx.run().outbox.is_empty(), "{:#?}", fx.run().outbox);
    assert_alive(&fx);
}
