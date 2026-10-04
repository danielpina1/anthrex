//! Milestone 9.5 decision 38 (task M9.5.5a, review ruling C1), the driver's half: the
//! anthrex server's `McpReady` passes the caller check before it reaches the engine,
//! and the first turn is pasted only into a window that has sent a signal and has no
//! attention open. A real manager and a real PTY window whose `claude` is a stand-in
//! shell script (never an agent); the engine's loop is not running, so what the driver
//! sends is read off its channel. Each test kills only the windows it made.

use std::path::Path;
use std::time::{Duration, Instant};

use proto::{
    AgentRole, Effort, HookSource, RunRef, RunReply, RunRequest, Runtime, Status, WindowSpec,
};
use serde_json::json;
use tokio::sync::mpsc::UnboundedReceiver;

use super::super::*;
use crate::headless::McpTarget;
use crate::launch::LaunchGate;
use crate::launch::role::RoleLaunch;
use crate::manager::{GitRoots, ManagerConfig, QUIET_AFTER};

struct NoRoots;
impl GitRoots for NoRoots {
    fn register(&self, _: std::path::PathBuf) {}
    fn unregister(&self, _: &std::path::Path) {}
}

/// Slack on top of each derived bound below (a process start, a few 20 ms polls).
pub(in crate::run::driver::wake) const SLACK: Duration = Duration::from_secs(5);

pub(in crate::run::driver::wake) struct Rig {
    pub(in crate::run::driver::wake) _dir: tempfile::TempDir,
    pub(in crate::run::driver::wake) manager: Arc<WindowManager>,
    pub(in crate::run::driver::wake) runs: Arc<RunService>,
    pub(in crate::run::driver::wake) events: UnboundedReceiver<crate::run::driver::Msg>,
}

/// `claude` as a stand-in that appends its argv to `<dir>/claude.args`, prints one
/// line and then sleeps (`Starting`, then `Working` on its output, then `Idle` once
/// quiet: an agent at a prompt that has sent no signal).
fn stand_in(dir: &Path) -> String {
    let claude = dir.join("claude");
    let script = "#!/bin/sh\necho \"$*\" >> \"$0.args\"\necho starting\nexec sleep 300\n";
    std::fs::write(&claude, script).unwrap();
    std::fs::set_permissions(&claude, std::os::unix::fs::PermissionsExt::from_mode(0o755)).unwrap();
    claude.to_str().unwrap().into()
}

/// Run `run_id`'s orchestrator role, as the engine launches it.
fn role(run_id: &str) -> RoleLaunch {
    RoleLaunch {
        run_ref: RunRef {
            run_id: run_id.into(),
            task_id: None,
            role: AgentRole::Orchestrator,
            session: 1,
            lane: None,
        },
        mcp: McpTarget {
            role: AgentRole::Orchestrator,
            run_id: run_id.into(),
            task_id: None,
            scout_id: None,
            epic: None,
            chain: None,
            lane: None,
            agent_label: None,
        },
        instructions: "the orchestrator contract".into(),
        effort: Effort::High,
        claude_allowed_tools: Vec::new(),
        claude_disallowed_tools: Vec::new(),
        env: Vec::new(),
        remove_env: Vec::new(),
    }
}

pub(in crate::run::driver::wake) fn rig() -> Rig {
    let dir = tempfile::Builder::new()
        .prefix("ax-first-turn-")
        .tempdir_in("/tmp")
        .unwrap();
    let mut config = ManagerConfig::for_tests(dir.path().join("d.sock"), "/bin/sh".into());
    config.claude_bin = stand_in(dir.path());
    // Milestone 9.5 task M9.5.5b: a Codex orchestrator's tests draw screens with it.
    config.codex_bin = crate::status_codex::rig::stand_in(dir.path());
    config.worktrees_root = dir.path().join("worktrees");
    config.launch_gate = LaunchGate::open_already();
    let (manager, mut pumped) = WindowManager::new(config);
    let pump = manager.clone();
    tokio::spawn(async move {
        while let Some((id, event)) = pumped.recv().await {
            pump.handle_event(id, event);
        }
    });
    let runs = RunService::for_manager(&manager, dir.path().join("data"), Arc::new(NoRoots));
    let events = crate::lock(&runs.rx).take().unwrap();
    Rig {
        _dir: dir,
        manager,
        runs,
        events,
    }
}

impl Rig {
    /// A Claude run window, the orchestrator of run `run_id` as the manager has it.
    pub(in crate::run::driver::wake) async fn orchestrator_window(&self, run_id: &str) -> u32 {
        let spec = WindowSpec {
            name: Some(format!("{run_id}/orchestrator")),
            runtime: Runtime::Claude,
            cwd: self._dir.path().to_path_buf(),
            worktree_branch: None,
            model: None,
            initial_prompt: None,
        };
        self.manager
            .create_run_window(spec, self._dir.path().to_path_buf(), role(run_id))
            .await
            .expect("the stand-in's window")
            .id
    }

    /// Run `r1`, its live orchestrator in `window`, its first turn pending and its
    /// anthrex server ready (the engine's half done).
    fn run_waiting_first_turn(&self, window: u32) {
        let mut run = crate::run::orch::test_support::run_of(1);
        run.id = "r1".into();
        run.orch.mcp_ready = true;
        let mut o = crate::run::orch::test_support::orchestrator();
        o.window_id = Some(window);
        o.launch_op = None;
        o.live = true;
        o.launches = 1;
        o.first_turn_pending = true;
        o.first_prompt = "the first prompt".into();
        run.orch.orchestrator = Some(o);
        crate::lock(&self.runs.state).runs.insert("r1".into(), run);
    }

    /// The engine's first-turn wake-up, queued as `Effect::WakeOrchestrator` is.
    fn queue_first_turn(&self, window: u32) {
        self.runs.queue_wake(
            "r1".into(),
            window,
            "the first prompt".into(),
            (0, 0, None, true),
        );
    }

    pub(in crate::run::driver::wake) fn hook(&self, window: u32, event: &str) {
        let payload = json!({"hook_event_name": event, "session_id": "s-first"});
        self.manager
            .handle_hook(window, HookSource::Claude, &payload)
            .unwrap();
    }

    /// Every `McpReady` and first-turn `OrchestratorWoken` the driver sent since the
    /// last call, as `(kind, run, window or 0)`.
    fn sent(&mut self) -> Vec<(&'static str, String, u32)> {
        let mut seen = Vec::new();
        while let Ok(msg) = self.events.try_recv() {
            match msg {
                crate::run::driver::Msg::Event(EventKind::Orch(OrchEvent::McpReady {
                    run_id,
                    window_id,
                })) => seen.push(("ready", run_id, window_id)),
                crate::run::driver::Msg::Event(EventKind::Orch(OrchEvent::OrchestratorWoken {
                    run_id,
                    first_turn: true,
                    ..
                })) => seen.push(("first turn", run_id, 0)),
                crate::run::driver::Msg::Event(EventKind::Orch(OrchEvent::FirstSignal {
                    run_id,
                    window_id,
                    launch: 1,
                })) => seen.push(("signal", run_id, window_id)),
                _ => {}
            }
        }
        seen
    }

    /// Waits, at most `bound`, until the driver has sent the first turn's
    /// `OrchestratorWoken`.
    async fn wait_first_turn(&mut self, bound: Duration) {
        let deadline = Instant::now() + bound;
        loop {
            if self.sent().iter().any(|(kind, _, _)| *kind == "first turn") {
                return;
            }
            assert!(Instant::now() < deadline, "no first turn within {bound:?}");
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    }

    pub(in crate::run::driver::wake) fn status(&self, window: u32) -> (Status, bool) {
        let w = self
            .manager
            .list()
            .into_iter()
            .find(|w| w.id == window)
            .unwrap();
        (w.status, w.signals_seen)
    }

    fn wait_logs(&self) -> Vec<String> {
        crate::lock(&self.runs.wakes.waits.lines).clone()
    }
}

/// Decision 38's caller check: only the run's orchestrator window, as the manager
/// lists it, makes the engine's `McpReady`; every request is answered `Done`.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn mcp_ready_from_another_window_is_refused() {
    let mut rig = rig();
    let window = rig.orchestrator_window("r1").await;
    let other = rig.orchestrator_window("r2").await;
    for (run_id, window_id) in [("r1", other), ("r1", 9999), ("r2", window)] {
        let reply = rig
            .runs
            .request(RunRequest::McpReady {
                run_id: run_id.into(),
                window_id,
            })
            .await;
        assert_eq!(
            reply,
            RunReply::done(proto::run_wire::request::MCP_READY, "")
        );
    }
    assert_eq!(rig.sent(), vec![], "a notice from another window passed");
    rig.runs
        .request(RunRequest::McpReady {
            run_id: "r1".into(),
            window_id: window,
        })
        .await;
    assert_eq!(rig.sent(), vec![("ready", "r1".to_string(), window)]);
    // The windows this test started, and nothing else.
    let _ = rig.manager.kill(window);
    let _ = rig.manager.kill(other);
}

/// Whole-branch review C, m-1: a chain's orchestrator window adopted by a continued
/// run keeps its anthrex server's `--run` (the chain's first run). Its notice is read,
/// as its tool calls are, for the chain's current run, which the window now serves; a
/// run of no chain still gets nothing for another run's window.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn mcp_ready_from_an_adopted_chain_window_counts_for_its_current_run() {
    let mut rig = rig();
    let window = rig.orchestrator_window("r1").await;
    {
        let mut state = crate::lock(&rig.runs.state);
        for id in ["r1", "r2"] {
            let mut run = crate::run::orch::test_support::run_of(1);
            run.id = id.into();
            run.chain = Some("o-1".into());
            state.runs.insert(id.into(), run);
        }
        let chain = crate::run::chain::Chain {
            id: "o-1".into(),
            project: rig._dir.path().to_path_buf(),
            runs: vec!["r1".into(), "r2".into()],
            window_id: window,
            runtime: Runtime::Claude,
            model: String::new(),
            state: crate::run::chain::ChainState::Active,
            ended: false,
        };
        state.chains.insert("o-1".into(), chain);
    }
    let r2 = RunRef {
        run_id: "r2".into(),
        task_id: None,
        role: AgentRole::Orchestrator,
        session: 1,
        lane: None,
    };
    rig.manager.rebind_run_window(window, r2).unwrap();
    for run_id in ["r1", "r3"] {
        let reply = (rig.runs)
            .request(RunRequest::McpReady {
                run_id: run_id.into(),
                window_id: window,
            })
            .await;
        assert_eq!(
            reply,
            RunReply::done(proto::run_wire::request::MCP_READY, "")
        );
    }
    assert_eq!(rig.sent(), vec![("ready", "r2".to_string(), window)]);
    let _ = rig.manager.kill(window);
}

/// Review ruling C1: an `Idle`, quiet window with no attention open but no signal yet
/// is a start or trust prompt; the first turn is never pasted into it, however often
/// the driver looks, and the wait is logged once. A signal delivers it at the next look.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_unsignalled_window_gets_no_first_turn_even_with_mcp_ready() {
    let mut rig = rig();
    let window = rig.orchestrator_window("r1").await;
    // Its output makes it `Working`; quiet for `QUIET_AFTER`, a tick makes it `Idle`.
    let deadline = Instant::now() + QUIET_AFTER + SLACK;
    while rig.status(window) != (Status::Idle, false) {
        assert!(Instant::now() < deadline, "{:?}", rig.status(window));
        rig.manager.tick();
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    rig.run_waiting_first_turn(window);
    rig.queue_first_turn(window);
    rig.runs.check_orchestrators();
    rig.runs.check_orchestrators();
    tokio::time::sleep(SUBMIT_DELAY * 2).await;
    assert_eq!(rig.sent(), vec![], "pasted into an unsignalled window");
    assert!(crate::lock(&rig.runs.wakes.pending).contains_key("r1"));
    assert_eq!(
        rig.wait_logs(),
        ["wake-up for run r1 waits: no hook signal yet"]
    );
    // A signal: the next look pastes it.
    rig.hook(window, "SessionStart");
    assert_eq!(rig.status(window), (Status::Idle, true));
    rig.runs.check_orchestrators();
    rig.wait_first_turn(SUBMIT_DELAY + SLACK).await;
    // Milestone 9.5 decision 41 (task M9.5.5b): then the delivery's own line.
    let paste = super::super::encode_paste_within("the first prompt", REQUEST_WAKE_MAX_BYTES);
    assert_eq!(
        rig.wait_logs(),
        [
            "wake-up for run r1 waits: no hook signal yet".to_string(),
            format!(
                "wake-up delivered to window {window} ({} bytes)",
                paste.len()
            ),
        ]
    );
    let _ = rig.manager.kill(window);
}

/// The acceptance's other half: with the notice and a signal, an open attention still
/// holds the first turn (a paste's `\r` would answer the prompt); the turn's end lets
/// it go.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_open_attention_holds_the_first_turn() {
    let mut rig = rig();
    let window = rig.orchestrator_window("r1").await;
    rig.hook(window, "PermissionRequest");
    assert_eq!(rig.status(window), (Status::Attention, true));
    rig.run_waiting_first_turn(window);
    rig.queue_first_turn(window);
    rig.runs.check_orchestrators();
    tokio::time::sleep(SUBMIT_DELAY * 2).await;
    assert_eq!(rig.sent(), vec![], "pasted at an open prompt");
    rig.hook(window, "Stop");
    assert_eq!(rig.status(window).0, Status::Done);
    rig.runs.check_orchestrators();
    rig.wait_first_turn(SUBMIT_DELAY + SLACK).await;
    let _ = rig.manager.kill(window);
}

/// Fix round 1, ruling T5a-1: while the first turn waits for its server's notice, the
/// driver reports the window's first signal (with the launch it saw), from which the
/// engine counts `MCP_READY_GRACE_SECS`; with the notice in, it reports nothing.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_first_signal_is_reported_while_the_notice_is_missing() {
    let mut rig = rig();
    let window = rig.orchestrator_window("r1").await;
    rig.run_waiting_first_turn(window);
    let set_ready = |rig: &Rig, ready: bool| {
        crate::lock(&rig.runs.state)
            .runs
            .get_mut("r1")
            .unwrap()
            .orch
            .mcp_ready = ready;
    };
    set_ready(&rig, false);
    rig.runs.check_orchestrators();
    assert_eq!(rig.sent(), vec![], "no signal yet");
    rig.hook(window, "SessionStart");
    rig.runs.check_orchestrators();
    assert_eq!(rig.sent(), vec![("signal", "r1".to_string(), window)]);
    set_ready(&rig, true);
    rig.runs.check_orchestrators();
    assert_eq!(rig.sent(), vec![], "the notice came");
    let _ = rig.manager.kill(window);
}

/// Waits, at most `QUIET_AFTER + SLACK`, until the stand-in has started `n` times;
/// its argv lines.
async fn launches(rig: &Rig, n: usize) -> Vec<String> {
    let path = rig._dir.path().join("claude.args");
    let deadline = Instant::now() + QUIET_AFTER + SLACK;
    loop {
        let text = std::fs::read_to_string(&path).unwrap_or_default();
        let lines: Vec<String> = text.lines().map(str::to_string).collect();
        if lines.len() >= n {
            return lines;
        }
        assert!(Instant::now() < deadline, "{n} launches: {lines:?}");
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

/// Fix round 1 (m1, m2): `RestartOrchestrator` resumes a session only when its first
/// turn was delivered and the window knows the session. A session still waiting for
/// its first turn starts fresh (never `--resume` of a session that has had no turn),
/// and so does one with no session id; either is `RestartedFresh`, so the engine sends
/// the first prompt again.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_restart_resumes_only_a_session_that_had_its_first_turn() {
    use crate::run::engine::{OpKind, OpResult};
    let rig = rig();
    let window = rig.orchestrator_window("r1").await;
    launches(&rig, 1).await;
    rig.hook(window, "SessionStart");
    rig.run_waiting_first_turn(window);
    let ctx = {
        let state = crate::lock(&rig.runs.state);
        super::super::super::context::OpCtx::of(&state.runs["r1"])
    };
    let restart = || {
        let (runs, ctx) = (rig.runs.clone(), ctx.clone());
        async move {
            let kind = OpKind::RestartOrchestrator { window_id: window };
            super::super::super::orch_ops::run(&runs, &ctx, kind).await
        }
    };
    // Pending: fresh, although the window knows a session.
    assert_eq!(restart().await, OpResult::RestartedFresh);
    let argv = launches(&rig, 2).await;
    assert!(!argv[1].contains("--resume"), "{argv:#?}");
    // Delivered, with a session: resumed.
    crate::lock(&rig.runs.state)
        .runs
        .get_mut("r1")
        .unwrap()
        .orch
        .orchestrator
        .as_mut()
        .unwrap()
        .first_turn_pending = false;
    rig.hook(window, "SessionStart");
    assert_eq!(restart().await, OpResult::Restarted);
    let argv = launches(&rig, 3).await;
    assert!(argv[2].contains("--resume s-first"), "{argv:#?}");
    // Delivered, but no session known (its hooks never came): fresh.
    rig.manager.forget_session(window);
    assert_eq!(restart().await, OpResult::RestartedFresh);
    let argv = launches(&rig, 4).await;
    assert!(!argv[3].contains("--resume"), "{argv:#?}");
    let _ = rig.manager.kill(window);
}
