//! M8a.14: the merge queue and M8a.6 ruling N5 — a task blocked by its second conflict
//! that gains a dependency (carry T11-RR), the mail a merged task leaves behind, and
//! fix round 1's guards on the `resolving` flag (ruling T14-I3) and on a fresh session
//! while halted (T14-I2). A told conflict that is then held is `merge_fixes.rs`'s. Every sequence ends with the liveness check.

use proto::{AgentRole, BlockReason, PlanEdit, TaskState};
use serde_json::json;

use super::dispatch::{edit, replies};
use super::fixture::*;
use super::holds::delivers;
use super::merge::{
    candidate, claim_as, commit, doc_task, head_of, merge, pending_one, start, to_queue, window_of,
};
use super::turns_fixes::assert_alive;
use crate::run::engine::{AgentSignal, Effect, EventKind, OpResult};

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
            onto: Some(head_of("t1")),
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

/// Delivers nothing yet: the worker's own turn is open, so a queued message waits.
fn busy(fx: &mut Fixture, window: u32) {
    fx.signal(window, AgentSignal::TurnStarted);
}

/// t1's queue candidate conflicts; the hand-back comes back with conflicts onto the
/// claimed commit. Returns the effects of that step.
fn handed_back_with_conflicts(fx: &mut Fixture) -> Vec<Effect> {
    let (op, _) = candidate(fx, "t1");
    let files = vec!["docs/t1/a.md".to_string()];
    fx.done(
        op,
        OpResult::Conflict {
            files: files.clone(),
        },
    );
    let (hand_back, _) = pending_one(fx, "HandBack", Some("t1"));
    fx.done(
        hand_back,
        OpResult::HandedBack {
            files,
            head: Some(head_of("t1")),
            onto: Some(head_of("t1")),
        },
    )
}

/// t1 blocks on a question, gains t2 as a dependency and is answered; t2 merges.
/// Returns the effects of t2's merge.
fn held_on_t2_then_released(fx: &mut Fixture, windows: &[(String, u32)]) -> Vec<Effect> {
    let window = window_of(windows, "t1");
    let args = json!({"kind": "question", "reason": "which table?"});
    fx.tool_as(AgentRole::Worker, window, "t1", "task_blocked", args);
    let aborts = edit(
        fx,
        vec![PlanEdit::AddDep {
            task_id: "t1".into(),
            dep: "t2".into(),
        }],
    );
    for (op, _) in ops_in(&aborts, "AbortMerge") {
        fx.done(op, OpResult::MergeAborted);
    }
    edit(
        fx,
        vec![PlanEdit::Answer {
            task_id: "t1".into(),
            text: "users".into(),
        }],
    );
    to_queue(fx, "t2", window_of(windows, "t2"));
    merge(fx, "t2", &commit(2))
}

/// Ruling T14-I3's flag ends with the conflict it marks: an accepted claim resolved it,
/// so a later hold (after a red candidate) is handed back as M8a.6 ruling N5 says.
#[test]
fn a_resolved_conflict_does_not_skip_a_later_hand_back() {
    let (mut fx, windows) = start(&[doc_task("t1", ""), doc_task("t2", "")]);
    let window = window_of(&windows, "t1");
    to_queue(&mut fx, "t1", window);
    handed_back_with_conflicts(&mut fx);
    claim_as(&mut fx, "t1", window, &head_of("t1r"), Some(true));
    let (op, _) = candidate(&fx, "t1");
    fx.done(
        op,
        OpResult::CandidateRed {
            code: Some(1),
            timed_out: false,
            tail: "red".into(),
            secs: 1,
        },
    );
    assert_eq!(fx.task("t1").state, TaskState::Working);
    let effects = held_on_t2_then_released(&mut fx, &windows);
    assert_eq!(ops_in(&effects, "HandBack").len(), 1, "{effects:#?}");
    assert!(!fx.task("t1").handback_due);
    assert_alive(&fx);
}

/// A conflict the worker was never told of is undone when the task is held again
/// (re-review 2 M2); nothing is left to resolve, so the N5 hand-back follows the
/// dependency as usual.
#[test]
fn an_untold_conflict_undone_on_hold_is_handed_back_as_usual() {
    let (mut fx, windows) = start(&[doc_task("t1", ""), doc_task("t2", "")]);
    let window = window_of(&windows, "t1");
    to_queue(&mut fx, "t1", window);
    busy(&mut fx, window);
    let effects = handed_back_with_conflicts(&mut fx);
    assert!(
        delivers(&effects).is_empty(),
        "the turn is open: {effects:#?}"
    );
    let effects = held_on_t2_then_released(&mut fx, &windows);
    assert_eq!(ops_in(&effects, "HandBack").len(), 1, "{effects:#?}");
    assert!(!fx.task("t1").handback_due);
    assert_alive(&fx);
}

/// Ruling T14-I2 for a fresh session: a diff that comes back while the run is halted
/// starts no session; the running pass asks for it again after the resume.
#[test]
fn a_fresh_session_waits_for_the_run_to_run() {
    let (mut fx, windows) = start(&[doc_task("t1", ""), doc_task("t2", "")]);
    to_queue(&mut fx, "t2", window_of(&windows, "t2"));
    let t1 = window_of(&windows, "t1");
    fx.task_mut("t1").fresh_session = Some(crate::run::model::FreshSession {
        reason: "stalled".into(),
        append: None,
    });
    fx.task_mut("t1").rounds[0].retiring = true;
    fx.signal(
        t1,
        AgentSignal::ProcessExited {
            code: None,
            killed_by_engine: true,
            pid: 0,
        },
    );
    let (diff, _) = pending_one(&fx, "DiffSoFar", Some("t1"));
    let (op, _) = candidate(&fx, "t2");
    fx.done(
        op,
        OpResult::RefMoved {
            reason: "moved".into(),
        },
    );
    let stat = || OpResult::Diff {
        stat: "1 file".into(),
        patch: "diff".into(),
    };
    let effects = fx.done(diff, stat());
    assert!(ops_in(&effects, "CreateWindow").is_empty(), "{effects:#?}");
    let reply = fx.reply();
    let effects = fx.next(EventKind::Resume {
        reply,
        run_id: RUN_ID.into(),
        rebaseline: Some((BASE.into(), BASE.into())),
    });
    assert_eq!(ops_in(&effects, "DiffSoFar").len(), 1, "{effects:#?}");
    let (diff, _) = pending_one(&fx, "DiffSoFar", Some("t1"));
    let effects = fx.done(diff, stat());
    assert_eq!(ops_in(&effects, "CreateWindow").len(), 1, "{effects:#?}");
    assert_alive(&fx);
}
