//! Milestone 9.5 task 12: history's critical-path weights order dispatch (decision 7,
//! engine half); decision 15's pause accounting (ruling RE-4), which keeps paused time
//! and a restart's downtime out of the estimate's elapsed time; and a later round's
//! approval time (ruling RE-1). The estimate's arithmetic is `run/estimate_tests.rs`.

use proto::{PathWeights, PlanEdit, RunState, TaskState};
use serde_json::json;

use super::control::resume;
use super::control_restore::restart;
use super::dispatch::edit;
use super::fixture::*;
use super::goal_rounds_stages::{add_in, plan_round};
use super::goal_rounds_start::{complete, iterate, reply, started};
use crate::run::estimate::estimate;
use crate::run::refit::Tuned;

fn weights(s_secs: u64, m_secs: u64) -> PathWeights {
    PathWeights {
        s_secs,
        m_secs,
        hub_secs: m_secs,
        derived: vec!["hub".into()],
        at: 1,
    }
}

fn weighted(s_secs: u64, m_secs: u64) -> Tuned {
    Tuned {
        weights: Some(weights(s_secs, m_secs)),
        ..Tuned::default()
    }
}

/// One S task (6000 s) working in its window, its plan approved by `--yes`.
fn working_weighted() -> Fixture {
    let plan = plan_with(
        &profile_with("max_writers = 1"),
        &[task("t1", "S", "a", "")],
    );
    let mut fx = Fixture::with_tuning(&plan, weighted(6_000, 18_000));
    fx.ready(true);
    fx.launch_all();
    assert_eq!(fx.task("t1").state, TaskState::Working);
    fx
}

/// The bound ratio at `now`, t1 having done `done` seconds by then.
fn ratio_at(fx: &mut Fixture, now: u64, done: u64) -> u32 {
    let task = fx.task_mut("t1");
    task.phases = Default::default();
    task.phase_since = now - done;
    estimate(fx.run(), now)
        .and_then(|e| e.bound_ratio_permille)
        .expect("a ratio")
}

#[test]
fn weights_change_dispatch_order() {
    let plan = plan_with(
        &profile_with("max_writers = 1"),
        &[
            task("a", "S", "a", ""),
            task("a2", "S", "a2", "deps = [\"a\"]"),
            task("b", "M", "b", ""),
        ],
    );
    // Today's weights: a's chain is 1 + 1, b alone is 3.
    let mut fx = Fixture::new(&plan);
    fx.ready(true);
    assert_eq!(tasks_of(&fx.log, "PrepareWorktree"), vec!["b"]);
    // History's: a's chain is 2000 + 2000 s, b's 1000 s.
    let mut fx = Fixture::with_tuning(&plan, weighted(2_000, 1_000));
    fx.ready(true);
    assert_eq!(tasks_of(&fx.log, "PrepareWorktree"), vec!["a"]);
}

#[test]
fn paused_time_is_not_work() {
    let mut fx = working_weighted();
    let approved = fx.run().approved_at.expect("approved by --yes");
    fx.now = 3_000;
    edit(&mut fx, vec![PlanEdit::Pause]);
    assert_eq!(fx.run().state, RunState::Paused);
    assert_eq!((fx.run().paused_at, fx.run().paused_secs), (Some(3_001), 0));
    // While paused the elapsed time stands at 1000 s past the approval, and the open
    // phase counts only up to the pause: 500 s done at 3001 is all that is done at
    // 7001 (left 5500: (1000 + 5500) × 1000 / 6000).
    assert_eq!(approved, 2_001);
    let task = fx.task_mut("t1");
    (task.phases, task.phase_since) = (Default::default(), 2_501);
    let paused = estimate(fx.run(), 7_001).unwrap();
    assert_eq!(
        (paused.left_secs, paused.bound_ratio_permille),
        (5_500, Some(1_083))
    );
    fx.now = 8_000;
    edit(&mut fx, vec![PlanEdit::Resume]);
    assert_eq!(fx.run().state, RunState::Running);
    assert_eq!((fx.run().paused_at, fx.run().paused_secs), (None, 5_000));
    // 9001 is 7000 s after the approval, 5000 of them paused: (2000 + 6000) / 6000.
    assert_eq!(ratio_at(&mut fx, 9_001, 0), 1_333);
}

#[test]
fn a_restart_counts_downtime_as_paused() {
    // A run whose sessions the restart ended: from `Run.restored`.
    let mut fx = working_weighted();
    fx.now = 4_999;
    restart(&mut fx, Vec::new());
    assert_eq!(fx.run().state, RunState::Paused);
    assert_eq!(fx.run().restored, Some(5_000));
    assert_eq!(fx.run().paused_at, Some(5_000));
    fx.now = 9_099;
    resume(&mut fx);
    assert_eq!(fx.run().state, RunState::Running);
    assert_eq!((fx.run().paused_at, fx.run().paused_secs), (None, 4_100));

    // A run with no session yet: from the last step that changed it.
    let plan = plan_with(
        &profile_with("max_writers = 1"),
        &[task("t1", "S", "a", "")],
    );
    let mut fx = Fixture::with_tuning(&plan, weighted(6_000, 18_000));
    fx.ready(true);
    let last = fx.now;
    assert_eq!(fx.run().last_step_at, last);
    fx.now = 4_999;
    restart(&mut fx, Vec::new());
    assert_eq!(fx.run().restored, None);
    assert_eq!(fx.run().paused_at, Some(last));
    fx.now = 9_099;
    resume(&mut fx);
    assert_eq!(fx.run().paused_secs, 9_100 - last);
}

#[test]
fn tick_without_a_due_change_still_changes_nothing() {
    let mut fx = working_weighted();
    assert_eq!(fx.run().last_step_at, fx.now);
    let (before, revision) = (fx.run().clone(), fx.state.revision);
    fx.now += 5;
    let effects = fx.tick();
    assert!(effects.is_empty(), "{effects:#?}");
    assert_eq!(fx.run(), &before);
    assert_eq!(fx.state.revision, revision);
}

/// Ruling RE-1: a later round's approval is recorded on the round, and its estimate
/// runs from it; round 1's stays the run's.
#[test]
fn a_later_rounds_approval_is_recorded() {
    let mut fx = complete();
    let first = fx.run().approved_at;
    fx.run_mut().limits.path_weights = Some(weights(600, 1_800));
    assert_eq!(reply(&iterate(&mut fx, "more")), started(2));
    fx.now = 50_000;
    plan_round(&mut fx, json!([add_in("t2", "mail", 2, &[])]));
    let approved = fx.now;
    assert_eq!(fx.run().rounds[1].approved_at, Some(approved));
    assert_eq!(fx.run().rounds[0].approved_at, None);
    assert_eq!(fx.run().approved_at, first);
    // t2 alone is counted (t1, round 1's, merged): 300 s in, 300 s done.
    let task = fx.task_mut("t2");
    (task.state, task.phase_since, task.phases) =
        (TaskState::Working, approved, Default::default());
    let e = estimate(fx.run(), approved + 300).unwrap();
    assert_eq!((e.left_secs, e.bound_ratio_permille), (300, Some(1_000)));
}
