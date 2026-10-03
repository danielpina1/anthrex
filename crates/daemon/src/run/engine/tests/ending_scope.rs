//! Milestone 9.3's final fix wave, W1 fix round 3: the round-scoped ending
//! (`full::ending_at`, fix round 2's item 4) at each of its uses beyond the bisect
//! (`bisect_ends.rs`): a CI red (`delivery/ci.rs::gone`), the completion's tier-3 hold
//! (`full::completion`, `full::holds_completion`) and the idle tier 3
//! (`full::idle_pass`). While round 2 is being cancelled, a stage of that round ends
//! with it and an earlier round's stage does not.
//!
//! The round is set in place, on round 1's single stage. Its cancel is
//! `goal_rounds_cancel`'s; here only the scope of the finish matters. `first_stage` 1
//! makes stage 1 the cancelled round's, and 2 makes it an earlier round's.

use proto::{RoundOutcome, RunState, TaskOrigin};

use super::delivery_ci::{RUN_A, answer_logs, ci_fixes, red_view, test_red, tier2};
use super::delivery_watch::{poll_with, watched};
use super::fixture::*;
use super::full::{
    block, full_jobs, later, merge_tiered, outcome, profile, red_at_completion, tier, verify_ok,
};
use super::merge::{commit, doc_task, start_on, window_of};
use crate::run::contract::sha7;
use crate::run::engine::EventKind;
use crate::run::engine::fixes::add_fix;
use crate::run::slots::Priority;

/// Round 2 open and being cancelled from stage `first_stage` (the round's `finish`).
fn cancelling(fx: &mut Fixture, first_stage: u16) {
    let run = fx.run_mut();
    let mut round = run.rounds.last().cloned().expect("round 1's record");
    run.rounds.last_mut().unwrap().ended_at = Some(1);
    round.n = 2;
    round.first_stage = first_stage;
    round.outcome = Some(RoundOutcome::Cancelled);
    round.ended_at = None;
    round.summary = None;
    run.rounds.push(round);
    run.finish_edit = true;
    run.round_finish = true;
}

/// A `pr` round ends once every task finished (`goal_rounds_end::delivered`): a
/// round-1 fix still to run keeps round 2 winding down (the round's cancel spares it;
/// its round is set to 1 in place, as the in-place round may own its stage).
fn held_open(fx: &mut Fixture) {
    let spec = super::fixes::spec(TaskOrigin::Bisect, "docs/other/**", Default::default());
    let now = fx.now;
    let id = add_fix(fx.run_mut(), spec, now, &mut Vec::new()).expect("added");
    fx.task_mut(&id).round = 1;
}

/// Round 2 is still being cancelled: nothing ended it during the test.
fn still_cancelling(fx: &Fixture) {
    let run = fx.run();
    assert_eq!(run.rounds[1].ended_at, None, "{:#?}", run.log);
    assert!(run.finish_edit && run.round_finish);
}

fn not_acted_on(why: &str) -> String {
    format!(
        "stage 1: CI red at {} is not acted on: {why}",
        sha7(&commit(1))
    )
}

fn logged(fx: &Fixture, text: &str) -> bool {
    fx.run().log.iter().any(|l| l.text == text)
}

/// `ci::gone`: a CI red on an earlier round's stage gets its fix task; one on the
/// cancelled round's own stage is dropped, as a finishing run's is.
#[test]
fn a_ci_red_ends_with_the_cancelled_rounds_stage_only() {
    let mut fx = watched();
    cancelling(&mut fx, 2);
    held_open(&mut fx);
    poll_with(&mut fx, red_view(&commit(1), test_red(RUN_A)));
    answer_logs(&mut fx, "--- FAIL: a::works");
    let (op, _) = tier2(&fx).expect("a tier-2 reproduction");
    fx.done(op, tier(outcome(2, &[])));
    assert_eq!(ci_fixes(&fx).len(), 1, "{:#?}", fx.run().log);
    assert!(!logged(&fx, &not_acted_on("the run is ending")));
    still_cancelling(&fx);

    let mut fx = watched();
    cancelling(&mut fx, 1);
    held_open(&mut fx);
    poll_with(&mut fx, red_view(&commit(1), test_red(RUN_A)));
    answer_logs(&mut fx, "--- FAIL: a::works");
    if let Some((op, _)) = tier2(&fx) {
        fx.done(op, tier(outcome(2, &[])));
    }
    still_cancelling(&fx);
    assert!(ci_fixes(&fx).is_empty(), "{:#?}", fx.run().log);
    assert!(
        logged(&fx, &not_acted_on("the run is ending")),
        "{:#?}",
        fx.run().log
    );
}

/// `completion` and `holds_completion`: a red tier 3 on the cancelled round's stage no
/// longer holds the run's completion; one on an earlier round's stage still does.
#[test]
fn a_red_tier3_holds_completion_unless_its_stage_is_the_cancelled_rounds() {
    let mut fx = red_at_completion();
    cancelling(&mut fx, 2);
    let effects = later(&mut fx, 1_000);
    assert!(ops_in(&effects, "VerifyRefs").is_empty(), "{effects:#?}");
    assert_eq!(fx.run().state, RunState::Running);
    still_cancelling(&fx);

    let mut fx = red_at_completion();
    cancelling(&mut fx, 1);
    fx.tick();
    let effects = verify_ok(&mut fx);
    assert!(full_jobs(&effects).is_empty(), "{effects:#?}");
    assert_eq!(fx.run().state, RunState::Complete, "{:#?}", fx.run().log);
    assert!(fx.run().final_check_failed);
}

/// `idle_pass`: the idle tier 3 skips the cancelled round's stage and still runs on an
/// earlier round's.
#[test]
fn the_idle_tier3_skips_the_cancelled_rounds_stage_only() {
    let idle_jobs = |first_stage: u16| {
        let tasks = [doc_task("t1", ""), doc_task("t3", "")];
        let (mut fx, windows) = start_on(&profile(), &tasks);
        // A round-1 task blocked keeps the run from completing.
        block(&mut fx, "t3", window_of(&windows, "t3"));
        merge_tiered(&mut fx, "t1", window_of(&windows, "t1"), &commit(1));
        cancelling(&mut fx, first_stage);
        let since = fx.run().queue_idle_since.expect("the queue is idle");
        let idle = fx.run().limits.testing.full_idle_secs;
        let effects = fx.send(since + idle, EventKind::Tick);
        still_cancelling(&fx);
        (full_jobs(&effects).into_iter())
            .map(|(_, spec)| (spec.stage, spec.priority))
            .collect::<Vec<_>>()
    };
    assert_eq!(idle_jobs(2), [(1, Priority::FullIdle)]);
    assert_eq!(idle_jobs(1), []);
}
