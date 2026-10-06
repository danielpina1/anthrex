//! Milestone 9.7 task M9.7.4 (DH §1.1): a merged stage's unfinished fix tasks. A
//! merged top stage cancels them with a reason; a stage with a live stage above keeps
//! them (propagate carries their commits up); a fix whose deferred cancel arrives too
//! late, and lands after the merge, says its work is not delivered.

use proto::TaskState;

use super::delivery_land::{ci_fix, head, logged, merged_view};
use super::delivery_open::answer;
use super::delivery_open::{green, open_stage, pr_on};
use super::delivery_sync::base_fetch;
use super::delivery_watch::fast;
use super::delivery_watch_adopt::poll_stage;
use super::fixture::*;
use super::full::attention;
use super::merge::{commit, doc_task, merge, to_queue, window_of};
use super::propagate::land_propagates;
use crate::host::FetchOutcome;
use crate::run::contract::sha7;
use crate::run::delivery::ops::HostResult;
use crate::run::engine::stages::set_stage_head;

/// A `pr` run of `tasks` with two writers: `t1` merged into stage 1, its PR #11 open
/// (polled every second), and a CI fix task of stage 1 working in its window.
fn with_a_fix(tasks: &[String]) -> (Fixture, String, Vec<(String, u32)>) {
    let (mut fx, windows) = pr_on(&profile_with("max_writers = 2"), tasks);
    fast(fx.run_mut());
    to_queue(&mut fx, "t1", window_of(&windows, "t1"));
    merge(&mut fx, "t1", &commit(1));
    land_propagates(&mut fx);
    green(&mut fx, 1);
    open_stage(&mut fx, 1, 11);
    let fix = ci_fix(&mut fx);
    let mut windows = Vec::new();
    for _ in 0..2 {
        windows.extend(fx.launch_all());
    }
    assert_eq!(
        fx.task(&fix).state,
        TaskState::Working,
        "the fix task works"
    );
    (fx, fix, windows)
}

pub(super) fn one_stage_with_a_fix() -> (Fixture, String, Vec<(String, u32)>) {
    with_a_fix(&[doc_task("t1", "")])
}

#[test]
fn a_merged_top_stage_cancels_its_working_fix_task_with_the_reason() {
    let (mut fx, fix, _) = one_stage_with_a_fix();
    let at = head(&fx, 1);
    poll_stage(&mut fx, 1, merged_view(11, &at, &commit(70)));
    let task = fx.task(&fix);
    assert_eq!(task.state, TaskState::Cancelled);
    assert_eq!(
        task.history.last().map(|e| e.text.as_str()),
        Some("cancelled: stage 1 PR merged"),
        "{:#?}",
        task.history
    );
}

#[test]
fn a_merged_stage_with_a_live_stage_above_keeps_its_fix_task() {
    // `t2` keeps stage 2 live: it is still pending or working.
    let (mut fx, fix, _) = with_a_fix(&[doc_task("t1", ""), doc_task("t2", "stage = 2")]);
    assert!(!fx.task("t2").state.is_finished());
    let before = fx.task(&fix).state;
    let at = head(&fx, 1);
    poll_stage(&mut fx, 1, merged_view(11, &at, &commit(70)));
    assert_eq!(fx.task(&fix).state, before, "propagate carries it up");
    assert!(
        !(fx.task(&fix).history.iter()).any(|e| e.text.starts_with("cancelled")),
        "{:#?}",
        fx.task(&fix).history
    );
}

#[test]
fn a_deferred_fix_task_that_lands_after_the_merge_says_it_is_not_delivered() {
    let (mut fx, fix, windows) = one_stage_with_a_fix();
    // 1. The fix task is in the merge queue, its candidate in flight.
    to_queue(&mut fx, &fix, window_of(&windows, &fix));
    assert_eq!(fx.task(&fix).state, TaskState::MergeQueue);
    // 2. The merged view arrives; the cancel is deferred behind the merge.
    let at = head(&fx, 1);
    poll_stage(&mut fx, 1, merged_view(11, &at, &commit(70)));
    assert!(
        fx.task(&fix).cancel_deferred,
        "{:#?}",
        fx.task(&fix).history
    );
    assert!(attention(&fx).iter().all(|l| !l.contains("not delivered")));
    // 3. The candidate answers `Merged`.
    let landed = commit(5);
    merge(&mut fx, &fix, &landed);
    // 4. The too-late line, then the not-delivered line and its attention line.
    let late = format!("the cancel of {fix} arrived too late: it merged");
    let unlanded = format!(
        "stage 1 PR #11 was merged at {}, without {}; that work is not delivered (anthrex run cancel gives up)",
        sha7(&at),
        sha7(&landed)
    );
    assert!(logged(&fx, &late), "{:#?}", fx.run().log);
    assert!(logged(&fx, &unlanded), "{:#?}", fx.run().log);
    let texts: Vec<&str> = fx.run().log.iter().map(|l| l.text.as_str()).collect();
    let (l, u) = (
        texts.iter().position(|t| *t == late),
        texts.iter().position(|t| *t == unlanded),
    );
    assert!(l < u, "too late, then not delivered: {texts:#?}");
    assert_eq!(
        fx.run().delivery.alerts.get("1/unlanded"),
        Some(&unlanded),
        "{:#?}",
        fx.run().delivery.alerts
    );
}

/// BR-6 (task M9.7.5): a deferred fix task that lands on an undecided merged stage
/// decides it as not delivered at once: its commit was made after the host's merge,
/// so the merge cannot hold it. The check's later answer is stale and changes nothing.
#[test]
fn a_deferred_fix_that_lands_on_an_undecided_merge_decides_it_not_delivered() {
    let (mut fx, fix, windows) = one_stage_with_a_fix();
    to_queue(&mut fx, &fix, window_of(&windows, &fix));
    // The local head moved past what any view showed, so the merge is undecided.
    fx.run_mut().delivery.stages[0].held = Some("protected branch".into());
    set_stage_head(fx.run_mut(), 1, &commit(60));
    poll_stage(&mut fx, 1, merged_view(11, &commit(80), &commit(70)));
    let undecided = |fx: &Fixture| fx.run().delivery.stage(1).unwrap().undecided.clone();
    assert_eq!(undecided(&fx), Some((commit(60), commit(80))));
    assert!(fx.task(&fix).cancel_deferred);
    let landed = commit(5);
    merge(&mut fx, &fix, &landed);
    assert_eq!(undecided(&fx), None);
    let unlanded = format!(
        "stage 1 PR #11 was merged at 80eeeee, without {}; that work is not delivered (anthrex run cancel gives up)",
        sha7(&landed)
    );
    assert_eq!(
        fx.run().delivery.alerts.get("1/unlanded"),
        Some(&unlanded),
        "{:#?}",
        fx.run().log
    );
    let (op, _) = base_fetch(&fx);
    let outcome = FetchOutcome::Fetched {
        sha: commit(71),
        parents: Some(1),
        contains: Some(true),
    };
    answer(&mut fx, op, HostResult::Fetched(outcome));
    assert_eq!(
        fx.run().delivery.alerts.get("1/unlanded"),
        Some(&unlanded),
        "a stale answer"
    );
}
