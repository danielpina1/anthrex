//! M8b.14 review fixes: `run start --goal`'s refusals in the driver. A blank goal is
//! refused before anything else (m2), and `build_plan`'s fast-path barrier runs before
//! the runtime checks, so a hub task reports the hub reason (m1).

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;

use proto::RunReply;
use proto::run_wire::request;

use super::super::super::RunService;
use super::super::super::build::Shape;
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
            .start_goal(
                goal.into(),
                tmp.path().join("nowhere"),
                (false, false),
                false,
                None,
            )
            .await;
        let RunReply::Refused {
            request: r,
            message,
            ..
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
        .build_plan(plan(), root.clone(), true, false, true, Shape::Fast)
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
        .build_plan(plan(), root.clone(), true, false, true, Shape::PlanFile)
        .await;
    let Err(BuildError::Refused(text)) = planned else {
        panic!("the settings check did not refuse");
    };
    assert!(text.contains(".mcp.json"), "{text}");
    // Neither built a run directory.
    let runs = tmp.path().join("data").join("runs");
    assert!(!runs.exists() || std::fs::read_dir(&runs).unwrap().next().is_none());
}

/// M9.3 review: the configured `[orchestrator] planner_task_cap` reaches the triage
/// decider's input, so a non-default cap renders in the triage prompt.
#[test]
fn the_configured_planner_cap_reaches_triage() {
    let mut orchestrator = config::Orchestrator::default();
    orchestrator.agent.planner_task_cap = 7;
    let input = super::triage_input("Add a flag", &proto::RepoProfile::default(), &orchestrator);
    assert_eq!(input.planner_task_cap, 7);
    assert_eq!(input.goal, "Add a flag");
}

/// A repository whose base commit tracks `.codex/config.toml`, which a Codex session
/// loads unasked (`CLI_CAPS.codex_user_config_only` is `None`).
fn repo_with_codex_config(root: &Path) {
    std::fs::create_dir_all(root.join(".codex")).unwrap();
    std::fs::write(root.join(".codex/config.toml"), "model = \"x\"\n").unwrap();
    std::fs::create_dir_all(root.join("crates/a/src")).unwrap();
    std::fs::write(root.join("crates/a/src/lib.rs"), "// a\n").unwrap();
    git(root, &["init", "-q", "-b", "main"]);
    git(root, &["config", "user.name", "t"]);
    git(root, &["config", "user.email", "t@t"]);
    git(root, &["add", "-A"]);
    git(root, &["commit", "-q", "-m", "base"]);
}

/// A service whose caps are the shipped ones (Claude excludes project settings; Codex
/// cannot).
fn shipped_service(data: &Path) -> Arc<RunService> {
    let mut config = ManagerConfig::new("/tmp/ax-unused.sock".into(), "/bin/sh".into());
    config.claude_bin = "/nonexistent/ax-claude".into();
    config.codex_bin = "/nonexistent/ax-codex".into();
    config.cli_caps = crate::headless::argv::CLI_CAPS;
    config.worktrees_root = data.join("worktrees");
    let (manager, _events) = WindowManager::new(config);
    RunService::for_manager(&manager, data.to_path_buf(), Arc::new(NoRoots))
}

/// A one-task plan in `crates/a`, routed to Claude.
fn one_task_plan() -> String {
    HUB_PLAN
        .replace("hub = [\"crates/proto/**\"]\n", "")
        .replace("crates/proto/src/wire.rs", "crates/a/src/lib.rs")
}

fn planned(runtime: proto::Runtime) -> Shape {
    Shape::Planned(Box::new(super::super::super::build::Planned {
        triage: proto::TriageInfo {
            kinds: vec![proto::TaskKind::Code],
            scale: proto::Scale::Plan,
            path: proto::RunPath::Plan,
            reason: "several modules".into(),
            source: proto::DeciderSource::Decider,
            fallback_reason: None,
            at: 1,
        },
        usage: None,
        yes: false,
        choice: Some(proto::OrchestratorChoice {
            runtime,
            model: None,
        }),
    }))
}

/// Decisions 9 and 26: the planned run's start checks cover its orchestrator's runtime
/// (and its sub-planners', which default to it). A Codex orchestrator in a repository
/// with tracked `.codex` settings is refused without `--trust-project`, and starts with
/// it, the files recorded; a Claude one starts, since Claude excludes project settings.
#[tokio::test]
async fn project_settings_check_covers_the_orchestrator() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join("repo");
    repo_with_codex_config(&root);
    let service = shipped_service(&tmp.path().join("data"));
    // Decision 26: the planned run's plan has no task (the driver builds it so).
    let plan = || {
        let mut plan = parse_plan(&one_task_plan()).unwrap();
        plan.tasks.clear();
        plan
    };
    let codex = proto::Runtime::Codex;

    let refused = service
        .build_plan(plan(), root.clone(), false, false, true, planned(codex))
        .await;
    let Err(BuildError::Refused(text)) = refused else {
        panic!("a Codex orchestrator was not refused");
    };
    assert!(text.contains(".codex/config.toml"), "{text}");
    assert!(text.contains("--trust-project"), "{text}");

    let run = match service
        .build_plan(plan(), root.clone(), false, true, true, planned(codex))
        .await
    {
        Ok(run) => run,
        Err(error) => panic!("{}", error.text()),
    };
    assert_eq!(run.state, proto::RunState::Planning);
    assert_eq!(run.trusted_project, vec![".codex/config.toml".to_string()]);
    let o = run
        .orch
        .orchestrator
        .as_ref()
        .expect("an orchestrator record");
    assert_eq!(o.route.runtime, codex);
    assert_eq!(o.otlp_token.len(), 32, "decision 14a's token");
    assert_eq!(run.orch.installed.get("codex"), Some(&false));
    assert!(run.tasks.is_empty());

    let claude = service
        .build_plan(
            plan(),
            root,
            false,
            false,
            true,
            planned(proto::Runtime::Claude),
        )
        .await;
    assert!(claude.is_ok(), "{:?}", claude.err().map(BuildError::text));
}

/// Decisions 9 and 29: `run promote` repeats the check for the runtimes the promoted
/// run would newly reach, honouring what the run's start trusted.
#[tokio::test]
async fn promote_repeats_the_project_settings_check() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join("repo");
    repo_with_codex_config(&root);
    let service = shipped_service(&tmp.path().join("data"));
    let plan = parse_plan(&one_task_plan()).unwrap();
    let mut run = match service
        .build_plan(plan, root, true, true, true, Shape::Fast)
        .await
    {
        Ok(run) => run,
        Err(error) => panic!("{}", error.text()),
    };
    // A fast-path run whose roster's one Codex model is `fast`, below every route and
    // review its task can take, and whose start trusted nothing: it reaches Claude only.
    // Promoted with a Codex orchestrator, it would reach Codex.
    run.path = Some(proto::RunPath::Fast);
    run.state = proto::RunState::Running;
    run.roster.retain(|e| e.runtime == proto::Runtime::Claude);
    run.roster.push(proto::ModelEntry {
        runtime: proto::Runtime::Codex,
        model: "codex-mini".into(),
        strength: proto::Strength::Fast,
        ..run.roster[0].clone()
    });
    run.trusted_project.clear();
    assert_eq!(
        crate::run::reach::reachable_runtimes(&run),
        vec![proto::Runtime::Claude]
    );
    let id = run.id.clone();
    crate::lock(&service.state).runs.insert(id.clone(), run);
    let set = |f: &dyn Fn(&mut crate::run::model::Run)| {
        f(crate::lock(&service.state).runs.get_mut(&id).unwrap())
    };

    let choice = proto::OrchestratorChoice {
        runtime: proto::Runtime::Codex,
        model: None,
    };
    let (codex, claude) = (Some(&choice), None);
    let refusal = service.promote_refusal(&id, codex).await.unwrap_err();
    assert!(refusal.contains(".codex/config.toml"), "{refusal}");
    assert!(
        refusal.starts_with("promoting would start Codex sessions"),
        "{refusal}"
    );
    // `run promote` itself is refused with it, before the engine (not running here)
    // is asked.
    let promote = service.promote(id.clone(), Some(choice.clone()));
    let reply = tokio::time::timeout(std::time::Duration::from_secs(10), promote)
        .await
        .expect("refused without asking the engine");
    assert_eq!(
        reply,
        proto::RunReply::refused(proto::run_wire::request::PROMOTE, refusal)
    );
    // Trusted at the start: it passes.
    set(&|run| run.trusted_project = vec![".codex/config.toml".into()]);
    assert!(service.promote_refusal(&id, codex).await.is_ok());
    // A Claude orchestrator (the run's default) reaches nothing new.
    set(&|run| {
        run.trusted_project.clear();
        run.limits.default_runtime = proto::Runtime::Claude;
    });
    assert!(service.promote_refusal(&id, claude).await.is_ok());
}
