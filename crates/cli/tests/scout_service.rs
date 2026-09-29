//! M8b.9: scouts as real headless windows (decisions 12 to 15). A real `WindowManager`
//! runs `fake-agent` as Claude, a real socket server answers `anthrex mcp`, and every
//! `submit_scout_report` goes through `RunService::request(Tool)` to
//! `ScoutService::tool`. Codex is a path that does not exist, so no test can reach a
//! real agent binary.

mod support;

use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use daemon::decider::DeciderContext;
use daemon::launch::LaunchGate;
use daemon::manager::{ManagerConfig, WindowManager};
use daemon::profile::service::{ProfileContext, ProfileService};
use daemon::run::driver::{Adaptation, INTERRUPT_GRACE, RETIRE_AFTER, RunService};
use daemon::scout::contract::SCOUT_NUDGE;
use daemon::scout::service::{ScoutHandle, ScoutOutcome, ScoutService};
use daemon::scout::spec::{ScoutContext, ScoutSpec};
use daemon::server::{GitWiring, serve};
use proto::{
    AgentRole, ClientKind, ClientMsg, DaemonMsg, PROTO_VERSION, RunReply, Runtime, ScoutKind,
    ScoutReport, ScoutState, Status, ToolCall, read_frame, write_frame,
};
use serde_json::{Value, json};
use tokio::net::UnixStream;
use tokio_util::sync::CancellationToken;

use support::run_harness::{init_repo, script_in};
use support::{ANTHREX, fake_agent_bin, tempdir};

/// A runtime command that exists nowhere, for the runtime no test runs.
const NO_CODEX_BIN: &str = "/nonexistent/anthrex-test/codex";
/// Deadline for a scout's session to start, report, or exit (spawns, `anthrex mcp`, one
/// socket round trip; docs/timing-budgets.md, M8b.9).
const SESSION_WAIT: Duration = Duration::from_secs(30);

struct Rig {
    dir: tempfile::TempDir,
    repo: PathBuf,
    manager: Arc<WindowManager>,
    scouts: Arc<ScoutService>,
    shutdown: CancellationToken,
}

impl Rig {
    async fn new() -> Rig {
        Rig::with(Runtime::Claude, None, LaunchGate::open_already()).await
    }

    /// A rig whose scouts run on `runtime`. Claude is always the test's fake-agent;
    /// Codex is too for a Codex rig, else a path that does not exist.
    async fn with(runtime: Runtime, max_tool_calls: Option<u32>, gate: LaunchGate) -> Rig {
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

        let socket = dir.path().join("d.sock");
        let listener = tokio::net::UnixListener::bind(&socket).unwrap();
        let mut config = ManagerConfig::for_tests(socket, "/bin/sh".into());
        config.exe = PathBuf::from(ANTHREX);
        config.claude_bin = wrapper.display().to_string();
        config.codex_bin = match runtime {
            Runtime::Codex => wrapper.display().to_string(),
            _ => NO_CODEX_BIN.to_string(),
        };
        config.worktrees_root = dir.path().join("worktrees");
        config.launch_gate = gate;
        // The incident rule: this manager can start nothing but the test's fake-agent.
        let ours = |bin: &str| bin == wrapper.display().to_string();
        assert!(ours(&config.claude_bin));
        match runtime {
            Runtime::Codex => assert!(ours(&config.codex_bin)),
            _ => assert!(!Path::new(&config.codex_bin).exists()),
        }
        let (manager, mut events) = WindowManager::new(config);
        let pump = manager.clone();
        tokio::spawn(async move {
            while let Some((id, event)) = events.recv().await {
                pump.handle_event(id, event);
            }
        });
        let shutdown = CancellationToken::new();
        let data = dir.path().join("data");
        let git = GitWiring::new(config::Git {
            enabled: false,
            ..config::Git::default()
        });
        let runs = RunService::for_manager(&manager, data.clone(), git.registry.clone());
        runs.spawn(shutdown.clone());
        let orchestrator = config::Orchestrator::default();
        let mut limits = orchestrator.scouts.clone();
        limits.max_tool_calls = max_tool_calls.unwrap_or(limits.max_tool_calls);
        let scouts = ScoutService::new(
            manager.clone(),
            ScoutContext {
                roster: orchestrator.models.clone(),
                default_runtime: runtime,
                scouts: limits,
                claude: orchestrator.claude.clone(),
                caps: manager.config().cli_caps,
                data_dir: data,
            },
        );
        scouts.spawn(shutdown.clone());
        let profiles = ProfileService::new(
            scouts.clone(),
            manager.clone(),
            ProfileContext {
                data_dir: dir.path().join("data"),
                worktrees_root: dir.path().join("worktrees"),
                git: "git".into(),
                orchestrator: orchestrator.clone(),
                git_queue: runs.git_queue(),
                cli_caps: manager.config().cli_caps,
                daemon_socket: dir.path().join("d.sock"),
            },
        );
        // No run here asks a decider; mode off keeps any call from spawning one.
        let mut deciders =
            DeciderContext::new(&orchestrator, manager.config(), &dir.path().join("data"));
        deciders.mode = proto::DeciderMode::Off;
        runs.set_adaptation(Adaptation {
            profiles,
            scouts: scouts.clone(),
            deciders,
        });
        tokio::spawn(serve(
            listener,
            manager.clone(),
            git,
            runs,
            shutdown.clone(),
        ));
        Rig {
            dir,
            repo,
            manager,
            scouts,
            shutdown,
        }
    }

    fn spec(&self, id: &str) -> ScoutSpec {
        ScoutSpec {
            id: id.into(),
            kind: ScoutKind::Onboarding,
            run_id: None,
            question: "How is this repository built and tested?".into(),
            first_turn: "[anthrex] Work out this repository's profile.".into(),
            cwd: self.repo.clone(),
            project: self.repo.clone(),
            web: false,
            codex_config: Vec::new(),
            base_sha: String::new(),
            repo_paths: vec![self.repo.join(".git")],
        }
    }

    async fn start(&self, id: &str, steps: &[Value]) -> ScoutHandle {
        script_in(&self.repo, &format!("scout-{id}-1"), steps);
        self.scouts.start(self.spec(id)).await.expect("scout start")
    }

    fn status(&self, window: u32) -> Option<Status> {
        self.manager
            .list()
            .into_iter()
            .find(|w| w.id == window)
            .map(|w| w.status)
    }

    fn file(&self, name: &str) -> String {
        std::fs::read_to_string(self.dir.path().join(name)).unwrap_or_default()
    }

    /// Stops what is left and waits, with a deadline, until no session of ours runs.
    async fn finish(self) {
        self.manager.shutdown().await;
        self.shutdown.cancel();
        let windows: Vec<u32> = self.manager.list().iter().map(|w| w.id).collect();
        for window in windows {
            wait_for("the session to end", SESSION_WAIT, || {
                self.manager.child_pid(window).ok().flatten().is_none()
            })
            .await;
        }
    }
}

async fn wait_for(what: &str, limit: Duration, mut done: impl FnMut() -> bool) {
    let deadline = Instant::now() + limit;
    while !done() {
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

async fn outcome(handle: ScoutHandle) -> ScoutOutcome {
    tokio::time::timeout(SESSION_WAIT, handle.outcome)
        .await
        .expect("the scout's outcome in time")
        .expect("an outcome")
}

fn report_args() -> Value {
    json!({
        "summary": "A Rust workspace; cargo builds and tests it.",
        "files": [{"path": "Cargo.toml", "why": "the workspace manifest"}],
        "modules": ["crates/*"],
        "risks": ["no CI configuration"],
        "profile": {"languages": ["rust"], "check": "cargo test", "manifests": ["Cargo.toml"]},
    })
}

fn report_step() -> Value {
    json!({"mcp_call": {"tool": "submit_scout_report", "args": report_args()}})
}

fn call(window_id: u32, scout: &str) -> ToolCall {
    ToolCall {
        run_id: String::new(),
        task_id: None,
        role: AgentRole::Scout,
        window_id,
        tool: "submit_scout_report".into(),
        args: report_args(),
        scout_id: Some(scout.into()),
        epic: None,
    }
}

fn refusal(reply: RunReply) -> String {
    match reply {
        RunReply::ToolResult {
            ok: false, text, ..
        } => text,
        other => panic!("expected a refusal, got {other:?}"),
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn scout_report_is_stored_and_the_session_retired() {
    let rig = Rig::new().await;
    let id = "onboarding-1695000000";
    let handle = rig.start(id, &[report_step()]).await;
    let window = handle.window_id;
    let report = match outcome(handle).await {
        ScoutOutcome::Report(report) => report,
        other => panic!("expected a report, got {other:?}"),
    };
    assert_eq!(report.id, id);
    assert_eq!(report.kind, ScoutKind::Onboarding);
    assert_eq!(report.run_id, None);
    assert_eq!(report.window_id, Some(window));
    assert_eq!(report.summary, report_args()["summary"]);
    assert_eq!(report.files[0].path, "Cargo.toml");
    assert_eq!(report.modules, ["crates/*"]);
    assert_eq!(report.risks, ["no CI configuration"]);
    assert_eq!(report.route.model, "claude-haiku-4-5");
    // Ruling M2: the report call itself is not counted.
    assert_eq!(report.tool_calls, 0, "{report:?}");
    let profile = report.profile.clone().expect("the onboarding profile");
    assert_eq!(profile.check.as_deref(), Some("cargo test"));

    let path = rig.scouts.report_path(&rig.spec(id));
    let data = rig.dir.path().join("data");
    assert_eq!(
        path,
        daemon::profile::repo_dir(&data, &rig.repo).join(format!("scouts/{id}.json"))
    );
    let stored: ScoutReport = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    assert_eq!(stored, report);
    let info = rig.scouts.info(id).expect("the scout is known");
    assert_eq!(info.state, ScoutState::Reported);
    assert_eq!(info.files, ["Cargo.toml"]);

    // Retired: stdin closed, so the session ends by itself; the window stays listed,
    // `Exited`, then is removed after `RETIRE_AFTER`. The test only looks.
    wait_for("the scout window to be exited", SESSION_WAIT, || {
        rig.status(window) == Some(Status::Exited)
    })
    .await;
    wait_for(
        "the scout window to be removed",
        RETIRE_AFTER + INTERRUPT_GRACE + Duration::from_secs(5),
        || rig.status(window).is_none(),
    )
    .await;
    // Ruling R-T9-2: the finished scout left the table with its window.
    assert!(rig.scouts.info(id).is_none());
    rig.finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn scout_without_a_report_is_nudged_then_failed() {
    let rig = Rig::new().await;
    // The first `read_message` takes the first turn's message; the second ends that
    // turn and takes the next one, which must be the nudge; the third ends the second.
    let steps = [
        json!({"read_message": {}}),
        json!({"read_message": {"expect": "Your turn ended without a report"}}),
        json!({"read_message": {}}),
    ];
    let handle = rig.start("onboarding-1695000001", &steps).await;
    let window = handle.window_id;
    assert_eq!(
        outcome(handle).await,
        ScoutOutcome::Failed {
            reason: "the scout ended two turns without a report".into()
        }
    );
    let stdin = rig.file("stdin.jsonl");
    let lines: Vec<&str> = stdin.lines().collect();
    assert_eq!(lines.len(), 2, "{stdin}");
    let second: Value = serde_json::from_str(lines[1]).unwrap();
    assert_eq!(
        second["message"]["content"][0]["text"], SCOUT_NUDGE,
        "{second}"
    );
    wait_for("the failed scout to be exited", SESSION_WAIT, || {
        rig.status(window) == Some(Status::Exited)
    })
    .await;
    rig.finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_report_from_another_window_is_refused() {
    let rig = Rig::new().await;
    let id = "onboarding-1695000002";
    let handle = rig.start(id, &[json!({"hang": {}})]).await;
    let window = handle.window_id;
    assert_eq!(
        refusal(rig.scouts.tool(call(window + 100, id)).await),
        format!("this window is not scout {id}")
    );
    assert_eq!(
        refusal(rig.scouts.tool(call(window, "onboarding-9")).await),
        "unknown scout onboarding-9"
    );
    rig.scouts.stop(id);
    assert_eq!(
        outcome(handle).await,
        ScoutOutcome::Failed {
            reason: "stopped by the user".into()
        }
    );
    assert_eq!(
        refusal(rig.scouts.tool(call(window, id)).await),
        format!("scout {id} is failed")
    );
    rig.finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_second_report_is_refused() {
    let rig = Rig::new().await;
    let id = "onboarding-1695000003";
    let handle = rig.start(id, &[report_step()]).await;
    let window = handle.window_id;
    assert!(matches!(outcome(handle).await, ScoutOutcome::Report(_)));
    assert_eq!(
        refusal(rig.scouts.tool(call(window, id)).await),
        format!("a report for scout {id} was already recorded")
    );
    rig.finish().await;
}

fn after<'a>(args: &'a [String], flag: &str) -> &'a str {
    let at = args.iter().position(|a| a == flag).expect(flag);
    &args[at + 1]
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_scout_argv_is_read_only() {
    let rig = Rig::new().await;
    let handle = rig.start("onboarding-1695000004", &[report_step()]).await;
    assert!(matches!(outcome(handle).await, ScoutOutcome::Report(_)));
    let args: Vec<String> = serde_json::from_str(&rig.file("args.json")).unwrap();
    assert_eq!(after(&args, "--permission-mode"), "dontAsk");
    assert_eq!(after(&args, "--disallowedTools"), "Edit,Write,NotebookEdit");
    assert!(!after(&args, "--allowedTools").contains("Edit"));
    let settings: Value = serde_json::from_str(after(&args, "--settings")).unwrap();
    let filesystem = &settings["sandbox"]["filesystem"];
    assert_eq!(filesystem["allowWrite"], json!([]), "{settings}");
    // Ruling R-T1-1: the checkout root and the repository's git directory are denied.
    let denied: Vec<&str> = filesystem["denyWrite"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap())
        .collect();
    let repo = rig.repo.display().to_string();
    let git_dir = rig.repo.join(".git").display().to_string();
    assert!(denied.contains(&repo.as_str()), "{denied:?}");
    assert!(denied.contains(&git_dir.as_str()), "{denied:?}");
    assert_eq!(settings["sandbox"]["enabled"], json!(true));
    let mcp: Value = serde_json::from_str(after(&args, "--mcp-config")).unwrap();
    let mcp_args = &mcp["mcpServers"]["anthrex"]["args"];
    assert_eq!(mcp_args[2], "scout");
    assert!(!mcp_args.as_array().unwrap().iter().any(|a| a == "--run"));
    rig.finish().await;
}

async fn connect(socket: &Path) -> UnixStream {
    let mut stream = UnixStream::connect(socket).await.unwrap();
    let hello = ClientMsg::Hello {
        proto_version: PROTO_VERSION,
        client: ClientKind::Cli,
    };
    write_frame(&mut stream, &hello).await.unwrap();
    let welcome = read_frame::<_, DaemonMsg>(&mut stream).await.unwrap();
    assert!(
        matches!(welcome, Some(DaemonMsg::Welcome { .. })),
        "{welcome:?}"
    );
    stream
}

async fn error_reply(stream: &mut UnixStream) -> DaemonMsg {
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let message = read_frame::<_, DaemonMsg>(stream).await.unwrap().unwrap();
            if matches!(message, DaemonMsg::Error { .. }) {
                return message;
            }
        }
    })
    .await
    .expect("an error reply in time")
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_run_less_scout_window_refuses_client_input_with_the_profile_hint() {
    let rig = Rig::new().await;
    let id = "onboarding-1695000005";
    // The first turn is taken, so the session is working, then it hangs in that turn.
    let steps = [json!({"read_message": {}}), json!({"hang": {}})];
    let handle = rig.start(id, &steps).await;
    let window = handle.window_id;
    wait_for("the scout to be working", SESSION_WAIT, || {
        rig.manager.child_pid(window).ok().flatten().is_some()
            && rig.status(window) == Some(Status::Working)
    })
    .await;
    let pid = rig.manager.child_pid(window).unwrap().unwrap();
    let mut client = connect(&rig.dir.path().join("d.sock")).await;
    let scout_only = format!(
        "window {window} is a headless scout session; only the daemon drives it. Use anthrex profile reject to stop it"
    );
    let cases = [
        (
            ClientMsg::Input {
                window_id: window,
                bytes: b"rm -rf /\n".to_vec(),
            },
            "input",
            scout_only.clone(),
        ),
        (ClientMsg::Kill { window_id: window }, "kill", scout_only),
        (
            ClientMsg::Subscribe {
                window_id: window,
                cols: 80,
                rows: 24,
            },
            "subscribe",
            format!("window {window} is a headless session; open its conversation with C-b m"),
        ),
    ];
    for (message, request, text) in cases {
        write_frame(&mut client, &message).await.unwrap();
        assert_eq!(
            error_reply(&mut client).await,
            DaemonMsg::Error {
                request: request.into(),
                message: text,
            },
            "{message:?}"
        );
        assert_eq!(rig.manager.child_pid(window).unwrap(), Some(pid));
    }
    // Task 11 review I1: the scout is still alive after every refusal, not merely
    // not yet reaped.
    assert_eq!(rig.status(window), Some(Status::Working));
    rig.scouts.stop(id);
    assert!(matches!(outcome(handle).await, ScoutOutcome::Failed { .. }));
    rig.finish().await;
}

/// Ruling I2: a Codex scout's first process exits after its turn ends, while the nudge's
/// `exec resume` runs; that exit is the turn's end, not the scout's.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_codex_scout_is_nudged_then_reports() {
    let rig = Rig::with(Runtime::Codex, None, LaunchGate::open_already()).await;
    let steps = [
        json!({"read_message": {}}),
        json!({"read_message": {"expect": "Your turn ended without a report"}}),
        report_step(),
    ];
    let handle = rig.start("onboarding-1695000006", &steps).await;
    let report = match outcome(handle).await {
        ScoutOutcome::Report(report) => report,
        other => panic!("expected a report, got {other:?}"),
    };
    assert_eq!(report.route.runtime, Runtime::Codex);
    let args: Vec<String> = serde_json::from_str(&rig.file("args.json")).unwrap();
    assert_eq!(&args[..2], ["exec", "resume"], "{args:?}");
    assert_eq!(args.last().map(String::as_str), Some(SCOUT_NUDGE));
    rig.finish().await;
}

/// Ruling I1: Codex cannot take a message mid-turn, so the wrap-up comes at the turn's
/// end, in place of the nudge; the report call does not count toward the budget (M2).
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_codex_scout_gets_its_wrap_up_at_the_turn_end() {
    let rig = Rig::with(Runtime::Codex, Some(2), LaunchGate::open_already()).await;
    let steps = [
        json!({"read_message": {}}),
        json!({"sh": {"cmd": "true"}}),
        json!({"sh": {"cmd": "true"}}),
        json!({"read_message": {"expect": "You have used 2 tool calls"}}),
        report_step(),
    ];
    let handle = rig.start("onboarding-1695000007", &steps).await;
    let report = match outcome(handle).await {
        ScoutOutcome::Report(report) => report,
        other => panic!("expected a report, got {other:?}"),
    };
    assert_eq!(report.tool_calls, 2);
    let args: Vec<String> = serde_json::from_str(&rig.file("args.json")).unwrap();
    assert_eq!(
        args.last().map(String::as_str),
        Some(daemon::scout::contract::scout_wrap_up(2).as_str())
    );
    rig.finish().await;
}

/// Ruling M4: a stop while `create_headless` waits (here, at a closed launch gate) is
/// done once the window exists.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_scout_stopped_before_its_window_exists_is_killed_when_it_does() {
    let gate = LaunchGate::closed();
    let rig = Arc::new(Rig::with(Runtime::Claude, None, gate.clone()).await);
    let id = "onboarding-1695000008";
    script_in(&rig.repo, &format!("scout-{id}-1"), &[json!({"hang": {}})]);
    let starter = {
        let rig = rig.clone();
        tokio::spawn(async move { rig.scouts.start(rig.spec(id)).await })
    };
    wait_for("the scout to be listed", SESSION_WAIT, || {
        rig.scouts
            .info(id)
            .is_some_and(|i| i.state == ScoutState::Working)
    })
    .await;
    rig.scouts.stop(id);
    assert_eq!(
        rig.scouts.info(id).unwrap().failure.as_deref(),
        Some("stopped by the user")
    );
    gate.open();
    let handle = starter.await.unwrap().expect("the window is still created");
    let window = handle.window_id;
    assert_eq!(
        outcome(handle).await,
        ScoutOutcome::Failed {
            reason: "stopped by the user".into()
        }
    );
    wait_for("the stopped scout's session to end", SESSION_WAIT, || {
        rig.status(window) == Some(Status::Exited)
            && rig.manager.child_pid(window).ok().flatten().is_none()
    })
    .await;
    Arc::try_unwrap(rig).ok().expect("one rig").finish().await;
}
