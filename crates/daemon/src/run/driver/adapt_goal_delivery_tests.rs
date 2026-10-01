//! Milestone 9.2 decision 16, M9.2.3's review fix 1: the daemon's `[delivery]` table
//! reaches a run through the production wiring. `config.toml`'s table goes into the
//! daemon's `RunContext` (`RunContext::from_config`, which `lifecycle` calls), and
//! `build_plan` freezes it into `run.delivery.limits`.

use std::sync::Arc;

use super::super::super::super::RunContext;
use super::super::super::super::RunService;
use super::super::super::super::build::Shape;
use super::{INSTALLED_STAND_IN, NoRoots, git, one_task_plan};
use crate::manager::{ManagerConfig, WindowManager};
use crate::run::delivery::{DeliveryLimits, SyncPolicy};
use crate::run::plan::parse_plan;

/// A daemon config whose `[delivery]` differs from the defaults in every key.
fn loaded_config() -> config::Config {
    config::Config {
        delivery: config::Delivery {
            poll_secs: 5,
            poll_max_secs: 9,
            ci_log_max_bytes: 4096,
            ci_fix_max: 0,
            review_fix_max: 7,
            review_batch_secs: 0,
            reviewers: vec!["alice".into()],
            reply_to_comments: false,
            sync: config::SyncPolicy::Always,
            delete_merged_branches: true,
            stage_target_lines: (100, 200),
        },
        ..config::Config::default()
    }
}

#[tokio::test]
async fn the_daemons_delivery_table_is_frozen_into_a_started_run() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join("repo");
    std::fs::create_dir_all(root.join("crates/a/src")).unwrap();
    std::fs::write(root.join("crates/a/src/lib.rs"), "// a\n").unwrap();
    git(&root, &["init", "-q", "-b", "main"]);
    git(&root, &["config", "user.name", "t"]);
    git(&root, &["config", "user.email", "t@t"]);
    git(&root, &["add", "-A"]);
    git(&root, &["commit", "-q", "-m", "base"]);
    let data = tmp.path().join("data");

    let mut config = ManagerConfig::for_tests("/tmp/ax-unused.sock".into(), "/bin/sh".into());
    config.claude_bin = INSTALLED_STAND_IN.into();
    config.codex_bin = INSTALLED_STAND_IN.into();
    config.cli_caps = crate::headless::argv::CLI_CAPS;
    config.worktrees_root = data.join("worktrees");
    let (manager, _events) = WindowManager::new(config);
    let loaded = loaded_config();
    let ctx = RunContext::from_config(data.clone(), manager.config(), &loaded, Arc::new(NoRoots));
    let service = RunService::new(manager, ctx);

    let plan = parse_plan(&one_task_plan()).unwrap();
    let run = match service
        .build_plan(plan, root, true, false, true, Shape::Fast)
        .await
    {
        Ok(run) => run,
        Err(error) => panic!("{}", error.text()),
    };
    let limits = &run.delivery.limits;
    assert_eq!(*limits, DeliveryLimits::from(&loaded.delivery));
    assert_ne!(*limits, DeliveryLimits::default());
    assert_eq!(limits.ci_fix_max, 0);
    assert_eq!(limits.review_fix_max, 7);
    assert_eq!(limits.sync, SyncPolicy::Always);
    assert_eq!(limits.reviewers, vec!["alice".to_string()]);
    assert_eq!(run.delivery.mode, proto::DeliveryMode::Local);
}
