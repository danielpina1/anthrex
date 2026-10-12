//! Milestone 9.13 decisions 14-17 (task M9.13.7): the orchestrator retries a task at most
//! twice (`MAX_ORCH_RETRIES`); the block after that is the user's, and a user's retry
//! gives the orchestrator its two again.

use proto::{BlockReason, RaceLane, TaskState};
use serde_json::json;

use super::control::{blocked, retry};
use super::dispatch::replies;
use super::fixture::*;
use super::gates::only_op;
use super::orch::{edit_plan, error};
use super::race::racing;
use super::race_lanes::{HEAD_B, passes};
use super::turns::killed_exit;
use super::wake_notes::approved;
use crate::run::engine::{Actor, Effect, OpResult, requests};
use crate::run::model::Run;
use crate::run::orch::TaskOrch;

const SUFFIX: &str = " · the orchestrator retried it 2 times; the user decides";

/// `approved()` with `t1` launched and blocked on `text` (an ordinary environment block);
/// its window.
fn blocked_t1(text: &str) -> (Fixture, u32) {
    let mut fx = approved();
    let windows = fx.launch_all();
    let (_, window) = windows.iter().find(|(t, _)| t == "t1").cloned().unwrap();
    blocked(&mut fx, window, "environment", text);
    (fx, window)
}

/// The retry's effects (`retried`) have ended `window`'s session; finish the re-entry
/// and block `t1` again in its fresh window, which is returned.
fn reblock(fx: &mut Fixture, mut retried: Vec<Effect>, window: u32, text: &str) -> u32 {
    retried.extend(killed_exit(fx, window));
    let (op, _) = only_op(&retried, "DiffSoFar");
    let (stat, patch) = (" a | 1 +".to_string(), "+x".to_string());
    fx.done(op, OpResult::Diff { stat, patch });
    let windows = fx.complete_windows();
    let (_, window) = windows.iter().find(|(t, _)| t == "t1").cloned().unwrap();
    blocked(fx, window, "environment", text);
    window
}

fn orch_retry(fx: &mut Fixture) -> Vec<Effect> {
    edit_plan(
        fx,
        json!({"edits": [{"op": "retry", "task_id": "t1", "reason": "transient"}]}),
    )
}

fn count(fx: &Fixture) -> u32 {
    fx.task("t1").orch.orch_retries
}

#[test]
fn orchestrator_retries_count_and_survive_rung2() {
    let (mut fx, _) = blocked_t1("the build cache is locked");
    assert_eq!(count(&fx), 0);
    let effects = orch_retry(&mut fx);
    assert!(replies(&effects)[0].is_ok(), "{effects:#?}");
    // The rung 2 re-entry reset every other counter and left this one.
    assert_eq!(fx.task("t1").rung, 2);
    assert_eq!(count(&fx), 1);
}

/// Block, retry, block, retry, block: the third block is the user's.
fn twice_retried() -> (Fixture, u32) {
    let (mut fx, window) = blocked_t1("the build cache is locked");
    let effects = orch_retry(&mut fx);
    assert!(replies(&effects)[0].is_ok());
    let window = reblock(&mut fx, effects, window, "the build cache is locked");
    let first = fx.task("t1").block.clone().unwrap();
    assert!(!first.user_only);
    assert!(!first.text.ends_with(SUFFIX), "{}", first.text);
    let effects = orch_retry(&mut fx);
    assert!(replies(&effects)[0].is_ok());
    assert_eq!(count(&fx), 2);
    let window = reblock(&mut fx, effects, window, "the build cache is locked");
    (fx, window)
}

#[test]
fn the_third_block_after_two_orchestrator_retries_is_user_only() {
    let (mut fx, _) = twice_retried();
    let block = fx.task("t1").block.clone().unwrap();
    assert!(block.text.ends_with(SUFFIX), "{}", block.text);
    assert!(block.user_only);
    assert_eq!(block.reason, BlockReason::Environment);
    // The orchestrator is refused at once.
    let effects = orch_retry(&mut fx);
    assert!(!error(&effects).is_empty());
    assert_eq!(count(&fx), 2);
}

#[test]
fn a_retry_at_the_cap_is_refused() {
    let (mut fx, _) = blocked_t1("the build cache is locked");
    fx.task_mut("t1").orch.orch_retries = 2;
    let effects = orch_retry(&mut fx);
    assert_eq!(
        error(&effects),
        "task t1 was retried by the orchestrator 2 times; only the user can retry it now"
    );
    assert_eq!(fx.task("t1").state, TaskState::Blocked);
    assert_eq!(count(&fx), 2);
    // C20: not blocked, the user's own rule speaks first.
    let mut fx = approved();
    fx.launch_all();
    fx.task_mut("t1").orch.orch_retries = 2;
    let user = replies(&retry(&mut fx, "t1"))[0].clone().unwrap_err();
    let effects = orch_retry(&mut fx);
    assert_eq!(error(&effects), user);
}

#[test]
fn a_user_retry_resets_the_count() {
    let (mut fx, window) = twice_retried();
    assert_eq!(count(&fx), 2);
    let effects = retry(&mut fx, "t1");
    assert!(replies(&effects)[0].is_ok(), "{effects:#?}");
    assert_eq!(count(&fx), 0);
    reblock(&mut fx, effects, window, "the build cache is locked");
    assert!(!fx.task("t1").block.as_ref().unwrap().user_only);
    assert!(replies(&orch_retry(&mut fx))[0].is_ok());
    assert_eq!(count(&fx), 1);
}

/// C19: the crown path returns before the rewrite reset; both actors still count right.
#[test]
fn crown_retry_counts() {
    for (actor, want) in [(Actor::Orchestrator, 1), (Actor::User, 0)] {
        let (mut fx, _, _) = racing();
        let effects = passes(&mut fx, RaceLane::B, HEAD_B);
        let (op, _) = only_op(&effects, "CrownRacer");
        let message = "disk full".into();
        fx.done(op, OpResult::Failed { message });
        assert_eq!(fx.task("t1").state, TaskState::Blocked);
        fx.task_mut("t1").orch.orch_retries = if actor == Actor::User { 2 } else { 0 };
        let mut out = Vec::new();
        let result = requests::retry_on(fx.run_mut(), "t1", actor, 1, &mut out);
        let text = result.unwrap();
        assert!(text.contains("crowned again"), "{text}");
        assert_eq!(count(&fx), want, "{actor:?}");
    }
}

#[test]
fn count_survives_save_and_restore() {
    let (mut fx, _) = blocked_t1("the build cache is locked");
    fx.task_mut("t1").orch.orch_retries = 2;
    let json = serde_json::to_string(fx.run()).unwrap();
    let run: Run = serde_json::from_str(&json).unwrap();
    assert_eq!(run.tasks[0].orch.orch_retries, 2);
    // A pre-9.13 task has no key: it loads as 0.
    // A zero is not written, so an older run's JSON round-trips unchanged.
    let value = serde_json::to_value(TaskOrch::default()).unwrap();
    assert!(value.get("orch_retries").is_none(), "{value}");
    let old: TaskOrch = serde_json::from_value(value).unwrap();
    assert_eq!(old.orch_retries, 0);
}
