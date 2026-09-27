//! M8b.14 review fixes: `run start --goal`'s refusals in the driver. A blank goal is
//! refused before anything else (m2), and `build_plan`'s fast-path barrier runs before
//! the runtime checks, so a hub task reports the hub reason (m1).

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;

use proto::RunReply;
use proto::run_wire::request;

use super::super::super::RunService;
use super::BuildError;
use crate::manager::{GitRoots, ManagerConfig, WindowManager};
use crate::run::plan::parse_plan;

struct NoRoots;
impl GitRoots for NoRoots {
    fn register(&self, _: PathBuf) {}
    fn unregister(&self, _: &Path) {}
}

/// A service over `data` whose runtimes are nonexistent paths, and whose Claude caps
/// check a repository's project settings (as `ANTHREX_TEST_NO_SETTING_SOURCES` does).
fn service(data: &Path) -> Arc<RunService> {
    let mut config = ManagerConfig::new("/tmp/ax-unused.sock".into(), "/bin/sh".into());
    config.claude_bin = "/nonexistent/ax-claude".into();
    config.codex_bin = "/nonexistent/ax-codex".into();
    config.cli_caps.claude_user_settings_only = None;
    config.worktrees_root = data.join("worktrees");
    let (manager, _events) = WindowManager::new(config);
    RunService::for_manager(&manager, data.to_path_buf(), Arc::new(NoRoots))
}

fn git(dir: &Path, args: &[&str]) {
    let output = Command::new("git")
        .arg("--no-optional-locks")
        .args([
            "-c",
            "user.name=t",
            "-c",
            "user.email=t@t",
            "-c",
            "commit.gpgsign=false",
        ])
        .args(args)
        .current_dir(dir)
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env_remove("GIT_DIR")
        .env_remove("GIT_WORK_TREE")
        .env_remove("GIT_COMMON_DIR")
        .env_remove("GIT_INDEX_FILE")
        .env_remove("GIT_PREFIX")
        .output()
        .unwrap();
    assert!(output.status.success(), "git {args:?}: {output:?}");
}

/// A repository whose base commit tracks `.mcp.json`: untrusted project settings.
fn repo_with_project_settings(root: &Path) {
    std::fs::create_dir_all(root.join("crates/proto/src")).unwrap();
    std::fs::write(root.join("crates/proto/src/wire.rs"), "// wire\n").unwrap();
    std::fs::write(root.join(".mcp.json"), "{\"mcpServers\": {}}\n").unwrap();
    git(root, &["init", "-q", "-b", "main"]);
    // Preflight reads the repository's own identity (CI has no global one).
    git(root, &["config", "user.name", "t"]);
    git(root, &["config", "user.email", "t@t"]);
    git(root, &["add", "-A"]);
    git(root, &["commit", "-q", "-m", "base"]);
}

/// The fast path's one-task plan, owning a hub file, routed to Claude.
const HUB_PLAN: &str = r#"
goal = "Rename a wire field"

[profile]
modules = ["crates/*"]
hub = ["crates/proto/**"]
source = ["crates/*/src/**"]
check = "true"
single_test = "true {test}"
test_passed = "ok {test}"

[[task]]
id = "t1"
title = "Rename the field"
kind = "code"
size = "M"
test_mode = "check"
test_mode_reason = "a rename"
owns = ["crates/proto/src/wire.rs"]
brief = "Rename it."
acceptance = ["renamed"]
[task.route]
runtime = "claude"
model = "claude-sonnet-5"
strength = "standard"
effort = "medium"
"#;

#[tokio::test]
async fn a_blank_goal_is_refused_before_anything_else() {
    let tmp = tempfile::tempdir().unwrap();
    let service = service(&tmp.path().join("data"));
    // No profile service, and a directory that is not a repository: a refusal from
    // either would mean the goal was not checked first, before any triage call.
    for goal in ["", "   \n\t"] {
        let reply = service
            .start_goal(goal.into(), tmp.path().join("nowhere"), false, false)
            .await;
        let RunReply::Refused {
            request: r,
            message,
        } = reply
        else {
            panic!("not refused: {reply:?}");
        };
        assert_eq!(r, request::START_GOAL);
        assert_eq!(message, "goal: must not be blank", "{goal:?}");
    }
}

#[tokio::test]
async fn a_hub_goal_reports_the_hub_reason_before_the_runtime_checks() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join("repo");
    repo_with_project_settings(&root);
    let service = service(&tmp.path().join("data"));
    let plan = || parse_plan(HUB_PLAN).unwrap();

    // The fast path: the hub reason, not the project-settings refusal.
    let fast = service
        .build_plan(plan(), root.clone(), true, false, true, true)
        .await;
    match fast {
        Err(BuildError::NotFast(reason)) => assert_eq!(
            reason,
            "the fast path does not apply: task t1 touches a hub file"
        ),
        Err(other) => panic!("not the hub reason: {}", other.text()),
        Ok(_) => panic!("a hub task built for the fast path"),
    }
    // The control: `run start --plan`'s path reaches the runtime checks with the same
    // plan and refuses with the settings text.
    let planned = service
        .build_plan(plan(), root.clone(), true, false, true, false)
        .await;
    let Err(BuildError::Refused(text)) = planned else {
        panic!("the settings check did not refuse");
    };
    assert!(text.contains(".mcp.json"), "{text}");
    // Neither built a run directory.
    let runs = tmp.path().join("data").join("runs");
    assert!(!runs.exists() || std::fs::read_dir(&runs).unwrap().next().is_none());
}
