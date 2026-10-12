//! Milestone 9.11 decision 19 (M9.11.13, B6): a session spawned just before the daemon
//! stops has its pid in the journal, the only place restore can find it; nothing else
//! of the stopping op's handling changes. The sessions are `/bin/sh` stand-ins this test
//! starts and removes; no agent is reached.

use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};

use super::super::RunService;
use super::super::effects::Ready;
use crate::headless::HeadlessSpec;
use crate::manager::{GitRoots, ManagerConfig, WindowManager};
use crate::run::driver::OpCtx;
use crate::run::engine::{EventKind, OpKind, OpResult};
use crate::run::journal::{JOURNAL_FILE, JournalLine};

/// The workspace's allowance for a loaded machine (`docs/timing-budgets.md`).
const WALL_CLOCK_SLACK: Duration = Duration::from_secs(5);
const RUN_ID: &str = "r-launch";
const OP: u64 = 7;

struct NoRoots;
impl GitRoots for NoRoots {
    fn register(&self, _: std::path::PathBuf) {}
    fn unregister(&self, _: &Path) {}
}

fn spec(cwd: &Path) -> HeadlessSpec {
    HeadlessSpec {
        runtime: proto::Runtime::Claude,
        model: "model-x".into(),
        effort: proto::Effort::MEDIUM,
        cwd: cwd.to_path_buf(),
        instructions: "the contract".into(),
        mcp: None,
        allowed_tools: vec!["Read".into()],
        claude_permission_mode: None,
        claude_disallowed_tools: Vec::new(),
        claude_sandbox: None,
        codex_sandbox: "workspace-write".into(),
        codex_writable_roots: Vec::new(),
        codex_read_only: Vec::new(),
        codex_grant_dialect: None,
        env: Vec::new(),
        claude_auth: config::ClaudeAuth::Login,
        api_key_helper: None,
        run_ref: None,
        codex_config_guard: None,
        output_filter: None,
    }
}

/// A service whose Claude is `claude`, its op hook stopping the service the moment a
/// `CreateWindow` returns (the injected race with `daemon stop`), and the engine's queue.
fn rig(
    dir: &Path,
    claude: &str,
) -> (
    Arc<RunService>,
    tokio::sync::mpsc::UnboundedReceiver<super::super::Msg>,
) {
    let mut config = ManagerConfig::for_tests(dir.join("d.sock"), "/bin/sh".into());
    config.claude_bin = claude.into();
    config.codex_bin = "/nonexistent/anthrex-test/codex".into();
    config.decider_bin = Some("/nonexistent/anthrex-test/decider".into());
    let (manager, mut events) = WindowManager::new(config);
    let pump = manager.clone();
    tokio::spawn(async move {
        while let Some((id, event)) = events.recv().await {
            pump.handle_event(id, event);
        }
    });
    let s = RunService::for_manager(&manager, dir.join("data"), Arc::new(NoRoots));
    let engine = crate::lock(&s.rx).take().expect("the engine's queue");
    *crate::lock(&s.after_op) = Some(Box::new(|s: &RunService, name| {
        if name == "CreateWindow" {
            s.stopped.store(true, Ordering::SeqCst);
        }
    }));
    (s, engine)
}

/// Starts the launch as the driver starts every op, and returns once its task has ended:
/// the task holds its run's op lock shared for its whole life.
async fn launch(s: &Arc<RunService>, dir: &Path) -> std::path::PathBuf {
    let data_dir = dir.join("runs").join(RUN_ID);
    std::fs::create_dir_all(&data_dir).unwrap();
    let ctx = OpCtx {
        run_id: RUN_ID.into(),
        project: dir.to_path_buf(),
        data_dir: data_dir.clone(),
        git_timeout: Duration::from_secs(5),
        check_timeout: Duration::from_secs(5),
        confine: None,
        pinned: Default::default(),
        pool: None,
        pool_grant: None,
    };
    let kind = OpKind::CreateWindow {
        name: "launch-1".into(),
        spec: Box::new(spec(dir)),
        session_uuid: Some("00000000-0000-4000-8000-0000000000d1".into()),
        first_turn: "go".into(),
        project: dir.to_path_buf(),
        worktree: dir.to_path_buf(),
        jitter_ms: 0,
        extract: None,
    };
    s.execute(vec![Ready::Op { ctx, op: OP, kind }], 0).await;
    // A `/bin/sh` stand-in's spawn, warmed first, plus the journal's two appends.
    let ended = tokio::time::timeout(WALL_CLOCK_SLACK, s.op_lock(RUN_ID).write_owned());
    let _ended = ended.await.expect("the launch's task ended");
    data_dir
}

fn done_lines(data_dir: &Path) -> Vec<OpResult> {
    let text = std::fs::read_to_string(data_dir.join(JOURNAL_FILE)).unwrap_or_default();
    (text.lines())
        .filter_map(|line| match serde_json::from_str(line) {
            Ok(JournalLine::Done { op, result }) if op == OP => Some(result),
            _ => None,
        })
        .collect()
}

fn no_op_done(engine: &mut tokio::sync::mpsc::UnboundedReceiver<super::super::Msg>) {
    while let Ok(msg) = engine.try_recv() {
        if let super::super::Msg::Event(EventKind::OpDone { .. }) = msg {
            panic!("an OpDone reached the engine while the daemon stops");
        }
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_spawned_sessions_pid_is_journaled_while_stopping() {
    let dir = tempfile::tempdir().unwrap();
    // Only the session (its argv names its id) stays up; anything else, the warm-up
    // and the model probe a service makes, exits at once.
    let claude = testexec::write_executable(
        dir.path().join("claude"),
        "#!/bin/sh\ncase \"$*\" in *0000000000d1*) exec sleep 30 ;; esac\nexit 0\n",
    );
    let warmed = std::process::Command::new(&claude).status().unwrap();
    assert!(warmed.success());
    let (s, mut engine) = rig(dir.path(), claude.to_str().unwrap());
    let started = Instant::now();
    let data_dir = launch(&s, dir.path()).await;
    let windows = s.manager.list();
    let window = windows
        .iter()
        .find(|w| w.name == "launch-1")
        .expect("the session's window");
    // The session this test started is killed and removed on every path out (D9-M2), by
    // its own handle: its `ProcessStarted` may not have been applied yet, and `remove`
    // signals only a child it knows is alive.
    let _session = Session(s.manager.clone(), window.id);
    let pid = s.manager.headless_pid(window.id);
    assert!(pid.is_some(), "the stand-in is running");
    assert_eq!(
        done_lines(&data_dir),
        vec![OpResult::Window {
            window_id: window.id,
            pid,
        }],
        "after {:?}",
        started.elapsed()
    );
    no_op_done(&mut engine);
}

struct Session(Arc<WindowManager>, u32);

impl Drop for Session {
    fn drop(&mut self) {
        let _ = self.0.headless_kill(self.1);
        let _ = self.0.remove(self.1);
    }
}

/// Pinning: a launch that failed while the daemon stops is still not journaled.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_failed_launch_is_still_not_journaled_while_stopping() {
    let dir = tempfile::tempdir().unwrap();
    let (s, mut engine) = rig(dir.path(), "/nonexistent/anthrex-test/claude");
    let data_dir = launch(&s, dir.path()).await;
    assert_eq!(done_lines(&data_dir), Vec::new());
    no_op_done(&mut engine);
    assert!(s.manager.list().iter().all(|w| w.name != "launch-1"));
}
