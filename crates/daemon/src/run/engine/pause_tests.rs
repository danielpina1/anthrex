//! Task 12's carry N2: a `pr`-mode review wait open across a pause is not taken off
//! the estimate's elapsed time twice (once as paused time, once as review time).

use proto::{DeliveryMode, PathWeights, RunState, TaskState};
use serde_json::json;

use super::*;
use crate::run::delivery::{PrRecord, StageDelivery};
use crate::run::estimate::estimate;
use crate::run::refit::Tuned;
use crate::run::test_support::{build_tuned, plan_with, task_toml};

const APPROVED: u64 = 1_000_000;

/// One S task (600 s), running, approved at [`APPROVED`], in `pr` mode with its stage's
/// pull request waiting on a person since `APPROVED + 100`.
fn waiting_on_review() -> Run {
    let weights = PathWeights {
        s_secs: 600,
        m_secs: 1_800,
        hub_secs: 3_600,
        derived: Vec::new(),
        at: 1,
    };
    let tuning = Tuned {
        weights: Some(weights),
        ..Tuned::default()
    };
    let plan = plan_with(
        "goal = \"Estimate\"\nmax_writers = 1\n[profile]\nmodules = [\"crates/*\"]\n",
        &[task_toml("a", "S", "[\"crates/a/**\"]", "")],
    );
    let mut run = build_tuned(&plan, &config::Orchestrator::default(), tuning)
        .unwrap_or_else(|e| panic!("{e:?}"));
    run.state = RunState::Running;
    run.approved_at = Some(APPROVED);
    run.delivery.mode = DeliveryMode::Pr;
    let pr: PrRecord = serde_json::from_value(json!({
        "number": 7, "url": "https://github.com/fake/app/pull/7", "base": "main",
        "opened_at": APPROVED, "pushed_head": "h", "state": "open"
    }))
    .unwrap();
    run.delivery.stages = vec![StageDelivery {
        pr: Some(pr),
        wait_from: Some(APPROVED + 100),
        ..StageDelivery::default()
    }];
    run
}

/// The run steps to `state` at `now`, its pause accounted.
fn step_to(run: &mut Run, state: RunState, now: u64) {
    let old = run.clone();
    run.state = state;
    account(&old, run, now);
}

#[test]
fn a_review_wait_open_across_a_pause_is_not_subtracted_twice() {
    let mut run = waiting_on_review();
    step_to(&mut run, RunState::Paused, APPROVED + 400);
    step_to(&mut run, RunState::Running, APPROVED + 1_400);
    assert_eq!(run.paused_secs, 1_000);
    // `a` starts now: nothing done, 600 s left on a 600 s bound.
    let end = APPROVED + 1_500;
    let a = &mut run.tasks[0];
    (a.state, a.phase_since) = (TaskState::Working, end);
    // 1500 s since the approval, 1000 paused, and 400 waiting on the review outside
    // the pause (300 before it, 100 after): 100 s elapsed, (100 + 600) × 1000 / 600.
    let e = estimate(&run, end).expect("an estimate");
    assert_eq!((e.left_secs, e.bound_ratio_permille), (600, Some(1_166)));
}

/// Ruling FW-2 (b): the resume step's delivery pass runs before the pause is
/// accounted, so it books the wait across the pause into `review_wait_secs` (decision
/// 43: a pause does not stop a person's wait) and reopens it at the resume. The
/// estimate still takes the pause off once.
#[test]
fn a_review_wait_booked_across_a_pause_is_not_subtracted_twice() {
    let mut run = waiting_on_review();
    step_to(&mut run, RunState::Paused, APPROVED + 400);
    // The resume step: its pass books the wait since `APPROVED + 100`, then accounts.
    let old = run.clone();
    run.state = RunState::Running;
    let stage = &mut run.delivery.stages[0];
    stage.review_wait_secs = 1_300;
    stage.wait_from = Some(APPROVED + 1_400);
    account(&old, &mut run, APPROVED + 1_400);
    assert_eq!(run.paused_secs, 1_000);
    assert_eq!(run.delivery.stages[0].review_wait_secs, 1_300, "wall time");
    let end = APPROVED + 1_500;
    let a = &mut run.tasks[0];
    (a.state, a.phase_since) = (TaskState::Working, end);
    // As above: 100 s elapsed outside both the pause and the wait.
    let e = estimate(&run, end).expect("an estimate");
    assert_eq!((e.left_secs, e.bound_ratio_permille), (600, Some(1_166)));
}
