//! End-to-end tests for `daemon::run` itself — not `server::serve` in isolation, the way
//! `tests/server.rs`'s harness starts things, but the whole startup and shutdown sequence:
//! the lifetime lock, the socket, the pid file. Each test drives a real `daemon::run` on
//! its own temporary socket and data directory, and talks to it with raw frames, exactly
//! like `tests/server.rs` does — this file just cannot reuse `tests/support`, because that
//! harness starts `server::serve` directly and never exercises `run`'s own setup and
//! teardown, which is what these tests are about.

use daemon::lockfile::DaemonLock;
use daemon::{DaemonOptions, run};
use proto::{ClientKind, ClientMsg, DaemonMsg, PROTO_VERSION, Runtime, WindowSpec};
use proto::{read_frame, write_frame};
use std::path::{Path, PathBuf};
use std::time::Duration;
use tokio::net::UnixStream;
use tokio::net::unix::{OwnedReadHalf, OwnedWriteHalf};

/// A `DaemonOptions` for a fresh, isolated socket and data directory. `lock_wait` is
/// `Duration::ZERO` throughout this file: these tests want a locked-out daemon to fail
/// fast, not to sit in `acquire`'s retry loop.
fn opts(socket: PathBuf, data_dir: PathBuf) -> DaemonOptions {
    DaemonOptions {
        socket_path: socket,
        // A config path that does not exist: `run` does not read it in this milestone's
        // task, but the field must be filled in regardless.
        config_path: data_dir.join("config.toml"),
        data_dir,
        lock_wait: Duration::ZERO,
    }
}

fn shell_spec(name: &str) -> WindowSpec {
    WindowSpec {
        name: Some(name.into()),
        runtime: Runtime::Shell,
        cwd: std::env::temp_dir(),
        worktree_branch: None,
        model: None,
        initial_prompt: None,
    }
}

async fn wait_for_path(path: &Path, timeout: Duration) {
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

struct Client {
    rd: OwnedReadHalf,
    wr: OwnedWriteHalf,
}

impl Client {
    async fn connect(socket: &Path) -> (Self, DaemonMsg) {
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

    async fn send(&mut self, m: ClientMsg) {
        write_frame(&mut self.wr, &m).await.unwrap();
    }

    async fn recv(&mut self) -> DaemonMsg {
        tokio::time::timeout(Duration::from_secs(5), read_frame(&mut self.rd))
            .await
            .expect("timed out waiting for a frame")
            .unwrap()
            .expect("daemon closed the connection")
    }

    async fn recv_until(&mut self, mut pred: impl FnMut(&DaemonMsg) -> bool) -> DaemonMsg {
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

/// The one-line-mutation check this test aims at: an implementation that skipped the
/// lock, or that unlinked the socket unconditionally instead of by inode, would still
/// pass a version of this test that only checked `Ok(())`. So it also checks every piece
/// of cleanup decision 24 and decision 26 promise: the socket and pid file are gone, and
/// the lock is free enough for a fresh `DaemonLock::acquire` to succeed at once.
#[tokio::test]
async fn run_starts_serves_and_stops_cleanly() {
    let dir = tempfile::tempdir().unwrap();
    let socket = dir.path().join("d.sock");
    let data_dir = dir.path().join("data");
    let handle = tokio::spawn(run(opts(socket.clone(), data_dir.clone())));
    wait_for_path(&socket, Duration::from_secs(5)).await;

    let (mut c, welcome) = Client::connect(&socket).await;
    match welcome {
        DaemonMsg::Welcome { windows, .. } => assert!(windows.is_empty(), "{windows:?}"),
        other => panic!("expected Welcome, got {other:?}"),
    }

    c.send(ClientMsg::CreateWindow {
        spec: shell_spec("w"),
        cols: 80,
        rows: 24,
    })
    .await;
    c.recv_until(|m| matches!(m, DaemonMsg::Created { .. }))
        .await;

    c.send(ClientMsg::Shutdown).await;
    c.recv_until(|m| matches!(m, DaemonMsg::Bye { .. })).await;

    let result = tokio::time::timeout(Duration::from_secs(10), handle)
        .await
        .expect("daemon::run did not return within 10s")
        .expect("the daemon task panicked");
    assert!(result.is_ok(), "run returned an error: {result:?}");

    assert!(!socket.exists(), "the socket file was not removed");
    assert!(
        !data_dir.join("daemon.pid").exists(),
        "daemon.pid was not removed"
    );
    let _lock = DaemonLock::acquire(&data_dir, Duration::ZERO)
        .expect("the lock must be free once run has returned");
}

/// The adversarial case decision 24 exists for: a second daemon must never be able to
/// share the first's data directory, must never touch a socket of its own before the
/// lock check fails, and must never disturb the daemon that is still holding it.
#[tokio::test]
async fn second_daemon_with_the_same_data_dir_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    let socket1 = dir.path().join("d1.sock");
    let socket2 = dir.path().join("d2.sock");
    let data_dir = dir.path().join("data");
    let handle1 = tokio::spawn(run(opts(socket1.clone(), data_dir.clone())));
    wait_for_path(&socket1, Duration::from_secs(5)).await;

    let err = run(opts(socket2.clone(), data_dir.clone()))
        .await
        .expect_err("a second daemon on the same data dir must be refused");
    assert!(err.to_string().contains("another anthrex daemon"), "{err}");
    assert!(
        !socket2.exists(),
        "the refused daemon must never create its own socket"
    );

    // The first daemon must be completely unaffected by the second's failed attempt.
    let (mut c, welcome) = Client::connect(&socket1).await;
    assert!(matches!(welcome, DaemonMsg::Welcome { .. }), "{welcome:?}");
    c.send(ClientMsg::Shutdown).await;
    c.recv_until(|m| matches!(m, DaemonMsg::Bye { .. })).await;
    tokio::time::timeout(Duration::from_secs(10), handle1)
        .await
        .expect("daemon::run did not return within 10s")
        .expect("the daemon task panicked")
        .expect("run returned an error");
}

/// Decision 26's other adversarial case: a daemon mid-shutdown must unlink its socket
/// path only if the file there is still the one it bound — never a replacement that
/// another daemon (here, a plain listener standing in for one) bound at the same path in
/// the meantime.
#[tokio::test]
async fn a_stopping_daemon_leaves_a_replacement_socket_alone() {
    let dir = tempfile::tempdir().unwrap();
    let socket = dir.path().join("d.sock");
    let data_dir = dir.path().join("data");
    let handle = tokio::spawn(run(opts(socket.clone(), data_dir.clone())));
    wait_for_path(&socket, Duration::from_secs(5)).await;

    // Open the connection that will carry `Shutdown` *before* the swap below, exactly as
    // decision 26 describes: an already-open connection outlives the path it was opened
    // through, so the daemon can still be asked to stop after its socket file has been
    // replaced out from under it.
    let (mut c, _welcome) = Client::connect(&socket).await;

    std::fs::remove_file(&socket).unwrap();
    let replacement = std::os::unix::net::UnixListener::bind(&socket).unwrap();
    replacement.set_nonblocking(true).unwrap();

    c.send(ClientMsg::Shutdown).await;
    c.recv_until(|m| matches!(m, DaemonMsg::Bye { .. })).await;

    let result = tokio::time::timeout(Duration::from_secs(10), handle)
        .await
        .expect("daemon::run did not return within 10s")
        .expect("the daemon task panicked");
    assert!(result.is_ok(), "run returned an error: {result:?}");

    assert!(
        socket.exists(),
        "the stopping daemon must leave the replacement socket file alone"
    );
    let probe = std::os::unix::net::UnixStream::connect(&socket)
        .expect("the replacement listener's path must still be connectable");
    let deadline = std::time::Instant::now() + Duration::from_secs(2);
    let accepted = loop {
        match replacement.accept() {
            Ok(_) => break true,
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                if std::time::Instant::now() >= deadline {
                    break false;
                }
                std::thread::sleep(Duration::from_millis(10));
            }
            Err(_) => break false,
        }
    };
    assert!(
        accepted,
        "the replacement listener must still be accepting connections"
    );
    drop(probe);
}
