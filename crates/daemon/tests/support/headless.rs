//! Shared pieces for M8a.17's headless-window tests: a session spec, `/bin/sh` scripts
//! standing in for `claude` and `codex` (never the real binaries), and a real manager.

use daemon::headless::{HeadlessSpec, SessionArg};
use daemon::manager::{ManagerConfig, WindowManager, WindowSignal};
use proto::{AgentRole, Effort, RunRef, Runtime, WindowInfo};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::broadcast;

/// Generous: every wait ends on something a live script does within milliseconds.
pub const DEADLINE: Duration = Duration::from_secs(10);
pub const RUN_ID: &str = "run-17";

pub fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/headless")
        .join(name)
}

pub fn run_ref() -> RunRef {
    RunRef {
        run_id: RUN_ID.into(),
        task_id: Some("t1".into()),
        role: AgentRole::Worker,
        session: 1,
    }
}

/// A worker's spec with every field set to something distinct, no MCP server.
pub fn spec(runtime: Runtime, cwd: &Path) -> HeadlessSpec {
    HeadlessSpec {
        runtime,
        model: "model-x".into(),
        effort: Effort::Medium,
        cwd: cwd.to_path_buf(),
        instructions: "the contract".into(),
        mcp: None,
        allowed_tools: vec!["Read".into()],
        claude_permission_mode: Some("acceptEdits".into()),
        claude_disallowed_tools: Vec::new(),
        claude_sandbox: None,
        codex_sandbox: "workspace-write".into(),
        codex_writable_roots: Vec::new(),
        env: vec![("PROFILE_VAR".into(), "profile-value".into())],
        claude_auth: config::ClaudeAuth::Login,
        api_key_helper: None,
        run_ref: Some(run_ref()),
    }
}

/// An executable `/bin/sh` script at `dir/name`.
pub fn script(dir: &Path, name: &str, body: &str) -> PathBuf {
    use std::os::unix::fs::PermissionsExt;
    let path = dir.join(name);
    std::fs::write(&path, format!("#!/bin/sh\n{body}\n")).unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
    path
}

/// Shell text that waits until `dir/name` exists.
pub fn gate(dir: &Path, name: &str) -> String {
    format!(
        "while [ ! -f '{}' ]; do sleep 0.02; done",
        dir.join(name).display()
    )
}

pub fn open_gate(dir: &Path, name: &str) {
    std::fs::write(dir.join(name), b"").unwrap();
}

/// A manager whose Claude and Codex are the given programs, with its events pumped.
pub fn manager(
    claude: &Path,
    codex: &Path,
    configure: impl FnOnce(&mut ManagerConfig),
) -> Arc<WindowManager> {
    let mut config = ManagerConfig::new("/tmp/anthrex-m8a17-unused.sock".into(), "/bin/sh".into());
    config.claude_bin = claude.to_str().unwrap().into();
    config.codex_bin = codex.to_str().unwrap().into();
    configure(&mut config);
    let (manager, mut events) = WindowManager::new(config);
    let pump = manager.clone();
    tokio::spawn(async move {
        while let Some((id, event)) = events.recv().await {
            pump.handle_event(id, event);
        }
    });
    manager
}

pub async fn create(
    m: &Arc<WindowManager>,
    name: &str,
    spec: HeadlessSpec,
    first_turn: &str,
) -> WindowInfo {
    let cwd = spec.cwd.clone();
    let session = match spec.runtime {
        Runtime::Claude => SessionArg::New {
            uuid: Some("00000000-0000-4000-8000-00000000c1a0".into()),
        },
        _ => SessionArg::New { uuid: None },
    };
    m.create_headless(
        name.into(),
        spec,
        session,
        first_turn.into(),
        cwd.clone(),
        cwd,
    )
    .await
    .expect("create_headless")
}

pub fn find(m: &WindowManager, id: u32) -> WindowInfo {
    m.list()
        .into_iter()
        .find(|w| w.id == id)
        .expect("the window is listed")
}

pub async fn wait_until(what: &str, mut done: impl FnMut() -> bool) {
    let deadline = Instant::now() + DEADLINE;
    while !done() {
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

/// The next feed signal matching `pred`, within [`DEADLINE`].
pub async fn next_signal(
    rx: &mut broadcast::Receiver<WindowSignal>,
    what: &str,
    mut pred: impl FnMut(&WindowSignal) -> bool,
) -> WindowSignal {
    tokio::time::timeout(DEADLINE, async {
        loop {
            match rx.recv().await {
                Ok(signal) if pred(&signal) => return signal,
                Ok(_) => {}
                Err(broadcast::error::RecvError::Lagged(_)) => {}
                Err(broadcast::error::RecvError::Closed) => panic!("the feed closed"),
            }
        }
    })
    .await
    .unwrap_or_else(|_| panic!("timed out waiting for {what}"))
}
