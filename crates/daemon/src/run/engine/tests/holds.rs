//! M8a.11 fix round 1, ruling T11-I1..I3: the N5 hold is task state. A started task
//! that gains an unfinished dependency is held: it stays blocked, answers queue, and
//! it resumes only after one hand-back with every dependency finished. The three
//! sequences are the review's probes r1–r3; r6 is minor 8.

use proto::{BlockReason, PlanEdit, PlanTask, TaskState};

use super::dispatch::{edit, replies};
use super::fixture::*;
use crate::run::contract::{answer_message, conflict_message};
use crate::run::engine::{AgentSignal, Effect, OpKind, OpResult, TurnOutcome};
use crate::run::messages::join_turn;
use crate::run::model::{OpId, Outgoing};

fn plan_task(id: &str, owns: &str) -> PlanTask {
    let text = plan_with(PROFILE, &[task_toml(id, "S", owns, "")]);
    crate::run::plan::parse_plan(&text).unwrap().tasks.remove(0)
}

fn delivers(effects: &[Effect]) -> Vec<String> {
    effects
        .iter()
        .filter_map(|e| match e {
            Effect::Deliver { text, .. } => Some(text.clone()),
            _ => None,
        })
        .collect()
}

fn joined(texts: &[String]) -> String {
    let messages: Vec<Outgoing> = texts
        .iter()
        .enumerate()
        .map(|(i, t)| Outgoing {
            id: i as u64,
            window_id: 0,
            task_id: "t1".into(),
            text: t.clone(),
            queued_at: 0,
            delivered_at: None,
        })
        .collect();
    join_turn(&messages.iter().collect::<Vec<_>>())
}

fn add_dep(task: &str, dep: &str) -> PlanEdit {
    PlanEdit::AddDep {
        task_id: task.into(),
        dep: dep.into(),
    }
}

fn answer(text: &str) -> PlanEdit {
    PlanEdit::Answer {
        task_id: "t1".into(),
        text: text.into(),
    }
}

/// t1 started and `blocked(question)` with its turn closed; t2 unstarted, overlapping.
fn blocked_t1() -> Fixture {
    let plan = plan_with(
        PROFILE,
        &[
            task_toml("t1", "S", "[\"crates/a/**\"]", ""),
            task_toml("t2", "S", "[\"crates/a/src/**\"]", ""),
        ],
    );
    let mut fx = Fixture::new(&plan);
    fx.ready(true);
    let window = fx.launch_all()[0].1;
    fx.signal(
        window,
        AgentSignal::TurnEnded {
            outcome: TurnOutcome::Completed,
            usage: None,
            denials: 0,
        },
    );
    let task = fx.task_mut("t1");
    task.state = TaskState::Blocked;
    task.block = Some(proto::BlockInfo {
        reason: BlockReason::Question,
        text: "which table?".into(),
    });
    fx.tick();
    fx
}

fn assert_held(fx: &Fixture) {
    let t1 = fx.task("t1");
    assert_eq!(t1.state, TaskState::Blocked);
    assert!(t1.awaiting_deps, "t1 carries the hold");
}

/// `[add_dep t1 t2, answer t1 "A"]`, t2 merged: returns the in-flight hand-back.
fn hand_back_in_flight(fx: &mut Fixture) -> OpId {
    edit(fx, vec![add_dep("t1", "t2"), answer("A")]);
    fx.launch_all();
    let effects = fx.merge("t2", &"c2".repeat(20));
    let hand_backs = ops_in(&effects, "HandBack");
    assert_eq!(hand_backs.len(), 1, "{effects:#?}");
    hand_backs[0].0
}

/// Review I1: the dependency merges before the answer comes; the answer still waits for
/// the hand-back.
#[test]
fn an_answer_after_the_new_dependency_merged_hands_back_first() {
    let mut fx = blocked_t1();
    let effects = edit(&mut fx, vec![add_dep("t1", "t2")]);
    assert!(replies(&effects)[0].is_ok(), "{effects:#?}");
    assert_held(&fx);
    fx.launch_all();
    let effects = fx.merge("t2", &"c2".repeat(20));
    assert!(
        ops_in(&effects, "HandBack").is_empty(),
        "nothing to resume yet"
    );
    assert_held(&fx);
    let effects = edit(&mut fx, vec![answer("users")]);
    let hand_backs = ops_in(&effects, "HandBack");
    assert_eq!(hand_backs.len(), 1, "{effects:#?}");
    assert!(delivers(&effects).is_empty(), "{effects:#?}");
    assert_held(&fx);
    let effects = fx.done(hand_backs[0].0, OpResult::HandedBack { files: vec![] });
    assert_eq!(fx.task("t1").state, TaskState::Working);
    assert!(!fx.task("t1").awaiting_deps);
    assert_eq!(delivers(&effects), vec![answer_message("users")]);
}

/// Review I2: a dependency added while the hand-back runs; the result re-checks.
#[test]
fn a_dependency_added_while_the_hand_back_runs_keeps_the_hold() {
    let mut fx = blocked_t1();
    let first = hand_back_in_flight(&mut fx);
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
    let effects = fx.done(first, OpResult::HandedBack { files: vec![] });
    assert_held(&fx);
    assert!(delivers(&effects).is_empty(), "{effects:#?}");
    assert!(ops_in(&effects, "HandBack").is_empty(), "t3 is unfinished");
    assert!(
        ops_in(&effects, "AbortMerge").is_empty(),
        "a clean merge stays"
    );
    fx.launch_all();
    let effects = fx.merge("t3", &"c3".repeat(20));
    let hand_backs = ops_in(&effects, "HandBack");
    assert_eq!(hand_backs.len(), 1, "a second hand-back once t3 merged");
    let effects = fx.done(hand_backs[0].0, OpResult::HandedBack { files: vec![] });
    assert_eq!(fx.task("t1").state, TaskState::Working);
    assert_eq!(delivers(&effects), vec![answer_message("A")]);
}

/// Review I3: an answer while the hand-back runs is queued; the conflicted result is
/// handled; exactly one hand-back is ever in flight.
#[test]
fn an_answer_while_the_hand_back_runs_waits_for_it() {
    let mut fx = blocked_t1();
    let hand_back = hand_back_in_flight(&mut fx);
    let effects = edit(&mut fx, vec![answer("B")]);
    assert!(replies(&effects)[0].is_ok(), "{effects:#?}");
    assert!(delivers(&effects).is_empty(), "{effects:#?}");
    assert_held(&fx);
    for _ in 0..3 {
        let effects = fx.tick();
        assert!(
            ops_in(&effects, "HandBack").is_empty(),
            "no second hand-back"
        );
    }
    let files = vec!["crates/a/x.rs".to_string()];
    let effects = fx.done(
        hand_back,
        OpResult::HandedBack {
            files: files.clone(),
        },
    );
    let t1 = fx.task("t1");
    assert_eq!(t1.state, TaskState::Working);
    assert!(!t1.awaiting_deps);
    assert_eq!(t1.block, None);
    assert_eq!(
        delivers(&effects),
        vec![joined(&[
            answer_message("A"),
            answer_message("B"),
            conflict_message(&files),
        ])]
    );
}

/// Review minor 8: a cancelled dependency replaces the hold with `dep_cancelled`.
#[test]
fn a_cancelled_dependency_replaces_the_hold() {
    let mut fx = blocked_t1();
    edit(&mut fx, vec![add_dep("t1", "t2"), answer("A")]);
    assert_held(&fx);
    edit(
        &mut fx,
        vec![PlanEdit::CancelTask {
            task_id: "t2".into(),
        }],
    );
    let t1 = fx.task("t1");
    assert_eq!(t1.state, TaskState::Blocked);
    assert_eq!(t1.block.as_ref().unwrap().reason, BlockReason::DepCancelled);
    assert!(!t1.awaiting_deps, "the hold is gone with its dependency");
    let effects = fx.tick();
    assert!(ops_in(&effects, "HandBack").is_empty());
    assert!(delivers(&effects).is_empty());
}

/// Review minor 6: a failed delivery is retried after `DELIVERY_RETRY_SECS`, not in the
/// same step (a hot loop once a driver exists).
#[test]
fn a_failed_delivery_waits_before_it_is_retried() {
    use crate::run::messages::DELIVERY_RETRY_SECS;
    let mut fx = blocked_t1();
    let effects = edit(&mut fx, vec![answer("A")]);
    let ids: Vec<u64> = effects
        .iter()
        .filter_map(|e| match e {
            Effect::Deliver { message_ids, .. } => Some(message_ids.clone()),
            _ => None,
        })
        .flatten()
        .collect();
    assert_eq!(ids.len(), 1, "{effects:#?}");
    let failed_at = fx.now + 1;
    let effects = fx.send(
        failed_at,
        crate::run::engine::EventKind::Delivered {
            run_id: RUN_ID.into(),
            message_ids: ids,
            ok: false,
            error: Some("broken pipe".into()),
        },
    );
    assert!(delivers(&effects).is_empty(), "{effects:#?}");
    let effects = fx.send(
        failed_at + DELIVERY_RETRY_SECS - 1,
        crate::run::engine::EventKind::Tick,
    );
    assert!(delivers(&effects).is_empty(), "{effects:#?}");
    let effects = fx.send(
        failed_at + DELIVERY_RETRY_SECS,
        crate::run::engine::EventKind::Tick,
    );
    assert_eq!(delivers(&effects), vec![answer_message("A")]);
}

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
