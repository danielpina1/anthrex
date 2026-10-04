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
use super::merge::{claim, doc_task};
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

/// One S task (6000 s) working in its window, its plan approved by `--yes`. Its own
/// budget is roomy; rung 4's ceiling (M's 60 minutes) still bounds a test's span.
fn working_weighted() -> (Fixture, u32) {
    let roomy = "[task.budget]\ntool_calls = 1000\nminutes = 1000";
    let plan = plan_with(&profile_with("max_writers = 1"), &[doc_task("t1", roomy)]);
    let mut fx = Fixture::with_tuning(&plan, weighted(6_000, 18_000));
    fx.ready(true);
    let window = fx.launch_all()[0].1;
    assert_eq!(fx.task("t1").state, TaskState::Working);
    (fx, window)
}

/// `(left, ratio)` at `now`.
fn at(fx: &Fixture, now: u64) -> (u64, Option<u32>) {
    let e = estimate(fx.run(), now).expect("an estimate");
    (e.left_secs, e.bound_ratio_permille)
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
    let (mut fx, window) = working_weighted();
    let approved = fx.run().approved_at.expect("approved by --yes");
    assert_eq!(approved, 2_001);
    // t1 has worked since 2101 (its phase's start; no pause yet). Every step below
    // comes within `stall_after_secs` of the worker's last event, so no stall, and
    // within rung 4's 60 minutes of session time.
    let task = fx.task_mut("t1");
    (task.phases, task.phase_since) = (Default::default(), 2_101);
    fx.now = 2_500;
    edit(&mut fx, vec![PlanEdit::Pause]);
    assert_eq!(fx.run().state, RunState::Paused);
    assert_eq!((fx.run().paused_at, fx.run().paused_secs), (Some(2_501), 0));
    // While paused neither the elapsed time (500 s) nor t1's work (400 s) grows:
    // left 5600, (500 + 5600) × 1000 / 6000.
    assert_eq!(at(&fx, 2_501), (5_600, Some(1_016)));
    assert_eq!(at(&fx, 4_001), (5_600, Some(1_016)));
    fx.now = 4_500;
    edit(&mut fx, vec![PlanEdit::Resume]);
    assert_eq!(fx.run().state, RunState::Running);
    assert_eq!((fx.run().paused_at, fx.run().paused_secs), (None, 2_000));
    // Ruling T12-2: at 5001, with the same phase still open, t1 has done 2900 − 2000
    // seconds, so `left` does not collapse to 3100: it is 5100, and the run is as far
    // behind as before the pause.
    assert_eq!(fx.task("t1").phase_since, 2_101);
    assert_eq!(at(&fx, 5_001), (5_100, Some(1_016)));
    // The phase ends: its paused seconds move with it, and the next phase starts from
    // the run's paused total, so the pause is never counted as work later either.
    fx.now = 5_000;
    claim(&mut fx, "t1", window, HEAD);
    let task = fx.task("t1");
    assert_ne!(task.state, TaskState::Working);
    assert_eq!((task.paused.active, task.paused.base), (2_000, 2_000));
    let now = fx.now + 10;
    assert_eq!(at(&fx, now).0, 6_000 - (now - 4_101));
}

#[test]
fn a_restart_counts_downtime_as_paused() {
    // Ruling T12-1: a run whose sessions the restart ended is paused from its last
    // change, not from the restore.
    let (mut fx, _) = working_weighted();
    let last = fx.run().last_step_at;
    assert_eq!(last, fx.now);
    fx.now = 4_999;
    restart(&mut fx, Vec::new());
    assert_eq!(fx.run().state, RunState::Paused);
    assert_eq!(fx.run().restored, Some(5_000));
    assert_eq!(fx.run().paused_at, Some(last));
    fx.now = 9_099;
    resume(&mut fx);
    assert_eq!(fx.run().state, RunState::Running);
    assert_eq!(
        (fx.run().paused_at, fx.run().paused_secs),
        (None, 9_100 - last)
    );

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
    let (mut fx, _) = working_weighted();
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

/// Review m1: a run the restore leaves halted, and a paused one stored before 9.5
/// (no `paused_at`, no `last_step_at`), are paused from their last change, else from
/// the restore.
#[test]
fn a_run_restored_stopped_is_paused_from_its_last_change() {
    let (mut fx, _) = working_weighted();
    let last = fx.run().last_step_at;
    let run = fx.run_mut();
    run.state = RunState::Halted;
    run.halted_reason = Some("refs/heads/main was deleted".into());
    fx.now = 4_999;
    restart(&mut fx, Vec::new());
    assert_eq!(fx.run().state, RunState::Halted);
    assert_eq!(fx.run().paused_at, Some(last));

    let (mut fx, _) = working_weighted();
    let run = fx.run_mut();
    (run.state, run.paused_from) = (RunState::Paused, Some(RunState::Running));
    run.last_step_at = 0;
    fx.now = 4_999;
    restart(&mut fx, Vec::new());
    assert_eq!(fx.run().paused_at, Some(5_000));
    fx.now = 9_099;
    resume(&mut fx);
    assert_eq!(fx.run().paused_secs, 4_100);
}

/// Task 12's carry N1: a check in flight when the daemon stops, its result replayed by
/// the restore. The phase it closes held the downtime, which is paused time (ruling
/// T12-1), so it moves into the task's paused seconds and the next phase starts from
/// the run's paused total, as any step's phase change does.
#[test]
fn a_phase_change_replayed_at_a_restore_keeps_the_pause_bookkeeping() {
    let (mut fx, window) = working_weighted();
    claim(&mut fx, "t1", window, HEAD);
    let (op, _) = super::merge::pending_one(&fx, "Check", Some("t1"));
    assert_eq!(fx.task("t1").state, TaskState::Check);
    let last = fx.run().last_step_at;
    let run = fx.run().clone();
    fx.state = crate::run::engine::EngineState::default();
    let back = last + 3_000;
    fx.send(
        back,
        crate::run::engine::EventKind::Restore {
            held: Vec::new(),
            runs: vec![run],
            replay: vec![(RUN_ID.into(), op, super::gates::check_result(true))],
        },
    );
    let run = fx.run();
    let task = fx.task("t1");
    assert_ne!(
        task.state,
        TaskState::Check,
        "the replayed check moved it on"
    );
    assert_eq!(run.paused_total(back), back - last);
    assert_eq!(
        (task.paused.active, task.paused.base),
        (back - last, back - last),
        "the downtime is paused time, never work"
    );
}
