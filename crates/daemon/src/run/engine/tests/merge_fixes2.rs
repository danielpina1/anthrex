//! M8a.14 fix round 2: re-review 1's probes as regression tests (ruling T14-R2). Every
//! sequence ends with the liveness check.

use proto::{AgentRole, PlanEdit, RunState, TaskState};
use serde_json::json;

use super::dispatch::{edit, replies};
use super::fixture::*;
use super::holds::delivers;
use super::merge::{
    candidate, claim_as, commit, doc_task, head_of, merge, pending, pending_one, start, to_queue,
    window_of,
};
use super::turns_fixes::assert_alive;
use crate::run::engine::{AgentSignal, Effect, EventKind, OpKind, OpResult, ResolutionAt};

fn files() -> Vec<String> {
    vec!["docs/t1/a.md".to_string()]
}

fn acknowledge(fx: &mut Fixture) {
    let ids: Vec<u64> = fx
        .run()
        .outbox
        .iter()
        .filter(|m| m.delivered_at.is_some())
        .map(|m| m.id)
        .collect();
    fx.next(EventKind::Delivered {
        run_id: RUN_ID.into(),
        message_ids: ids,
        ok: true,
        error: None,
    });
}

/// t1's worker asks a question in a turn of its own.
fn ask(fx: &mut Fixture, window: u32) {
    fx.signal(window, AgentSignal::TurnStarted);
    let args = json!({"kind": "question", "reason": "which side wins?"});
    fx.tool_as(AgentRole::Worker, window, "t1", "task_blocked", args);
    fx.turn_completed(window);
}

fn add_dep(fx: &mut Fixture, dep: &str) -> Vec<Effect> {
    edit(
        fx,
        vec![PlanEdit::AddDep {
            task_id: "t1".into(),
            dep: dep.into(),
        }],
    )
}

fn answer(fx: &mut Fixture) -> Vec<Effect> {
    edit(
        fx,
        vec![PlanEdit::Answer {
            task_id: "t1".into(),
            text: "theirs".into(),
        }],
    )
}

fn conflicted_onto_the_claim() -> OpResult {
    OpResult::HandedBack {
        files: files(),
        head: Some(head_of("t1")),
        onto: Some(head_of("t1")),
    }
}

/// t1 in the queue, its candidate conflicted, the hand-back conflicted onto the claim
/// and told to the worker.
fn told_queue_conflict(fx: &mut Fixture, window: u32) {
    to_queue(fx, "t1", window);
    let (op, _) = candidate(fx, "t1");
    fx.done(op, OpResult::Conflict { files: files() });
    let (hand_back, _) = pending_one(fx, "HandBack", Some("t1"));
    fx.done(hand_back, conflicted_onto_the_claim());
    acknowledge(fx);
}

/// Ruling T14-R2 (#3): a claim after the queue's conflicted hand-back carries the
/// hand-back to `VerifyDone`, and goes straight back to the queue only when the git
/// layer finds it is the resolution and nothing more; otherwise every gate runs.
#[test]
fn a_resolution_skips_the_gates_only_when_it_is_only_the_resolution() {
    for only in [Some(true), Some(false), None] {
        let (mut fx, windows) = start(&[doc_task("t1", "")]);
        let window = window_of(&windows, "t1");
        told_queue_conflict(&mut fx, window);
        let effects = fx.tool_as(
            AgentRole::Worker,
            window,
            "t1",
            "task_done",
            json!({"summary": "resolved"}),
        );
        let (_, kind) = ops_in(&effects, "VerifyDone")[0].clone();
        let OpKind::VerifyDone { resolution, .. } = kind else {
            unreachable!()
        };
        assert_eq!(
            resolution,
            Some(ResolutionAt {
                onto: head_of("t1"),
                run_head: BASE.into(),
                files: files(),
            })
        );
        let (op, _) = pending_one(&fx, "VerifyDone", Some("t1"));
        let mut result = fx.clean_check("t1");
        if let OpResult::DoneChecked {
            head,
            resolution_only,
            ..
        } = &mut result
        {
            *head = head_of("t1r");
            *resolution_only = only;
        }
        let effects = fx.done(op, result);
        if only == Some(true) {
            assert_eq!(fx.task("t1").state, TaskState::MergeQueue);
            assert_eq!(candidate(&fx, "t1").1, head_of("t1r"));
        } else {
            assert_eq!(fx.task("t1").state, TaskState::Check, "{only:?}");
            assert_eq!(ops_in(&effects, "Check").len(), 1, "{effects:#?}");
        }
        assert!(!fx.task("t1").handed_back);
        // A later claim carries no resolution.
        assert_alive(&fx);
    }
    // An ordinary claim asks nothing of the kind.
    let (mut fx, windows) = start(&[doc_task("t1", "")]);
    let effects = fx.tool_as(
        AgentRole::Worker,
        window_of(&windows, "t1"),
        "t1",
        "task_done",
        json!({"summary": "done"}),
    );
    let (_, kind) = ops_in(&effects, "VerifyDone")[0].clone();
    assert!(matches!(
        kind,
        OpKind::VerifyDone {
            resolution: None,
            ..
        }
    ));
}

/// Probe R1 (ruling T14-R2, N1): a conflict told by an N5 hand-back is being resolved
/// too; a second hold does not hand back into it.
#[test]
fn an_n5_told_conflict_is_not_handed_back_into() {
    let tasks = [doc_task("t1", ""), doc_task("t2", ""), doc_task("t3", "")];
    let (mut fx, windows) = start(&tasks);
    let window = window_of(&windows, "t1");
    ask(&mut fx, window);
    add_dep(&mut fx, "t2");
    answer(&mut fx);
    to_queue(&mut fx, "t2", window_of(&windows, "t2"));
    merge(&mut fx, "t2", &commit(2));
    let (hand_back, _) = pending_one(&fx, "HandBack", Some("t1"));
    fx.done(hand_back, conflicted_onto_the_claim());
    acknowledge(&mut fx);
    assert!(fx.task("t1").resolving);
    ask(&mut fx, window);
    let effects = add_dep(&mut fx, "t3");
    assert!(
        ops_in(&effects, "AbortMerge").is_empty(),
        "told: {effects:#?}"
    );
    answer(&mut fx);
    to_queue(&mut fx, "t3", window_of(&windows, "t3"));
    let effects = merge(&mut fx, "t3", &commit(3));
    assert!(ops_in(&effects, "HandBack").is_empty(), "{effects:#?}");
    assert_eq!(fx.task("t1").state, TaskState::Working);
    assert!(fx.task("t1").handback_due);
    assert_alive(&fx);
}

/// Probe R2 (ruling T14-R2, N2): a queue conflict undone before the worker was told
/// ends the hand-back; the answer turn's claim passes the gates.
#[test]
fn an_undone_conflict_ends_the_straight_to_queue_pass() {
    let (mut fx, windows) = start(&[doc_task("t1", ""), doc_task("t2", "")]);
    let window = window_of(&windows, "t1");
    to_queue(&mut fx, "t1", window);
    let (op, _) = candidate(&fx, "t1");
    fx.done(op, OpResult::Conflict { files: files() });
    let (hand_back, _) = pending_one(&fx, "HandBack", Some("t1"));
    fx.signal(window, AgentSignal::TurnStarted);
    let effects = fx.done(hand_back, conflicted_onto_the_claim());
    assert!(delivers(&effects).is_empty(), "untold");
    let args = json!({"kind": "question", "reason": "unrelated"});
    fx.tool_as(AgentRole::Worker, window, "t1", "task_blocked", args);
    let effects = add_dep(&mut fx, "t2");
    let (abort, _) = ops_in(&effects, "AbortMerge")[0].clone();
    fx.done(abort, OpResult::MergeAborted);
    assert!(!fx.task("t1").handed_back);
    fx.turn_completed(window);
    answer(&mut fx);
    to_queue(&mut fx, "t2", window_of(&windows, "t2"));
    merge(&mut fx, "t2", &commit(2));
    let (hand_back, _) = pending_one(&fx, "HandBack", Some("t1"));
    fx.done(
        hand_back,
        OpResult::HandedBack {
            files: vec![],
            head: Some(head_of("t1m")),
            onto: Some(head_of("t1")),
        },
    );
    acknowledge(&mut fx);
    let effects = claim_as(&mut fx, "t1", window, &head_of("t1n"), Some(true));
    assert_eq!(fx.task("t1").state, TaskState::Check);
    assert!(
        ops_in(&effects, "MergeCandidate").is_empty(),
        "{effects:#?}"
    );
    assert_alive(&fx);
}

/// Probe R3 (ruling T14-R2, N2): rung 2 during a told resolution ends the pass; the
/// fresh session's claim passes the gates.
#[test]
fn rung_2_during_a_resolution_ends_the_straight_to_queue_pass() {
    let extra = "[task.budget]\ntool_calls = 4\nminutes = 1000";
    let (mut fx, windows) = start(&[doc_task("t1", extra)]);
    let window = window_of(&windows, "t1");
    told_queue_conflict(&mut fx, window);
    assert!(fx.task("t1").handed_back);
    fx.signal(window, AgentSignal::TurnStarted);
    let mut killed = false;
    for _ in 0..12 {
        let effects = fx.signal(
            window,
            AgentSignal::ToolUse {
                name: "Bash".into(),
            },
        );
        if effects.contains(&Effect::KillWindow { window_id: window }) {
            killed = true;
            break;
        }
    }
    assert!(killed, "rung 2");
    assert!(!fx.task("t1").handed_back);
    let effects = fx.signal(
        window,
        AgentSignal::ProcessExited {
            code: None,
            killed_by_engine: true,
            pid: 7,
        },
    );
    let (diff, _) = ops_in(&effects, "DiffSoFar")[0].clone();
    fx.done(
        diff,
        OpResult::Diff {
            stat: String::new(),
            patch: String::new(),
        },
    );
    let fresh = fx.complete_windows();
    let effects = claim_as(&mut fx, "t1", fresh[0].1, &head_of("t1f"), Some(true));
    assert_eq!(fx.task("t1").state, TaskState::Check);
    assert!(
        ops_in(&effects, "MergeCandidate").is_empty(),
        "{effects:#?}"
    );
    assert_alive(&fx);
}

/// Probe R5b (ruling T14-R2, N3): a due hand-back whose claim is accepted while the run
/// is halted waits for the running pass; it merges the rebaselined run head.
#[test]
fn a_due_hand_back_waits_for_the_run_to_run() {
    let tasks = [doc_task("t1", ""), doc_task("t2", ""), doc_task("t3", "")];
    let (mut fx, windows) = start(&tasks);
    let window = window_of(&windows, "t1");
    told_queue_conflict(&mut fx, window);
    ask(&mut fx, window);
    add_dep(&mut fx, "t2");
    answer(&mut fx);
    to_queue(&mut fx, "t2", window_of(&windows, "t2"));
    merge(&mut fx, "t2", &commit(2));
    assert!(fx.task("t1").handback_due);
    acknowledge(&mut fx);
    to_queue(&mut fx, "t3", window_of(&windows, "t3"));
    let effects = fx.tool_as(
        AgentRole::Worker,
        window,
        "t1",
        "task_done",
        json!({"summary": "done"}),
    );
    let (verify, _) = ops_in(&effects, "VerifyDone")[0].clone();
    let (op, _) = candidate(&fx, "t3");
    fx.done(
        op,
        OpResult::RefMoved {
            reason: "run ref moved".into(),
        },
    );
    assert_eq!(fx.run().state, RunState::Halted);
    let mut result = fx.clean_check("t1");
    if let OpResult::DoneChecked { head, .. } = &mut result {
        *head = head_of("t1r");
    }
    let effects = fx.done(verify, result);
    assert!(ops_in(&effects, "HandBack").is_empty(), "{effects:#?}");
    assert_eq!(fx.task("t1").state, TaskState::MergeQueue);
    assert_alive(&fx);
    let reply = fx.reply();
    let effects = fx.next(EventKind::Resume {
        reply,
        run_id: RUN_ID.into(),
        rebaseline: Some((BASE.into(), commit(9))),
    });
    let hand_backs = ops_in(&effects, "HandBack");
    assert_eq!(hand_backs.len(), 1, "{effects:#?}");
    assert!(matches!(
        &hand_backs[0].1,
        OpKind::HandBack { run_head, .. } if *run_head == commit(9)
    ));
    assert!(!fx.task("t1").handback_due);
    assert_alive(&fx);
}

/// Ruling T14-R2 (#4): a `cancel_task` edit's reply names a cancel deferred by a merge
/// in flight.
#[test]
fn a_deferred_cancel_edit_says_so() {
    let (mut fx, windows) = start(&[doc_task("t1", ""), doc_task("t2", "")]);
    to_queue(&mut fx, "t1", window_of(&windows, "t1"));
    candidate(&fx, "t1");
    let effects = edit(
        &mut fx,
        vec![PlanEdit::CancelTask {
            task_id: "t1".into(),
        }],
    );
    assert_eq!(
        replies(&effects),
        vec![Ok(
            "applied 1 edit; t1's merge is in flight: it is cancelled only if that merge does not land"
                .to_string()
        )]
    );
    assert!(pending(&fx, "MergeCandidate", Some("t1")).len() == 1);
    // A later edit does not repeat an earlier deferral.
    let effects = edit(
        &mut fx,
        vec![PlanEdit::CancelTask {
            task_id: "t2".into(),
        }],
    );
    assert_eq!(replies(&effects), vec![Ok("applied 1 edit".to_string())]);
    assert!(fx.task("t1").cancel_deferred);
    assert_alive(&fx);
}

/// Ruling T14-R3 (nit): a `cancel_task` edit clears a due hand-back, as `run cancel`
/// does, so nothing restored later reads it.
#[test]
fn a_cancel_edit_clears_a_due_hand_back() {
    let tasks = [doc_task("t1", ""), doc_task("t2", "")];
    let (mut fx, windows) = start(&tasks);
    let window = window_of(&windows, "t1");
    told_queue_conflict(&mut fx, window);
    ask(&mut fx, window);
    add_dep(&mut fx, "t2");
    answer(&mut fx);
    to_queue(&mut fx, "t2", window_of(&windows, "t2"));
    merge(&mut fx, "t2", &commit(2));
    assert!(fx.task("t1").handback_due);
    edit(
        &mut fx,
        vec![PlanEdit::CancelTask {
            task_id: "t1".into(),
        }],
    );
    assert_eq!(fx.task("t1").state, TaskState::Cancelled);
    assert!(!fx.task("t1").handback_due);
    assert_alive(&fx);
}
