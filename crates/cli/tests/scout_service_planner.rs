//! Milestone 9 task M9.8: a sub-planner session on M8b's scout machine (decision 31),
//! through a real `WindowManager` running `fake-agent` as Claude. Codex is a path that
//! does not exist, so no test can reach a real agent binary. (A file of its own:
//! `scout_service.rs`, M8b's suite, is at 592 lines.)

mod support;

use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use daemon::headless::{ClaudeSandbox, HeadlessSpec, McpTarget};
use daemon::manager::{ManagerConfig, WindowManager};
use daemon::run::orch::contract::PLANNER_NUDGE;
use daemon::scout::planner::{PlannerSpec, planner_names};
use daemon::scout::service::{ScoutOutcome, ScoutService};
use daemon::scout::spec::ScoutContext;
use proto::{AgentRole, Effort, Route, RunRef, Runtime, Status, Strength};
use serde_json::json;
use tokio_util::sync::CancellationToken;

use support::run_harness::{init_repo, script_in};
use support::{ANTHREX, fake_agent_bin, tempdir};

const NO_CODEX_BIN: &str = "/nonexistent/anthrex-test/codex";
/// Deadline for the session to start, take a turn, or exit (M8b.9's `SESSION_WAIT`;
/// docs/timing-budgets.md).
const SESSION_WAIT: Duration = Duration::from_secs(30);
const RUN_ID: &str = "planner-test-3f9a";

async fn wait_for(what: &str, limit: Duration, mut done: impl FnMut() -> bool) {
    let deadline = Instant::now() + limit;
    while !done() {
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

fn planner_spec(repo: &std::path::Path) -> PlannerSpec {
    let route = Route {
        runtime: Runtime::Claude,
        model: "claude-opus-4-5".into(),
        strength: Strength::Frontier,
        effort: Effort::High,
    };
    let headless = HeadlessSpec {
        runtime: Runtime::Claude,
        model: route.model.clone(),
        effort: route.effort,
        cwd: repo.to_path_buf(),
        instructions: "You are a sub-planner.".into(),
        mcp: Some(McpTarget {
            role: AgentRole::Planner,
            run_id: RUN_ID.into(),
            task_id: None,
            scout_id: None,
            epic: Some("mail".into()),
        }),
        allowed_tools: vec![
            "mcp__anthrex__get_context".into(),
            "mcp__anthrex__submit_epic".into(),
            "Read".into(),
        ],
        claude_permission_mode: Some("dontAsk".into()),
        claude_disallowed_tools: vec!["Edit".into(), "Write".into(), "NotebookEdit".into()],
        claude_sandbox: Some(ClaudeSandbox {
            writable_roots: Vec::new(),
            deny_write: vec![repo.to_path_buf()],
        }),
        codex_sandbox: "read-only".into(),
        codex_writable_roots: Vec::new(),
        env: Vec::new(),
        claude_auth: config::ClaudeAuth::default(),
        api_key_helper: None,
        run_ref: Some(RunRef {
            run_id: RUN_ID.into(),
            task_id: None,
            role: AgentRole::Planner,
            session: 1,
        }),
        codex_config_guard: None,
        output_filter: None,
    };
    PlannerSpec {
        run_id: RUN_ID.into(),
        epic: "mail".into(),
        session: 1,
        headless,
        first_turn: "[anthrex] Plan epic mail.".into(),
        project: repo.to_path_buf(),
        cwd: repo.to_path_buf(),
        route,
        max_tool_calls: 200,
        timeout_secs: 2400,
        extract: None,
    }
}

/// Decision 31: the session runs on the scout machine with the planner's texts (its
/// turn without a submit is nudged with `PLANNER_NUDGE`), is launched read-only with
/// its epic, is not one of the run's scouts, and is retired when the engine accepts
/// its epic (`accept_planner`): its stdin closes, it exits, and its outcome is
/// `Accepted`.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_planner_session_runs_on_the_scout_machine_and_is_accepted() {
    let dir = tempdir();
    let repo = dir.path().join("repo");
    init_repo(&repo, &[("Cargo.toml", "[workspace]\n")]);
    let wrapper = dir.path().join("claude.sh");
    std::fs::write(
        &wrapper,
        format!(
            "#!/bin/sh\nexport FAKE_AGENT_ARGS_FILE='{}'\nexport FAKE_AGENT_STDIN_FILE='{}'\nexec '{}' \"$@\"\n",
            dir.path().join("args.json").display(),
            dir.path().join("stdin.jsonl").display(),
            fake_agent_bin().display()
        ),
    )
    .unwrap();
    std::fs::set_permissions(&wrapper, std::fs::Permissions::from_mode(0o755)).unwrap();
    let mut config = ManagerConfig::new(dir.path().join("d.sock"), "/bin/sh".into());
    config.exe = PathBuf::from(ANTHREX);
    config.claude_bin = wrapper.display().to_string();
    config.codex_bin = NO_CODEX_BIN.to_string();
    config.worktrees_root = dir.path().join("worktrees");
    // The incident rule: this manager can start nothing but the test's fake-agent.
    assert!(!std::path::Path::new(&config.codex_bin).exists());
    let (manager, mut events) = WindowManager::new(config);
    let pump = manager.clone();
    tokio::spawn(async move {
        while let Some((id, event)) = events.recv().await {
            pump.handle_event(id, event);
        }
    });
    let orchestrator = config::Orchestrator::default();
    let scouts: Arc<ScoutService> = ScoutService::new(
        manager.clone(),
        ScoutContext {
            roster: orchestrator.models.clone(),
            default_runtime: Runtime::Claude,
            scouts: orchestrator.scouts.clone(),
            claude: orchestrator.claude.clone(),
            caps: manager.config().cli_caps,
            data_dir: dir.path().join("data"),
        },
    );
    let shutdown = CancellationToken::new();
    scouts.spawn(shutdown.clone());

    // Its first turn (the first `read_message`), then (the turn ended) the nudge; it then waits, its turn open,
    // until the test has accepted its epic.
    let go = dir.path().join("go");
    let wait = format!(
        "i=0; while [ ! -f '{}' ] && [ $i -lt 600 ]; do sleep 0.05; i=$((i+1)); done",
        go.display()
    );
    let nudge = "Your turn ended without an accepted epic";
    script_in(
        &repo,
        "planner-mail-1",
        &[
            json!({"read_message": {}}),
            json!({"read_message": {"expect": nudge}}),
            json!({"sh": {"cmd": wait}}),
        ],
    );
    let handle = scouts
        .start_planner(planner_spec(&repo))
        .await
        .expect("the planner starts");
    let window = handle.window_id;
    let (id, name) = planner_names(RUN_ID, "mail", 1);
    assert_eq!(handle.id, id);
    let info = manager.list().into_iter().find(|w| w.id == window).unwrap();
    assert_eq!(info.name, name);
    assert_eq!(info.run.as_ref().map(|r| r.role), Some(AgentRole::Planner));
    let stdin = dir.path().join("stdin.jsonl");
    wait_for("the nudge", SESSION_WAIT, || {
        std::fs::read_to_string(&stdin).is_ok_and(|s| s.contains(PLANNER_NUDGE))
    })
    .await;
    // A planner is reported through `RunInfo.planners`, not as a run scout.
    assert!(scouts.run_scouts(RUN_ID).is_empty());
    let args = std::fs::read_to_string(dir.path().join("args.json")).unwrap();
    let argv: Vec<String> = serde_json::from_str(&args).unwrap();
    let after = |flag: &str| {
        let k = argv.iter().position(|a| a == flag).expect(flag);
        argv[k + 1].clone()
    };
    assert_eq!(after("--permission-mode"), "dontAsk");
    let mcp: serde_json::Value = serde_json::from_str(&after("--mcp-config")).unwrap();
    let mcp_args = &mcp["mcpServers"]["anthrex"]["args"];
    assert_eq!(mcp_args[2], "planner", "{mcp_args}");
    assert_eq!(mcp_args[5], "--epic", "{mcp_args}");
    assert_eq!(mcp_args[6], "mail", "{mcp_args}");

    scouts.accept_planner(window);
    std::fs::write(&go, "").unwrap();
    let outcome = tokio::time::timeout(SESSION_WAIT, handle.outcome)
        .await
        .expect("the outcome in time")
        .expect("an outcome");
    assert_eq!(outcome, ScoutOutcome::Accepted);
    wait_for("the planner window to be exited", SESSION_WAIT, || {
        manager
            .list()
            .into_iter()
            .find(|w| w.id == window)
            .is_some_and(|w| w.status == Status::Exited)
    })
    .await;

    manager.shutdown().await;
    shutdown.cancel();
    wait_for("the session to end", SESSION_WAIT, || {
        manager.child_pid(window).ok().flatten().is_none()
    })
    .await;
}
