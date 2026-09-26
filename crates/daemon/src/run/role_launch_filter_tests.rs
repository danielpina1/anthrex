//! Milestone 8b decision 28: which sessions get the output-filter hook, and where its
//! log goes.

use super::*;
use crate::output_filter::{FilterHook, LOG_DIR_NAME};
use crate::run::test_support::{PROFILE, plan_with, run_ok, task_toml};
use proto::{Effort, OutputFilter, ProfileSource, Strength};

fn stored_run() -> Run {
    let mut run = run_ok(&plan_with(
        PROFILE,
        &[task_toml("t1", "S", "[\"crates/a/**\"]", "")],
    ));
    run.profile_source = Some(ProfileSource::Stored);
    run.output_filter = OutputFilter::FailuresOnly;
    run.filter_prefixes = vec!["cargo test".into(), "cargo build".into()];
    run
}

fn route(runtime: Runtime) -> Route {
    Route {
        runtime,
        model: "m".into(),
        strength: Strength::Standard,
        effort: Effort::Medium,
    }
}

#[test]
fn worker_spec_sets_the_hook_only_for_a_stored_profile_with_prefixes() {
    let run = stored_run();
    let mut task = run.tasks[0].clone();
    task.route = route(Runtime::Claude);
    assert_eq!(
        worker_spec(&run, &task).output_filter,
        Some(FilterHook {
            mode: OutputFilter::FailuresOnly,
            prefixes: vec!["cargo test".into(), "cargo build".into()],
            log_dir: task_tmp_dir(&run.data_dir, "t1").join(LOG_DIR_NAME),
        })
    );
    let mut tail = run.clone();
    tail.output_filter = OutputFilter::Tail;
    let hook = worker_spec(&tail, &task)
        .output_filter
        .expect("tail filters");
    assert_eq!(hook.mode, OutputFilter::Tail);

    let without = |change: &dyn Fn(&mut Run)| {
        let mut run = run.clone();
        change(&mut run);
        worker_spec(&run, &task).output_filter
    };
    assert_eq!(without(&|r| r.profile_source = None), None);
    assert_eq!(
        without(&|r| r.profile_source = Some(ProfileSource::Plan)),
        None
    );
    assert_eq!(
        without(&|r| r.profile_source = Some(ProfileSource::None)),
        None
    );
    assert_eq!(without(&|r| r.output_filter = OutputFilter::None), None);
    assert_eq!(without(&|r| r.filter_prefixes.clear()), None);

    // Codex workers and every reviewer never get it.
    let mut codex = task.clone();
    codex.route = route(Runtime::Codex);
    assert_eq!(worker_spec(&run, &codex).output_filter, None);
    for runtime in [Runtime::Claude, Runtime::Codex] {
        assert_eq!(
            reviewer_spec(&run, &task, &route(runtime)).output_filter,
            None
        );
    }
}

#[test]
fn the_filter_hook_adds_no_writable_root() {
    let run = stored_run();
    let mut task = run.tasks[0].clone();
    task.route = route(Runtime::Claude);
    let spec = worker_spec(&run, &task);
    let hook = spec.output_filter.as_ref().expect("filtered");
    let sandbox = spec.claude_sandbox.as_ref().expect("sandboxed");
    assert_eq!(
        sandbox.writable_roots,
        worker_git_roots(&run.data_dir, "t1")
    );
    let mut plain = run.clone();
    plain.profile_source = None;
    assert_eq!(
        worker_spec(&plain, &task).claude_sandbox.as_ref(),
        Some(sandbox)
    );
    // The log goes under the task's `TMPDIR`, which is already in the grant, and out of
    // the checkout.
    let tmpdir = spec
        .env
        .iter()
        .find(|(key, _)| key == "TMPDIR")
        .map(|(_, value)| PathBuf::from(value))
        .expect("TMPDIR");
    assert_eq!(hook.log_dir, tmpdir.join(LOG_DIR_NAME));
    assert!(sandbox.writable_roots.contains(&tmpdir));
    assert!(!hook.log_dir.starts_with(&task.worktree));
}
