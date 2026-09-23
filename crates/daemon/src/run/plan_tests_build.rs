//! Run building: limits from config, `--yes`, revision and `unverified`, and glob
//! validity (M8a.5 fix round 1).

use std::path::PathBuf;

use proto::RunState;

use super::super::*;
use crate::run::test_support::*;

#[test]
fn unset_plan_limits_come_from_config() {
    // Config values pairwise distinct and unlike the defaults (3, 3, 2).
    let config = config::Orchestrator {
        max_writers: 5,
        max_readers: 6,
        max_bounces: 4,
        ..config::Orchestrator::default()
    };
    let text = plan_with(
        PROFILE,
        &[task_toml("t1", "S", r#"["crates/a/src/lib.rs"]"#, "")],
    );
    let run = build_with(&text, &config).unwrap_or_else(|e| panic!("{}", show(&e)));
    assert_eq!(
        (
            run.limits.max_writers,
            run.limits.max_readers,
            run.limits.max_bounces
        ),
        (5, 6, 4)
    );
}

#[test]
fn yes_records_the_approval() {
    let plan = parse_plan(EXAMPLE_PLAN).unwrap();
    let config = config::Orchestrator::default();
    let run = build_run(
        plan,
        preflight(),
        BuildContext {
            id: RUN_ID.to_string(),
            wt_dir: PathBuf::from("/tmp/wt"),
            data_dir: PathBuf::from(format!("/tmp/data/runs/{RUN_ID}")),
            config: &config,
            now: 1_000,
            yes: true,
        },
    )
    .unwrap_or_else(|e| panic!("{}", show(&e)));
    assert_eq!(run.approved_by.as_deref(), Some("--yes"));
    assert_eq!(run.state, RunState::AwaitingApproval);
}

/// Decision 47: revisions start at 1. Decision 34: no `check` marks the run unverified.
#[test]
fn a_new_run_starts_at_revision_one_and_is_unverified_without_check() {
    let run = run_ok(EXAMPLE_PLAN);
    assert_eq!(run.revision, 1);
    assert!(!run.unverified);

    let no_check = plan_with(
        &PROFILE.replace("check = \"cargo test\"\n", ""),
        &[task_toml("t1", "S", r#"["crates/a/src/lib.rs"]"#, "")],
    );
    let run = run_ok(&no_check);
    assert_eq!(run.profile.check, None);
    assert!(run.unverified);
}

#[test]
fn globs_that_do_not_compile_are_rejected() {
    let text = plan_with(
        &format!("{PROFILE}generated = [\"a/[x\"]\nprotected = [\"b/[y\"]\n"),
        &[task_toml("t1", "S", r#"["crates/a/src/[x.rs"]"#, "")],
    );
    let bad = "is not a valid glob: unclosed character class; missing ']'";
    assert_eq!(
        errors_of(&text),
        vec![
            err(None, "profile.generated", "globs", &format!("a/[x {bad}")),
            err(None, "profile.protected", "globs", &format!("b/[y {bad}")),
            err(
                Some("t1"),
                "owns",
                "globs",
                &format!("crates/a/src/[x.rs {bad}")
            ),
        ]
    );
}

#[test]
fn dot_components_in_owns_are_rejected() {
    // Without the rejection, `./crates/…` would dodge rule 9 against `crates/…`.
    let text = plan_with(
        PROFILE,
        &[
            task_toml(
                "t1",
                "S",
                r#"["./crates/a/src/x.rs"]"#,
                "[task.route]\nruntime = \"claude\"",
            ),
            task_toml(
                "t2",
                "S",
                r#"["crates//b/src/y.rs"]"#,
                "[task.route]\nruntime = \"codex\"",
            ),
        ],
    );
    assert_eq!(
        errors_of(&text),
        vec![
            err(
                Some("t1"),
                "owns",
                "globs",
                "./crates/a/src/x.rs must not contain ."
            ),
            err(
                Some("t2"),
                "owns",
                "globs",
                "crates//b/src/y.rs must not contain //"
            ),
        ]
    );
}
