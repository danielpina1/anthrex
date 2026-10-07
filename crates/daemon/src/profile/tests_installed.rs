//! Milestone 9.5 task 10b (rulings RL-2, I6; follow-up FU-F10), milestone 9.8: the
//! onboarding scout and the deciders take their rows over what is installed. A service wired as the daemon wires
//! it, on a scratch data directory and a real git repository; Claude's binary is a
//! path that does not exist and Codex's a stand-in the probe only stats. No scout or
//! decider is started.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use proto::Runtime;
use proto::models::{ModelRef, ModelTable, Role, RoleChoice};

use super::service::ProfileService;
use super::tests_ready::repo;
use crate::decider::DeciderContext;
use crate::decider::DeciderKind::Triage;
use crate::decider::call::routed;
use crate::launch::LaunchGate;
use crate::manager::{ManagerConfig, WindowManager};
use crate::run::driver::{RunContext, RunService};
use crate::server::GitWiring;

const NO_CLAUDE: &str = "/nonexistent/anthrex-test/claude";

/// A stand-in `codex` the probe finds executable; nothing runs it.
fn codex_stand_in(dir: &Path) -> PathBuf {
    testexec::write_executable(dir.join("codex"), "#!/bin/sh\nexit 0\n")
}

/// A profile service and the daemon's decider context, both wired over `config` (the
/// decider context over the run service's live settings, as `profile::service::wire`
/// builds it).
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
    let deciders = DeciderContext::new(runs.live_settings().clone(), manager.config(), &data);
    let profiles = crate::profile::service::wire(&manager, &runs, &data, &socket, &config);
    (profiles, deciders)
}

fn row(model: &str, fallback: Option<&str>) -> RoleChoice {
    RoleChoice {
        model: ModelRef::parse(model).unwrap(),
        effort: None,
        fallback: fallback.map(|f| ModelRef::parse(f).unwrap()),
    }
}

/// Milestone 9.8 (D2): with only Codex installed, the onboarding scout and a decider
/// stay on their Claude rows unless the row falls back to Codex; the repository's
/// `models.toml` comes first for the onboarding scout.
#[tokio::test]
async fn onboarding_scouts_and_deciders_use_their_rows_over_what_is_installed() {
    let dir = tempfile::tempdir().unwrap();
    let project = repo(dir.path(), "app");
    let pre = crate::run::git::preflight("git".as_ref(), &project, Duration::from_secs(10))
        .expect("preflight");
    let codex = codex_stand_in(dir.path());
    let (profiles, deciders) = wired(dir.path(), &codex, config::Orchestrator::default());
    // The built-in rows name no fallback: Claude, though it is not installed.
    let (_, _, route) = profiles.scout_checks(&pre, true).await.expect("checked");
    assert_eq!(route.runtime, Runtime::Claude);
    let decider = routed(&deciders, Triage, Some(&pre.project)).await.ctx;
    assert_eq!(decider.route.runtime, Runtime::Claude);

    // Rows falling back to Codex: both move to their fallback.
    let mut config = config::Orchestrator::default();
    let haiku = "claude:claude-haiku-4-5";
    config
        .roles
        .rows
        .insert(Role::Research, row(haiku, Some("codex:default")));
    config
        .roles
        .rows
        .insert(Role::Helpers, row(haiku, Some("codex:gpt-6-luna")));
    let other = tempfile::tempdir().unwrap();
    let (profiles, deciders) = wired(other.path(), &codex, config);
    let (_, _, route) = profiles.scout_checks(&pre, true).await.expect("checked");
    assert_eq!((route.runtime, route.model.as_str()), (Runtime::Codex, ""));
    let decider = routed(&deciders, Triage, None).await.ctx;
    assert_eq!(decider.route.model, "gpt-6-luna");

    // The repository's file names the research row: it comes first.
    let mut table = ModelTable::default();
    table
        .rows
        .insert(Role::Research, row("codex:gpt-6-luna", None));
    let file = crate::profile::repo_dir(&other.path().join("data"), &pre.project)
        .join(config::models::REPO_FILE);
    std::fs::create_dir_all(file.parent().unwrap()).unwrap();
    config::models::save_repo(&file, &table).unwrap();
    let (_, _, route) = profiles.scout_checks(&pre, true).await.expect("checked");
    assert_eq!(
        (route.runtime, route.model.as_str()),
        (Runtime::Codex, "gpt-6-luna")
    );
}
