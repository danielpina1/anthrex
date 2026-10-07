//! Milestone 9.5 task 10b (rulings RL-2, I6; follow-up FU-F10): the onboarding scout
//! and the deciders route over what is installed. A service wired as the daemon wires
//! it, on a scratch data directory and a real git repository; Claude's binary is a
//! path that does not exist and Codex's a stand-in the probe only stats. No scout or
//! decider is started.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use proto::Runtime;

use super::service::ProfileService;
use super::tests_ready::repo;
use crate::decider::DeciderContext;
use crate::launch::LaunchGate;
use crate::manager::{ManagerConfig, WindowManager};
use crate::run::driver::{RunContext, RunService};
use crate::server::GitWiring;

const NO_CLAUDE: &str = "/nonexistent/anthrex-test/claude";

/// A stand-in `codex` the probe finds executable; nothing runs it.
fn codex_stand_in(dir: &Path) -> PathBuf {
    testexec::write_executable(dir.join("codex"), "#!/bin/sh\nexit 0\n")
}

/// A profile service and the daemon's decider context, both wired over `config`.
fn wired(
    dir: &Path,
    codex: &Path,
    config: config::Orchestrator,
) -> (Arc<ProfileService>, DeciderContext) {
    let socket = dir.join("d.sock");
    let data = dir.join("data");
    let mut manager = ManagerConfig::for_tests(socket.clone(), "/bin/sh".into());
    manager.claude_bin = NO_CLAUDE.into();
    manager.codex_bin = codex.display().to_string();
    manager.worktrees_root = dir.join("worktrees");
    manager.launch_gate = LaunchGate::open_already();
    let deciders = DeciderContext::new(&config, &manager, &data);
    let (manager, _events) = WindowManager::new(manager);
    let git = GitWiring::new(config::Git {
        enabled: false,
        ..config::Git::default()
    });
    let ctx = RunContext::new(
        data.clone(),
        manager.config(),
        config.clone(),
        git.registry.clone(),
    );
    let runs = RunService::new(manager.clone(), ctx);
    let profiles = crate::profile::service::wire(&manager, &runs, &data, &socket, &config);
    (profiles, deciders)
}

#[tokio::test]
async fn onboarding_scouts_and_deciders_use_what_is_installed() {
    let dir = tempfile::tempdir().unwrap();
    let project = repo(dir.path(), "app");
    let pre = crate::run::git::preflight("git".as_ref(), &project, Duration::from_secs(10))
        .expect("preflight");
    let codex = codex_stand_in(dir.path());
    let (profiles, deciders) = wired(dir.path(), &codex, config::Orchestrator::default());
    assert_eq!(
        deciders.route.runtime,
        Runtime::Claude,
        "the default mode's"
    );

    // Only Codex installed: a Codex onboarding scout and a Codex (triage) decider.
    let (_, _, route) = profiles.scout_checks(&pre, true).await.expect("checked");
    assert_eq!(route.runtime, Runtime::Codex);
    let decider = crate::decider::call::routed(&deciders).await.ctx;
    assert_eq!(decider.route.runtime, Runtime::Codex);

    // Nothing installed: both resolve exactly as before.
    std::fs::remove_file(&codex).unwrap();
    let ctx = profiles.scouts.context();
    let today = crate::scout::spec::scout_route(ctx);
    let (_, _, route) = profiles.scout_checks(&pre, true).await.expect("checked");
    assert_eq!(route, today);
    assert_eq!(route.runtime, Runtime::Claude);
    let decider = crate::decider::call::routed(&deciders).await.ctx;
    assert_eq!(decider.route, deciders.route);

    // A `scout` list: its first candidate installed.
    let codex = codex_stand_in(dir.path());
    let mut config = config::Orchestrator::default();
    config.tuning.routes.scout = config::RouteList {
        candidates: vec![
            config::Candidate {
                runtime: Runtime::Claude,
                model: "claude-haiku-4-5".into(),
                effort: None,
            },
            config::Candidate {
                runtime: Runtime::Codex,
                model: String::new(),
                effort: None,
            },
        ],
        pick: config::Pick::First,
    };
    let other = tempfile::tempdir().unwrap();
    let (profiles, _) = wired(other.path(), &codex, config);
    let (_, _, route) = profiles.scout_checks(&pre, true).await.expect("checked");
    assert_eq!((route.runtime, route.model.as_str()), (Runtime::Codex, ""));
}
