//! M8a.11 fix round 1, ruling T11-I1..I3: the N5 hold is task state. A started task
//! that gains an unfinished dependency is held: it stays blocked, answers queue, and
//! it resumes only after one hand-back with every dependency finished. The three
//! sequences are the review's probes r1–r3; r6 is minor 8.

use proto::{BlockReason, PlanEdit, PlanTask, TaskState};

use super::dispatch::{edit, replies};
use super::fixture::*;
use crate::run::contract::{answer_message, conflict_message};
use crate::run::engine::{AgentSignal, Effect, OpResult, TurnOutcome};
use crate::run::messages::join_turn;
use crate::run::model::{OpId, Outgoing};

pub(super) fn plan_task(id: &str, owns: &str) -> PlanTask {
    let text = plan_with(PROFILE, &[task_toml(id, "S", owns, "")]);
    crate::run::plan::parse_plan(&text).unwrap().tasks.remove(0)
}

pub(super) fn delivers(effects: &[Effect]) -> Vec<String> {
    effects
        .iter()
        .filter_map(|e| match e {
            Effect::Deliver { text, .. } => Some(text.clone()),
            _ => None,
        })
        .collect()
}

pub(super) fn joined(texts: &[String]) -> String {
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

pub(super) fn add_dep(task: &str, dep: &str) -> PlanEdit {
    PlanEdit::AddDep {
        task_id: task.into(),
        dep: dep.into(),
    }
}

pub(super) fn answer(text: &str) -> PlanEdit {
    PlanEdit::Answer {
        task_id: "t1".into(),
        text: text.into(),
    }
}

/// t1 started and `blocked(question)` with its turn closed; t2 unstarted, overlapping.
pub(super) fn blocked_t1() -> Fixture {
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

pub(super) fn assert_held(fx: &Fixture) {
    let t1 = fx.task("t1");
    assert_eq!(t1.state, TaskState::Blocked);
    assert!(t1.awaiting_deps, "t1 carries the hold");
}

/// `[add_dep t1 t2, answer t1 "A"]`, t2 merged: returns the in-flight hand-back.
pub(super) fn hand_back_in_flight(fx: &mut Fixture) -> OpId {
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
