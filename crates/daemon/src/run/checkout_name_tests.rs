//! Task M9.5.15, part 1 (ruling RR-1): a task's checkout is named by
//! `Task::checkout_name`, the task id until a lane of its race is crowned or adopted,
//! and every launch-side path keyed by a checkout name follows it. The engine's sites
//! (proof, check, review, merge and run clean-up) are `engine/tests/checkout_name.rs`'s.

use proto::{LaneState, OutputFilter, ProfileSource};

use crate::output_filter::LOG_DIR_NAME;
use crate::run::model::Run;
use crate::run::role_launch::{
    reviewer_spec, task_engine_dir, task_objects_dir, task_repo_dir, task_tmp_dir,
    worker_git_roots, worker_spec,
};
use crate::run::test_support::{PROFILE, plan_with, race_of, run_ok, task_toml};

fn run() -> Run {
    let mut run = run_ok(&plan_with(
        PROFILE,
        &[task_toml("t1", "S", "[\"crates/a/**\"]", "")],
    ));
    run.profile_source = Some(ProfileSource::Stored);
    run.output_filter = OutputFilter::FailuresOnly;
    run.filter_prefixes = vec!["cargo test".into()];
    run
}

#[test]
fn checkout_name_is_the_task_until_a_lane_is_crowned() {
    let run = run();
    let mut task = run.tasks[0].clone();
    assert_eq!(task.checkout_name(), "t1", "no race");
    task.race = Some(race_of(&task, [LaneState::Working, LaneState::Review]));
    assert_eq!(task.checkout_name(), "t1", "racing, nothing crowned");
    task.race = Some(race_of(&task, [LaneState::Lost, LaneState::Won]));
    assert_eq!(task.checkout_name(), "t1.b", "lane b won");
    task.race = Some(race_of(&task, [LaneState::Adopted, LaneState::Out]));
    assert_eq!(task.checkout_name(), "t1.a", "lane a adopted");
}

/// The crowned task, as the reducer leaves it (decision 21; ruling T1-3): lane b won,
/// and `task.worktree` is the lane's checkout.
fn crowned(run: &Run) -> crate::run::model::Task {
    let mut task = run.tasks[0].clone();
    task.race = Some(race_of(&task, [LaneState::Lost, LaneState::Won]));
    task.worktree = run.task_path(&task.checkout_name());
    task
}

#[test]
fn every_launch_site_follows_checkout_name() {
    let run = run();
    let task = crowned(&run);
    let data = &run.data_dir;
    assert_eq!(run.task_path(&task.checkout_name()), run.task_path("t1.b"));

    let spec = worker_spec(&run, &task);
    assert_eq!(spec.cwd, run.task_path("t1.b"));
    let tmpdir = spec
        .env
        .iter()
        .find(|(key, _)| key == "TMPDIR")
        .map(|(_, value)| value.clone());
    assert_eq!(
        tmpdir,
        Some(task_tmp_dir(data, "t1.b").display().to_string())
    );
    assert_eq!(
        spec.output_filter.map(|hook| hook.log_dir),
        Some(task_tmp_dir(data, "t1.b").join(LOG_DIR_NAME))
    );
    let roots = spec.claude_sandbox.map(|s| s.writable_roots);
    assert_eq!(roots, Some(worker_git_roots(data, "t1.b")));
    assert_eq!(
        worker_git_roots(data, "t1.b"),
        vec![task_objects_dir(data, "t1.b"), task_tmp_dir(data, "t1.b")]
    );
    assert_eq!(task_repo_dir(data, "t1.b"), data.join("tasks/t1.b"));
    assert_eq!(
        task_engine_dir(data, "t1.b"),
        data.join("tasks/t1.b/engine")
    );

    let route = task.route.clone();
    assert_eq!(
        reviewer_spec(&run, &task, &route).cwd,
        run.review_path("t1.b")
    );
}

/// Pinning: an unraced task's paths are the task id's, as before.
#[test]
fn an_unraced_task_keeps_its_own_paths() {
    let run = run();
    let task = run.tasks[0].clone();
    let data = &run.data_dir;
    let spec = worker_spec(&run, &task);
    assert_eq!(spec.cwd, run.task_path("t1"));
    assert_eq!(
        spec.output_filter.map(|hook| hook.log_dir),
        Some(task_tmp_dir(data, "t1").join(LOG_DIR_NAME))
    );
    assert_eq!(
        spec.claude_sandbox.map(|s| s.writable_roots),
        Some(worker_git_roots(data, "t1"))
    );
    let route = task.route.clone();
    assert_eq!(
        reviewer_spec(&run, &task, &route).cwd,
        run.review_path("t1")
    );
}
