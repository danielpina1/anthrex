//! The fixtures `lifecycle.rs` and `lifecycle_resume.rs` share: a `DaemonOptions` for an
//! isolated socket and data directory, a shell window spec, a wait for a path, and a
//! minimal raw-frame client. Included in each with `#[path]`.
#![allow(dead_code)]

use daemon::DaemonOptions;
use proto::{ClientKind, ClientMsg, DaemonMsg, PROTO_VERSION, Runtime, WindowSpec};
use proto::{read_frame, write_frame};
use std::path::{Path, PathBuf};
use std::time::Duration;
use tokio::net::UnixStream;
use tokio::net::unix::{OwnedReadHalf, OwnedWriteHalf};

/// A `DaemonOptions` for a fresh, isolated socket and data directory. `lock_wait` is
/// `Duration::ZERO` throughout this file: these tests want a locked-out daemon to fail
/// fast, not to sit in `acquire`'s retry loop.
pub fn opts(socket: PathBuf, data_dir: PathBuf) -> DaemonOptions {
    DaemonOptions {
        socket_path: socket,
        // A config path that does not exist: `run` does not read it in this milestone's
        // task, but the field must be filled in regardless.
        config_path: data_dir.join("config.toml"),
        data_dir,
        lock_wait: Duration::ZERO,
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

pub async fn wait_for_path(path: &Path, timeout: Duration) {
    tokio::time::timeout(timeout, async {
        loop {
            if path.exists() {
                return;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .unwrap_or_else(|_| panic!("timed out waiting for {} to appear", path.display()));
}

pub struct Client {
    rd: OwnedReadHalf,
    wr: OwnedWriteHalf,
}

impl Client {
    pub async fn connect(socket: &Path) -> (Self, DaemonMsg) {
        let stream = UnixStream::connect(socket)
            .await
            .unwrap_or_else(|e| panic!("connecting to {}: {e}", socket.display()));
        let (rd, wr) = stream.into_split();
        let mut c = Client { rd, wr };
        c.send(ClientMsg::Hello {
            proto_version: PROTO_VERSION,
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
            .expect("timed out waiting for a frame")
            .unwrap()
            .expect("daemon closed the connection")
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
        .expect("timed out waiting for the expected message")
    }
}
