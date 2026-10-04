//! The final fix wave's race items in the reducer: a task with its own checkout never
//! races (A-I2).

use proto::{AgentRole, PlanEdit, TaskState};
use serde_json::json;

use super::dispatch::{edit, replies, task_path};
use super::fixture::*;
use super::race::{RACING, launched};
use crate::run::engine::OpResult;

/// The note and log line of a task that already has its own checkout.
const HAS_CHECKOUT: &str = "race skipped: the task already has a checkout";

fn amend(id: &str, race: Option<bool>, deps: Option<Vec<String>>) -> PlanEdit {
    PlanEdit::AmendTask {
        task_id: id.into(),
        brief: None,
        acceptance: None,
        route: None,
        test_mode: None,
        test_mode_reason: None,
        priority: None,
        size: None,
        deps,
        stage: None,
        race,
        pair: None,
    }
}

/// `t1` runs one ordinary worker in its own checkout `t1`, never a race, and the note
/// says why, once.
fn single_in_own_checkout(fx: &Fixture) {
    let t1 = fx.task("t1");
    assert!(t1.race.is_none(), "{:?}", t1.race);
    assert_eq!(t1.worktree, task_path("t1"));
    assert_eq!(t1.notes.iter().filter(|n| *n == HAS_CHECKOUT).count(), 1);
    let line = format!("task t1: {HAS_CHECKOUT}");
    assert_eq!(fx.run().log.iter().filter(|l| l.text == line).count(), 1);
    assert!(fx.ops("CrownRacer").is_empty());
    let last = t1.rounds.last().expect("a session");
    assert_eq!((last.role, last.lane), (AgentRole::Worker, None));
    // Accept and discard list `task.worktree`: the pre-warm or start checkout is not
    // leaked.
    let racers = (fx.ops("CreateWindow").into_iter())
        .filter(|(_, kind)| format!("{kind:?}").contains("t1.a"))
        .count();
    assert_eq!(racers, 0, "no lane checkout is launched");
}

/// A-I2, route 1: a task pre-warmed at the plan gate is amended to `race = true`; its
/// pre-warmed checkout is its own, so it runs single there.
#[test]
fn a_prewarmed_task_amended_to_race_runs_single_in_its_checkout() {
    let mut fx = Fixture::new(&plan_with(PROFILE, &[task("t1", "M", "a", "")]));
    fx.ready(false);
    fx.complete_prepares();
    assert!(fx.task("t1").prewarmed);
    let effects = edit(&mut fx, vec![amend("t1", Some(true), None)]);
    assert!(replies(&effects)[0].is_ok(), "{effects:#?}");
    assert!(fx.task("t1").spec.race);
    fx.approve();
    fx.complete_prepares();
    fx.complete_windows();
    assert_eq!(fx.task("t1").state, TaskState::Working);
    single_in_own_checkout(&fx);
}

/// A-I2, route 2: a racing task decided single starts in its own checkout, is blocked
/// on a dependency that is cancelled, and the amend of its deps releases it to
/// `pending` (which clears the latch). Decided again, it still runs single in its own
/// checkout: it has a start commit.
#[test]
fn a_started_task_released_to_pending_runs_single_again() {
    let tasks = [
        task("t0", "M", "z", "priority = 1"),
        task("t1", "S", "a", RACING),
    ];
    let mut fx = launched(PROFILE, &tasks, config::Orchestrator::default());
    let t1 = fx.task("t1");
    assert!(
        t1.race.is_none() && t1.start_commit.is_some(),
        "{:?}",
        t1.state
    );
    let window = (t1.rounds.iter().rfind(|r| r.role == AgentRole::Worker))
        .and_then(|r| r.window_id)
        .expect("t1's worker");
    let args = json!({"kind": "question", "reason": "which API?"});
    fx.tool_as(AgentRole::Worker, window, "t1", "task_blocked", args);
    assert_eq!(fx.task("t1").state, TaskState::Blocked);
    let add = PlanEdit::AddDep {
        task_id: "t1".into(),
        dep: "t0".into(),
    };
    let effects = edit(&mut fx, vec![add]);
    assert!(replies(&effects)[0].is_ok(), "{effects:#?}");
    let cancel = PlanEdit::CancelTask {
        task_id: "t0".into(),
    };
    edit(&mut fx, vec![cancel]);
    assert_eq!(
        fx.task("t1").block.as_ref().map(|b| b.reason),
        Some(proto::BlockReason::DepCancelled)
    );
    let effects = edit(&mut fx, vec![amend("t1", None, Some(Vec::new()))]);
    assert!(replies(&effects)[0].is_ok(), "{effects:#?}");
    for (op, _) in fx.ops("PrepareWorktree") {
        if fx.run().pending_ops.contains_key(&op) {
            fx.done(op, OpResult::Worktree { head: BASE.into() });
        }
    }
    fx.complete_windows();
    let t1 = fx.task("t1");
    assert!(
        !matches!(t1.state, TaskState::Pending | TaskState::Queued),
        "{:?}",
        t1.state
    );
    single_in_own_checkout(&fx);
}
