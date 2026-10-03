//! Milestone 9.5 task 12: decision 14's run estimate and bound ratio, round-aware
//! (rulings RE-1 to RE-3), and the snapshot that carries them. Pure.

use proto::{DeliveryMode, PathWeights, RunState, TaskState};
use serde_json::json;

use super::*;
use crate::run::delivery::{PrRecord, StageDelivery};
use crate::run::engine::EngineState;
use crate::run::model::{FixOf, Round, Task};
use crate::run::refit::Tuned;
use crate::run::snapshot::snapshot;
use crate::run::test_support::{build_tuned, plan_with, task_toml};

/// When the fixtures' current round was approved.
const APPROVED: u64 = 1_000_000;

/// S 600 s, M 1800 s, hub 3600 s.
fn weights() -> PathWeights {
    PathWeights {
        s_secs: 600,
        m_secs: 1_800,
        hub_secs: 3_600,
        derived: Vec::new(),
        at: 1,
    }
}

fn profile(limits: &str) -> String {
    format!("goal = \"Estimate\"\n{limits}\n[profile]\nmodules = [\"crates/*\"]\n")
}

/// A task owning `crates/<module>/**`.
fn code(id: &str, size: &str, module: &str, extra: &str) -> String {
    task_toml(id, size, &format!("[\"crates/{module}/**\"]"), extra)
}

/// A research task (a reader, decision 35 of milestone 9).
fn research(id: &str) -> String {
    task_toml(id, "S", "[]", "kind = \"research\"")
}

/// `tasks` built with `max_writers` and `weights` frozen, running, approved at
/// [`APPROVED`].
fn run_with(max_writers: u8, weights: Option<PathWeights>, tasks: &[String]) -> Run {
    let tuning = Tuned {
        weights,
        ..Tuned::default()
    };
    let limits = format!("max_writers = {max_writers}");
    let plan = plan_with(&profile(&limits), tasks);
    let mut run = build_tuned(&plan, &config::Orchestrator::default(), tuning)
        .unwrap_or_else(|e| panic!("{e:?}"));
    run.state = RunState::Running;
    run.approved_at = Some(APPROVED);
    run
}

fn three_s() -> Run {
    let tasks = ["a", "b", "c"].map(|id| code(id, "S", id, ""));
    run_with(1, Some(weights()), &tasks)
}

fn task<'a>(run: &'a mut Run, id: &str) -> &'a mut Task {
    run.tasks.iter_mut().find(|t| t.id() == id).unwrap()
}

/// `task` in `state` since `since`, with `secs` of earlier active work.
fn at(task: &mut Task, state: TaskState, since: u64, secs: u64) {
    task.state = state;
    task.phase_since = since;
    task.phases.working = secs;
}

fn est(run: &Run, now: u64) -> (u64, Option<u32>) {
    let e = estimate(run, now).expect("an estimate");
    (e.left_secs, e.bound_ratio_permille)
}

#[test]
fn no_weights_no_estimate() {
    let tasks = [code("a", "S", "a", "")];
    assert_eq!(estimate(&run_with(1, None, &tasks), APPROVED), None);
    assert!(estimate(&run_with(1, Some(weights()), &tasks), APPROVED).is_some());
}

#[test]
fn estimate_of_a_fresh_run() {
    // left = max(600, 1800 / 1), bound the same: on time right after approval.
    assert_eq!(est(&three_s(), APPROVED), (1_800, Some(1_000)));
    // Two writers: left and bound are both 900.
    let tasks = ["a", "b", "c"].map(|id| code(id, "S", id, ""));
    let run = run_with(2, Some(weights()), &tasks);
    assert_eq!(est(&run, APPROVED), (900, Some(1_000)));
}

#[test]
fn estimate_counts_active_time_and_floors_remaining() {
    let mut run = three_s();
    let now = APPROVED + 400;
    // a: 200 s of earlier work and 100 s in `working` now; its queued time is no work.
    at(task(&mut run, "a"), TaskState::Working, now - 100, 200);
    task(&mut run, "a").phases.queued = 5_000;
    // b: past its weight, floored at a tenth of it; c merged.
    at(task(&mut run, "b"), TaskState::Review, now, 700);
    at(task(&mut run, "c"), TaskState::Merged, now, 600);
    // left = max(300, 300 + 60 + 0) = 360; (400 + 360) × 1000 / 1800 = 422.
    assert_eq!(est(&run, now), (360, Some(422)));
}

#[test]
fn ratio_rises_when_the_run_falls_behind() {
    assert_eq!(est(&three_s(), APPROVED + 1_200), (1_800, Some(1_666)));
}

#[test]
fn readers_count_on_paths_but_not_in_the_work_sum() {
    // r1 → w1 is the longest path (1200 s); r2 reads beside it. The work sum is w1's
    // alone: the readers hold no writer slot.
    let tasks = [
        research("r1"),
        research("r2"),
        code("w1", "S", "w", "deps = [\"r1\"]"),
    ];
    let run = run_with(1, Some(weights()), &tasks);
    assert_eq!(est(&run, APPROVED), (1_200, Some(1_000)));
}

/// [`three_s`] as round 2, approved at [`APPROVED`]: round 1 (task `old`, merged, and
/// the fix tasks `fix` unfinished and `done` merged) was approved days earlier.
fn round_two() -> Run {
    let tasks = [
        code("old", "M", "old", ""),
        code("fix", "S", "fix", ""),
        code("done", "M", "done", ""),
        code("n1", "S", "n1", ""),
        code("n2", "M", "n2", ""),
    ];
    let mut run = run_with(1, Some(weights()), &tasks);
    run.approved_at = Some(APPROVED - 300_000);
    let mut first = Round::first(&run);
    first.ended_at = Some(APPROVED - 200_000);
    let second = Round {
        n: 2,
        started_at: APPROVED - 100,
        ended_at: None,
        first_stage: 2,
        approved_at: Some(APPROVED),
        ..first.clone()
    };
    run.rounds = vec![first, second];
    let fix = FixOf::Bisect {
        culprit: "old".into(),
        stage: 1,
        tests: Vec::new(),
    };
    for id in ["old", "fix", "done"] {
        task(&mut run, id).round = 1;
    }
    for id in ["n1", "n2"] {
        task(&mut run, id).round = 2;
    }
    at(task(&mut run, "old"), TaskState::Merged, 1, 0);
    task(&mut run, "fix").fixes = Some(fix.clone());
    task(&mut run, "done").fixes = Some(fix);
    at(task(&mut run, "done"), TaskState::Merged, 1, 0);
    run
}

#[test]
fn a_later_round_is_estimated_from_its_own_approval() {
    let run = round_two();
    // Counted: n1 (600), n2 (1800) and round 1's unfinished fix task (600): 3000.
    // 300 s since round 2's approval: (300 + 3000) × 1000 / 3000 = 1100.
    assert_eq!(est(&run, APPROVED + 300), (3_000, Some(1_100)));
    // Before round 2 is approved there is no estimate, whatever round 1's approval.
    let mut planning = round_two();
    planning.rounds[1].approved_at = None;
    planning.state = RunState::AwaitingApproval;
    assert_eq!(estimate(&planning, APPROVED + 300), None);
}

/// An open pull request for every stage of `run`, recorded in `pr` mode.
fn delivering(run: &mut Run, wait: u64, wait_from: Option<u64>) {
    run.delivery.mode = DeliveryMode::Pr;
    let pr: PrRecord = serde_json::from_value(json!({
        "number": 7, "url": "https://github.com/fake/app/pull/7", "base": "main",
        "opened_at": APPROVED, "pushed_head": "h", "state": "open"
    }))
    .unwrap();
    run.delivery.stages = vec![StageDelivery {
        pr: Some(pr),
        review_wait_secs: wait,
        wait_from,
        ..StageDelivery::default()
    }];
}

#[test]
fn pr_mode_subtracts_human_review_time() {
    let mut run = three_s();
    let now = APPROVED + 1_000;
    at(task(&mut run, "a"), TaskState::Working, now, 0);
    // 200 s counted and 100 s waiting now: elapsed 700, (700 + 1800) × 1000 / 1800.
    delivering(&mut run, 200, Some(now - 100));
    assert_eq!(est(&run, now), (1_800, Some(1_388)));
    // In local mode the same records count nothing: (1000 + 1800) × 1000 / 1800.
    run.delivery.mode = DeliveryMode::Local;
    assert_eq!(est(&run, now), (1_800, Some(1_555)));
}

#[test]
fn delivering_with_no_task_running_has_no_estimate() {
    let mut run = three_s();
    for id in ["a", "b", "c"] {
        at(task(&mut run, id), TaskState::Merged, APPROVED, 600);
    }
    delivering(&mut run, 0, Some(APPROVED + 10));
    assert_eq!(estimate(&run, APPROVED + 50), None);
    // A fix task working on the stage brings it back.
    at(task(&mut run, "c"), TaskState::Working, APPROVED + 40, 0);
    assert!(estimate(&run, APPROVED + 50).is_some());
}

#[test]
fn ratio_is_none_before_approval() {
    let mut run = three_s();
    run.state = RunState::AwaitingApproval;
    run.approved_at = None;
    assert_eq!(estimate(&run, APPROVED), None);
    let mut state = EngineState::default();
    state.runs.insert(run.id.clone(), run);
    let info = &snapshot(&state, APPROVED).runs[0];
    assert_eq!(
        (info.estimate_left_secs, info.bound_ratio_permille),
        (None, None)
    );
}

#[test]
fn snapshot_fills_estimate_fields() {
    let mut state = EngineState::default();
    let run = three_s();
    state.runs.insert(run.id.clone(), run);
    let info = &snapshot(&state, APPROVED + 1_200).runs[0];
    assert_eq!(info.estimate_left_secs, Some(1_800));
    assert_eq!(info.bound_ratio_permille, Some(1_666));
}
