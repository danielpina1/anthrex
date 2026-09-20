//! Shared harness for the daemon's integration tests: a real daemon on an isolated,
//! temporary Unix socket, and a minimal client that speaks the wire protocol directly
//! (not `daemon::server`'s own client-side counterpart, so tests exercise exactly what
//! goes over the socket). Split out of `tests/server.rs` per AGENTS.md's ~600-line
//! guideline once `tests/server_git.rs` needed the same harness.
#![allow(dead_code)]

use daemon::manager::{ManagerConfig, WindowManager};
use daemon::server::serve;
use proto::{ClientKind, ClientMsg, DaemonMsg, Runtime, WindowSpec, read_frame, write_frame};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;
use tokio::net::UnixStream;
use tokio::net::unix::{OwnedReadHalf, OwnedWriteHalf};
use tokio_util::sync::CancellationToken;

pub struct TestDaemon {
    pub _dir: tempfile::TempDir,
    pub socket: PathBuf,
    pub shutdown: CancellationToken,
    pub manager: Arc<WindowManager>,
}

impl Drop for TestDaemon {
    fn drop(&mut self) {
        self.shutdown.cancel();
        for window in self.manager.list() {
            let _ = self.manager.remove(window.id);
        }
    }
}

pub async fn start_daemon() -> TestDaemon {
    start_daemon_with_git(true).await
}

pub async fn start_daemon_with_git(git_enabled: bool) -> TestDaemon {
    let dir = tempfile::tempdir().unwrap();
    let socket = dir.path().join("d.sock");
    let listener = tokio::net::UnixListener::bind(&socket).unwrap();
    use std::os::unix::fs::PermissionsExt;
    let stub = dir.path().join("stub.sh");
    std::fs::write(&stub, "#!/bin/sh\nexec sleep 300\n").unwrap();
    std::fs::set_permissions(&stub, std::fs::Permissions::from_mode(0o755)).unwrap();
    let mut config = ManagerConfig::new(socket.clone(), "/bin/sh".into());
    config.claude_bin = stub.to_str().unwrap().into();
    let (manager, mut events) = WindowManager::new(config);
    let pump = manager.clone();
    tokio::spawn(async move {
        while let Some((id, ev)) = events.recv().await {
            pump.handle_event(id, ev);
        }
    });
    let shutdown = CancellationToken::new();
    tokio::spawn(serve(
        listener,
        manager.clone(),
        git_enabled,
        shutdown.clone(),
    ));
    TestDaemon {
        _dir: dir,
        socket,
        shutdown,
        manager,
    }
}

pub struct Client {
    pub rd: OwnedReadHalf,
    pub wr: OwnedWriteHalf,
}

impl Client {
    pub async fn connect(d: &TestDaemon, version: u32) -> (Self, DaemonMsg) {
        let stream = UnixStream::connect(&d.socket).await.unwrap();
        let (rd, wr) = stream.into_split();
        let mut c = Client { rd, wr };
        c.send(ClientMsg::Hello {
            proto_version: version,
            client: ClientKind::Cli,
        })
        .await;
        let first = c.recv().await;
        (c, first)
    }

    pub async fn send(&mut self, m: ClientMsg) {
        write_frame(&mut self.wr, &m).await.unwrap();
    }

    pub async fn recv(&mut self) -> DaemonMsg {
        tokio::time::timeout(Duration::from_secs(5), read_frame(&mut self.rd))
            .await
            .expect("timed out")
            .unwrap()
            .expect("daemon closed")
    }

    pub async fn recv_until(&mut self, mut pred: impl FnMut(&DaemonMsg) -> bool) -> DaemonMsg {
        tokio::time::timeout(Duration::from_secs(8), async {
            loop {
                let m = self.recv().await;
                if pred(&m) {
                    return m;
                }
            }
        })
        .await
        .expect("timed out waiting for message")
    }
}

pub fn shell_spec(name: &str) -> WindowSpec {
    WindowSpec {
        name: Some(name.into()),
        runtime: Runtime::Shell,
        cwd: std::env::temp_dir(),
        worktree_branch: None,
        model: None,
        initial_prompt: None,
    }
}

pub fn git(dir: &std::path::Path, args: &[&std::ffi::OsStr]) {
    let output = std::process::Command::new("git")
        .args([
            "-c",
            "user.name=t",
            "-c",
            "user.email=t@t",
            "-c",
            "commit.gpgsign=false",
            "-c",
            "init.defaultBranch=main",
        ])
        .args(args)
        .current_dir(dir)
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

pub async fn claude_window(d: &TestDaemon, name: &str) -> u32 {
    let manager = d.manager.clone();
    let mut spec = shell_spec(name);
    spec.runtime = Runtime::Claude;
    tokio::task::spawn_blocking(move || {
        manager
            .create(spec, std::env::temp_dir(), None, 80, 24)
            .unwrap()
            .id
    })
    .await
    .unwrap()
}
