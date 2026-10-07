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
            tuning: Default::default(),
            id: RUN_ID.to_string(),
            wt_dir: PathBuf::from("/tmp/wt"),
            data_dir: PathBuf::from(format!("/tmp/data/runs/{RUN_ID}")),
            config: &config,
            models: crate::run::model_roles::RunModels::resolve(&config.roles, None),
            models_log: Vec::new(),
            testing: &config::Testing::default(),
            now: 1_000,
            yes: true,
            delivery: &config::Delivery::default(),
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
    // Milestone 9.8 decision 31: the two runtimes come from the role table (size M on
    // Codex), not from the plan's routes.
    let text = plan_with(
        PROFILE,
        &[
            task_toml("t1", "S", r#"["./crates/a/src/x.rs"]"#, ""),
            task_toml("t2", "M", r#"["crates//b/src/y.rs"]"#, ""),
        ],
    );
    assert_eq!(
        build_with(&text, &codex_medium()).unwrap_err(),
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

/// M8a.8's carry (minor 9, RR3): a run id is taken by `refs/heads/anthrex/<id>` itself
/// (git could not create `anthrex/<id>/integration` beside it) and by any branch under
/// `anthrex/<id>/`; a longer id sharing the prefix is not.
#[test]
fn a_run_id_is_taken_by_its_own_branch_or_anything_under_it() {
    let refs = |names: &[&str]| names.iter().map(|n| n.to_string()).collect::<Vec<_>>();
    let id = "add-reset-3f9a";
    assert!(!run_id_taken(id, &[]));
    assert!(run_id_taken(
        id,
        &refs(&["refs/heads/anthrex/add-reset-3f9a"])
    ));
    assert!(run_id_taken(
        id,
        &refs(&["refs/heads/anthrex/add-reset-3f9a/integration"])
    ));
    assert!(run_id_taken(
        id,
        &refs(&["refs/heads/anthrex/add-reset-3f9a/x/y"])
    ));
    assert!(!run_id_taken(
        id,
        &refs(&[
            "refs/heads/anthrex/add-reset-3f9ab",
            "refs/heads/anthrex/add-reset-3f9a-2/t1",
            "refs/heads/add-reset-3f9a",
            "refs/anthrex/salvage/add-reset-3f9a/t1/1",
        ])
    ));
}

/// Milestone 9.1 decision 3: `[testing]`'s run rules are frozen into the run at start
/// (`RunLimits.testing`), so a later config change cannot move a live run's.
#[test]
fn run_limits_freeze_the_testing_limits() {
    use crate::run::journal::{load_all, save_run};
    use crate::run::model::TestingLimits;

    let testing = config::Testing {
        test_slots: Some(3),
        full_idle_secs: 300,
        test_cache_days: 7,
        flaky_quarantine_after: 5,
        flaky_window_days: 30,
        bisect_fix_max: 4,
    };
    let frozen = TestingLimits {
        full_idle_secs: 300,
        bisect_fix_max: 4,
        flaky_quarantine_after: 5,
        flaky_window_days: 30,
    };
    let data = tempfile::tempdir().unwrap();
    let config = config::Orchestrator::default();
    let run = build_run(
        parse_plan(EXAMPLE_PLAN).unwrap(),
        preflight(),
        BuildContext {
            tuning: Default::default(),
            id: RUN_ID.to_string(),
            wt_dir: PathBuf::from("/tmp/wt"),
            data_dir: crate::run::journal::runs_dir(data.path()).join(RUN_ID),
            config: &config,
            models: crate::run::model_roles::RunModels::resolve(&config.roles, None),
            models_log: Vec::new(),
            testing: &testing,
            now: 1_000,
            yes: false,
            delivery: &config::Delivery::default(),
        },
    )
    .unwrap_or_else(|e| panic!("{}", show(&e)));
    assert_eq!(run.limits.testing, frozen);

    // A default `[testing]` freezes the defaults of Interfaces "config".
    let defaults = TestingLimits {
        full_idle_secs: 120,
        bisect_fix_max: 2,
        flaky_quarantine_after: 3,
        flaky_window_days: 14,
    };
    assert_eq!(TestingLimits::default(), defaults);
    assert_eq!(run_ok(EXAMPLE_PLAN).limits.testing, defaults);

    // A restored `run.json` keeps its own: restoring reads no config at all, so a
    // `[testing]` edited after the start never reaches the run.
    save_run(&run).unwrap();
    let (runs, problems) = load_all(data.path());
    assert!(problems.is_empty(), "{problems:?}");
    assert_eq!(runs.len(), 1);
    assert_eq!(runs[0].0.limits.testing, frozen);

    // An M9 `run.json`, written before the field existed, loads with the defaults.
    let mut old = serde_json::to_value(&run).unwrap();
    let limits = old["limits"].as_object_mut().unwrap();
    assert!(limits.remove("testing").is_some());
    let old: crate::run::model::Run = serde_json::from_value(old).unwrap();
    assert_eq!(old.limits.testing, defaults);
}

/// Milestone 9.2 decision 16: `[delivery]` is frozen into the run at start
/// (`RunDelivery.limits`); a restored run keeps its own, so a config edited after the
/// start never reaches it, and a 9.1 `run.json` loads with `local` and the defaults.
#[test]
fn limits_are_frozen_at_run_start() {
    use crate::run::delivery::{DeliveryLimits, SyncPolicy};
    use crate::run::journal::{load_all, save_run};

    let delivery = config::Delivery {
        poll_secs: 5,
        poll_max_secs: 9,
        ci_log_max_bytes: 8192,
        ci_fix_max: 1,
        review_fix_max: 4,
        review_batch_secs: 0,
        reviewers: vec!["alice".into()],
        reply_to_comments: false,
        sync: config::SyncPolicy::Always,
        delete_merged_branches: true,
        stage_target_lines: (100, 400),
    };
    let frozen = DeliveryLimits {
        poll_secs: 5,
        poll_max_secs: 9,
        ci_log_max_bytes: 8192,
        ci_fix_max: 1,
        review_fix_max: 4,
        review_batch_secs: 0,
        reviewers: vec!["alice".into()],
        reply_to_comments: false,
        sync: SyncPolicy::Always,
        delete_merged_branches: true,
        stage_target_lines: (100, 400),
    };
    let data = tempfile::tempdir().unwrap();
    let config = config::Orchestrator::default();
    let testing = config::Testing::default();
    let run = build_run(
        parse_plan(EXAMPLE_PLAN).unwrap(),
        preflight(),
        BuildContext {
            tuning: Default::default(),
            id: RUN_ID.to_string(),
            wt_dir: PathBuf::from("/tmp/wt"),
            data_dir: crate::run::journal::runs_dir(data.path()).join(RUN_ID),
            config: &config,
            models: crate::run::model_roles::RunModels::resolve(&config.roles, None),
            models_log: Vec::new(),
            testing: &testing,
            delivery: &delivery,
            now: 1_000,
            yes: false,
        },
    )
    .unwrap_or_else(|e| panic!("{}", show(&e)));
    assert_eq!(run.delivery.limits, frozen);
    assert_eq!(run.delivery.mode, proto::DeliveryMode::Local);

    // A default `[delivery]` freezes Interfaces "config"'s defaults.
    let defaults = DeliveryLimits {
        poll_secs: 60,
        poll_max_secs: 300,
        ci_log_max_bytes: 2_097_152,
        ci_fix_max: 2,
        review_fix_max: 3,
        review_batch_secs: 120,
        reviewers: Vec::new(),
        reply_to_comments: true,
        sync: SyncPolicy::OnConflict,
        delete_merged_branches: false,
        stage_target_lines: (300, 800),
    };
    assert_eq!(DeliveryLimits::default(), defaults);
    assert_eq!(run_ok(EXAMPLE_PLAN).delivery.limits, defaults);

    // A restored `run.json` keeps its own: restoring reads no config.
    save_run(&run).unwrap();
    let (runs, problems) = load_all(data.path());
    assert!(problems.is_empty(), "{problems:?}");
    assert_eq!(runs[0].0.delivery.limits, frozen);

    // A 9.1 `run.json`, written before the field existed, loads local with the defaults.
    let mut old = serde_json::to_value(&run).unwrap();
    assert!(old.as_object_mut().unwrap().remove("delivery").is_some());
    let old: crate::run::model::Run = serde_json::from_value(old).unwrap();
    assert_eq!(old.delivery.mode, proto::DeliveryMode::Local);
    assert_eq!(old.delivery.limits, defaults);
}
