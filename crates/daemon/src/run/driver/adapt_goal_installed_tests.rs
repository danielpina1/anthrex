//! M9.17 fix round 2: decision 26's start check at `build_plan`'s level. A user with
//! only Codex installed starts a planned run (the sub-planners' frontier route stays on
//! Codex instead of stepping to an uninstalled Claude model); a planner runtime that is
//! genuinely missing is still refused before anything is written; a user with both
//! runtimes is routed as before; and `run promote` makes the same check.

use std::path::Path;
use std::sync::Arc;

use proto::{OrchestratorChoice, RunReply, Runtime};

use super::super::super::super::RunContext;
use super::super::super::super::RunService;
use super::super::super::super::build::{Planned, Shape};
use super::super::BuildError;
use super::{INSTALLED_STAND_IN, NoRoots, git, one_task_plan};
use crate::manager::{ManagerConfig, WindowManager};
use crate::run::model::Run;
use crate::run::orch::launch::planner_route;
use crate::run::plan::parse_plan;

/// Claude's configured binary on a machine that has only Codex.
const NO_CLAUDE: &str = "/nonexistent/ax-claude";

/// A service with the shipped caps over `orchestrator`, whose Claude binary is `claude`
/// and whose Codex binary is an installed stand-in no build launches.
fn service(data: &Path, orchestrator: config::Orchestrator, claude: &str) -> Arc<RunService> {
    let mut config = ManagerConfig::for_tests("/tmp/ax-unused.sock".into(), "/bin/sh".into());
    config.claude_bin = claude.into();
    config.codex_bin = INSTALLED_STAND_IN.into();
    config.cli_caps = crate::headless::argv::CLI_CAPS;
    config.worktrees_root = data.join("worktrees");
    let (manager, _events) = WindowManager::new(config);
    let ctx = RunContext::new(
        data.to_path_buf(),
        manager.config(),
        orchestrator,
        Arc::new(NoRoots),
    );
    RunService::new(manager, ctx)
}

/// A repository with no project settings for either runtime.
fn plain_repo(root: &Path) {
    std::fs::create_dir_all(root.join("crates/a/src")).unwrap();
    std::fs::write(root.join("crates/a/src/lib.rs"), "// a\n").unwrap();
    git(root, &["init", "-q", "-b", "main"]);
    git(root, &["config", "user.name", "t"]);
    git(root, &["config", "user.email", "t@t"]);
    git(root, &["add", "-A"]);
    git(root, &["commit", "-q", "-m", "base"]);
}

fn empty_plan() -> proto::Plan {
    let mut plan = parse_plan(&one_task_plan()).unwrap();
    plan.tasks.clear();
    plan
}

fn planned(choice: Option<Runtime>) -> Shape {
    Shape::Planned(Box::new(Planned {
        triage: Some(proto::TriageInfo {
            kinds: vec![proto::TaskKind::Code],
            scale: proto::Scale::Plan,
            path: proto::RunPath::Plan,
            reason: "several modules".into(),
            source: proto::DeciderSource::Decider,
            fallback_reason: None,
            at: 1,
        }),
        usage: None,
        yes: false,
        choice: choice.map(|runtime| OrchestratorChoice {
            runtime,
            model: None,
        }),
    }))
}

fn planners_on(runtime: Runtime) -> config::Orchestrator {
    let mut config = config::Orchestrator::default();
    config.agent.planners.runtime = Some(runtime);
    config
}

async fn build(service: &RunService, root: &Path, choice: Option<Runtime>) -> Result<Run, String> {
    service
        .build_plan(
            empty_plan(),
            root.to_path_buf(),
            false,
            false,
            true,
            planned(choice),
        )
        .await
        .map_err(|e| e.text())
}

/// The orchestrator's and the sub-planners' routes of a built run.
fn routes(run: &Run) -> (proto::Route, proto::Route) {
    let o = run.orch.orchestrator.as_ref().expect("an orchestrator");
    (
        o.route.clone(),
        planner_route(run).expect("a planner route"),
    )
}

/// Nothing a refused start could have left: no run directory, no run branch.
fn assert_nothing_written(data: &Path, root: &Path) {
    let runs = data.join("runs");
    assert!(!runs.exists() || std::fs::read_dir(&runs).unwrap().next().is_none());
    let refs = std::process::Command::new("git")
        .args(["--no-optional-locks", "for-each-ref", "refs/heads/anthrex/"])
        .current_dir(root)
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .output()
        .unwrap();
    assert!(refs.status.success() && refs.stdout.is_empty(), "{refs:?}");
}

/// The review's finding: every Codex-only start was refused, because the default
/// frontier planner route stepped to Claude's frontier model.
#[tokio::test]
async fn a_codex_only_user_starts_planned_runs() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join("repo");
    plain_repo(&root);
    let data = tmp.path().join("data");

    // `--orchestrator codex`, the default config.
    let default = service(&data, config::Orchestrator::default(), NO_CLAUDE);
    let run = build(&default, &root, Some(Runtime::Codex)).await.unwrap();
    let (orchestrator, planner) = routes(&run);
    assert_eq!(orchestrator.runtime, Runtime::Codex);
    assert_eq!(
        (planner.runtime, planner.model.as_str()),
        (Runtime::Codex, "")
    );
    assert_eq!(run.orch.installed.get("claude"), Some(&false));

    // `[orchestrator.planners] runtime = "codex"`.
    let configured = service(&data, planners_on(Runtime::Codex), NO_CLAUDE);
    let run = build(&configured, &root, Some(Runtime::Codex))
        .await
        .unwrap();
    assert_eq!(routes(&run).1.runtime, Runtime::Codex);

    // An unpinned goal: Claude, the default runtime, is skipped for Codex.
    let run = build(&default, &root, None).await.unwrap();
    let (orchestrator, planner) = routes(&run);
    assert_eq!(orchestrator.runtime, Runtime::Codex);
    assert_eq!(planner.runtime, Runtime::Codex);
    let o = run.orch.orchestrator.as_ref().unwrap();
    let first = &o.routing.candidates[0];
    assert_eq!(first.route.runtime, Runtime::Claude);
    assert_eq!(first.skipped_reason.as_deref(), Some("not installed"));
}

/// A sub-planner runtime that is not installed is still refused, naming what to change,
/// before the run is written.
#[tokio::test]
async fn a_missing_planner_runtime_is_still_refused() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join("repo");
    plain_repo(&root);
    let data = tmp.path().join("data");
    let service = service(&data, planners_on(Runtime::Claude), NO_CLAUDE);
    let refusal = build(&service, &root, Some(Runtime::Codex))
        .await
        .unwrap_err();
    assert_eq!(
        refusal,
        format!(
            "the sub-planners' runtime claude is not installed ({NO_CLAUDE} is not an executable file); install it, or change [orchestrator.planners] runtime"
        )
    );
    assert_nothing_written(&data, &root);
}

/// With both runtimes installed nothing changes: a Codex orchestrator's frontier
/// planner is Claude's frontier model, and an unpinned goal gets a Claude orchestrator.
#[tokio::test]
async fn a_user_with_both_runtimes_is_routed_as_before() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join("repo");
    plain_repo(&root);
    let data = tmp.path().join("data");
    let service = service(&data, config::Orchestrator::default(), INSTALLED_STAND_IN);
    let run = build(&service, &root, Some(Runtime::Codex)).await.unwrap();
    let (orchestrator, planner) = routes(&run);
    assert_eq!(orchestrator.runtime, Runtime::Codex);
    assert_eq!(
        (planner.runtime, planner.model.as_str()),
        (Runtime::Claude, "claude-opus-5-5")
    );
    let run = build(&service, &root, None).await.unwrap();
    assert_eq!(routes(&run).0.runtime, Runtime::Claude);
    assert_eq!(routes(&run).1.runtime, Runtime::Claude);
}

/// `run promote` to a runtime that is not installed, or to one whose sub-planners'
/// runtime is not, is refused before the engine is asked, and the run is unchanged.
#[tokio::test]
async fn promoting_to_a_missing_runtime_is_refused_without_side_effects() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join("repo");
    plain_repo(&root);
    let data = tmp.path().join("data");
    let service = service(&data, config::Orchestrator::default(), NO_CLAUDE);
    let plan = parse_plan(&one_task_plan()).unwrap();
    let mut run = match service
        .build_plan(plan, root.clone(), true, false, true, Shape::Fast)
        .await
    {
        Ok(run) => run,
        Err(error) => panic!("{}", error.text()),
    };
    run.path = Some(proto::RunPath::Fast);
    run.state = proto::RunState::Running;
    let id = run.id.clone();
    crate::lock(&service.state)
        .runs
        .insert(id.clone(), run.clone());
    let unchanged = || assert!(crate::lock(&service.state).runs.get(&id) == Some(&run));
    let choice = |runtime| OrchestratorChoice {
        runtime,
        model: None,
    };
    let claude_missing = format!(
        "the orchestrator's runtime claude is not installed ({NO_CLAUDE} is not an executable file); install it, or choose another runtime with --orchestrator"
    );

    // Claude, chosen or the run's default.
    let refusal = service
        .promote_refusal(&id, Some(&choice(Runtime::Claude)))
        .await
        .unwrap_err();
    assert_eq!(refusal, claude_missing);
    assert_eq!(
        service.promote_refusal(&id, None).await.unwrap_err(),
        claude_missing
    );
    let promote = service.promote(id.clone(), Some(choice(Runtime::Claude)));
    let reply = tokio::time::timeout(std::time::Duration::from_secs(10), promote)
        .await
        .expect("refused without asking the engine");
    assert_eq!(
        reply,
        RunReply::refused(proto::run_wire::request::PROMOTE, claude_missing)
    );
    unchanged();

    // Codex is installed, and its frontier sub-planners stay on it.
    assert!(
        service
            .promote_refusal(&id, Some(&choice(Runtime::Codex)))
            .await
            .is_ok()
    );

    // Codex with its sub-planners configured on Claude.
    crate::lock(&service.state)
        .runs
        .get_mut(&id)
        .unwrap()
        .limits
        .orch
        .planners
        .runtime = Some(Runtime::Claude);
    let refusal = service
        .promote_refusal(&id, Some(&choice(Runtime::Codex)))
        .await
        .unwrap_err();
    assert!(
        refusal.starts_with("the sub-planners' runtime claude is not installed"),
        "{refusal}"
    );
    assert_nothing_written(&data, &root);
    let _ = BuildError::Refused(String::new());
}

/// M9.17 fix round 3, item 2: `run promote` through the real engine loop records what
/// its installed check found (`OrchEvent::Installed`, sent before `Promote`), so the
/// promoted run's sub-planners stay on Codex. The launch gate stays closed: the
/// promoted orchestrator's window is never started.
#[tokio::test(flavor = "multi_thread")]
async fn run_promote_records_what_its_check_found() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join("repo");
    plain_repo(&root);
    let data = tmp.path().join("data");
    let mut config = ManagerConfig::for_tests("/tmp/ax-unused.sock".into(), "/bin/sh".into());
    config.claude_bin = NO_CLAUDE.into();
    config.codex_bin = INSTALLED_STAND_IN.into();
    config.cli_caps = crate::headless::argv::CLI_CAPS;
    config.worktrees_root = data.join("worktrees");
    config.launch_gate = crate::launch::LaunchGate::closed();
    let (manager, _events) = WindowManager::new(config);
    let ctx = RunContext::new(
        data.clone(),
        manager.config(),
        config::Orchestrator::default(),
        Arc::new(NoRoots),
    );
    let service = RunService::new(manager, ctx);
    let plan = parse_plan(&one_task_plan()).unwrap();
    let mut run = match service
        .build_plan(plan, root.clone(), true, false, true, Shape::Fast)
        .await
    {
        Ok(run) => run,
        Err(error) => panic!("{}", error.text()),
    };
    run.path = Some(proto::RunPath::Fast);
    run.state = proto::RunState::Running;
    let id = run.id.clone();
    crate::lock(&service.state).runs.insert(id.clone(), run);
    let shutdown = tokio_util::sync::CancellationToken::new();
    service.spawn(shutdown.clone());

    let codex = OrchestratorChoice {
        runtime: Runtime::Codex,
        model: None,
    };
    let promote = service.promote(id.clone(), Some(codex));
    let reply = tokio::time::timeout(std::time::Duration::from_secs(30), promote)
        .await
        .expect("the engine answered");
    shutdown.cancel();
    assert!(
        !matches!(reply, RunReply::Refused { .. }),
        "refused: {reply:?}"
    );
    let state = crate::lock(&service.state);
    let run = &state.runs[&id];
    assert_eq!(
        run.orch.installed,
        [("claude".to_string(), false), ("codex".to_string(), true)].into()
    );
    let planner = planner_route(run).expect("promoted");
    assert_eq!(
        (planner.runtime, planner.model.as_str()),
        (Runtime::Codex, "")
    );
}

/// Whole-branch review, item 1: a run's scouts are among the runtimes its start checks.
/// With `default_runtime = "codex"` or `[orchestrator.scouts] runtime = "codex"`, a run
/// with a Claude orchestrator can still spawn Codex scouts, which load a tracked
/// `.codex/config.toml` unasked; the start is refused without `--trust-project`, and
/// starts with it, the file trusted. With the default config (Claude scouts) it starts.
#[tokio::test]
async fn a_codex_run_scout_is_covered_by_the_project_settings_check() {
    let codex_default = config::Orchestrator {
        default_runtime: Runtime::Codex,
        ..Default::default()
    };
    let mut codex_scouts = config::Orchestrator::default();
    codex_scouts.scouts.runtime = Some(Runtime::Codex);
    for (name, config) in [("default_runtime", codex_default), ("scouts", codex_scouts)] {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("repo");
        super::repo_with_codex_config(&root);
        let data = tmp.path().join("data");
        let service = service(&data, config, INSTALLED_STAND_IN);
        let claude = Some(Runtime::Claude);
        let refused = service
            .build_plan(
                empty_plan(),
                root.clone(),
                false,
                false,
                true,
                planned(claude),
            )
            .await;
        let Err(BuildError::Refused(text)) = refused else {
            panic!("{name}: a run with Codex scouts was not refused");
        };
        assert!(text.contains(".codex/config.toml"), "{name}: {text}");
        let run = match service
            .build_plan(
                empty_plan(),
                root.clone(),
                false,
                true,
                true,
                planned(claude),
            )
            .await
        {
            Ok(run) => run,
            Err(error) => panic!("{name}: {}", error.text()),
        };
        assert_eq!(run.trusted_project, vec![".codex/config.toml".to_string()]);
        assert_eq!(
            crate::run::reach::reachable_runtimes(&run),
            vec![Runtime::Claude, Runtime::Codex],
            "{name}"
        );
    }
    // The control: Claude scouts reach nothing new.
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join("repo");
    super::repo_with_codex_config(&root);
    let service = service(
        &tmp.path().join("data"),
        config::Orchestrator::default(),
        INSTALLED_STAND_IN,
    );
    let run = build(&service, &root, Some(Runtime::Claude)).await.unwrap();
    assert_eq!(
        crate::run::reach::reachable_runtimes(&run),
        vec![Runtime::Claude]
    );
}

/// Whole-branch review, item 4: a run started with `--trust-project` trusts the project
/// settings a plan edit would newly reach, as `run promote` does (decision 9); a run
/// started without it is still refused.
#[tokio::test]
async fn an_edit_of_a_run_started_with_trust_project_is_not_refused() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join("repo");
    super::repo_with_codex_config(&root);
    let data = tmp.path().join("data");
    let service = service(&data, config::Orchestrator::default(), INSTALLED_STAND_IN);
    let plan = parse_plan(&one_task_plan()).unwrap();
    let mut run = match service
        .build_plan(plan, root, true, true, true, Shape::Fast)
        .await
    {
        Ok(run) => run,
        Err(error) => panic!("{}", error.text()),
    };
    // It reaches Claude only (Codex's one model is below every route), so a task added
    // on Codex reaches `.codex/config.toml`, which the start never checked.
    run.roster.retain(|e| e.runtime == Runtime::Claude);
    run.roster.push(proto::ModelEntry {
        runtime: Runtime::Codex,
        model: "codex-mini".into(),
        strength: proto::Strength::Fast,
        ..run.roster[0].clone()
    });
    run.trusted_project.clear();
    assert_eq!(
        crate::run::reach::reachable_runtimes(&run),
        vec![Runtime::Claude]
    );
    let edits = vec![proto::PlanEdit::AddTask {
        task: run.tasks[0].spec.clone(),
    }];
    let id = run.id.clone();
    crate::lock(&service.state).runs.insert(id.clone(), run);
    let refusals = |trust: bool| {
        crate::lock(&service.state)
            .runs
            .get_mut(&id)
            .unwrap()
            .trust_project = trust;
        service.runtime_refusals(&id, &edits)
    };
    let refused = refusals(false).await.unwrap();
    assert_eq!(refused.len(), 1, "{refused:?}");
    assert_eq!(refused[0].0, Runtime::Codex);
    assert!(refused[0].1.contains(".codex/config.toml"), "{refused:?}");
    assert_eq!(refusals(true).await.unwrap(), Vec::new());
}

#[path = "adapt_goal_list_tests.rs"]
mod lists;
