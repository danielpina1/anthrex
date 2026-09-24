use daemon::manager::{ManagerConfig, WindowManager};
use daemon::run::driver::RunService;
use daemon::server::{GitWiring, serve};
use proto::{ClientMsg, DaemonMsg, Runtime, WindowSpec};
use std::path::PathBuf;
use std::time::Duration;
use tokio_util::sync::CancellationToken;
use tui::connection::Connection;

/// Starts a daemon listening at exactly `socket`, rather than one of its own
/// choosing, so a test can assert on `reconnect::attempt` against a known-dead path
/// that then comes alive.
async fn start_daemon_at(socket: &PathBuf) -> CancellationToken {
    let listener = tokio::net::UnixListener::bind(socket).unwrap();
    let (manager, mut events) =
        WindowManager::new(ManagerConfig::new(socket.clone(), "/bin/sh".into()));
    let pump = manager.clone();
    tokio::spawn(async move {
        while let Some((id, ev)) = events.recv().await {
            pump.handle_event(id, ev);
        }
    });
    let token = CancellationToken::new();
    let git = GitWiring::new(config::Git::default());
    let data = socket.parent().unwrap().join("data");
    let runs = RunService::for_manager(&manager, data, git.registry.clone());
    runs.spawn(token.clone());
    tokio::spawn(serve(listener, manager, git, runs, token.clone()));
    token
}

async fn start_daemon() -> (tempfile::TempDir, PathBuf, CancellationToken) {
    let dir = tempfile::tempdir().unwrap();
    let socket = dir.path().join("d.sock");
    let token = start_daemon_at(&socket).await;
    (dir, socket, token)
}

async fn recv_until(conn: &mut Connection, mut pred: impl FnMut(&DaemonMsg) -> bool) -> DaemonMsg {
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let m = conn.recv().await.expect("connection closed");
            if pred(&m) {
                return m;
            }
        }
    })
    .await
    .expect("timed out")
}

#[tokio::test]
async fn connects_creates_and_receives_output() {
    let (_dir, socket, _token) = start_daemon().await;
    let mut conn = Connection::connect(&socket).await.unwrap();
    assert!(conn.windows.is_empty());
    assert!(!conn.daemon_version.is_empty());

    let spec = WindowSpec {
        name: Some("w".into()),
        runtime: Runtime::Shell,
        cwd: std::env::temp_dir(),
        worktree_branch: None,
        model: None,
        initial_prompt: None,
    };
    assert!(conn.send(ClientMsg::CreateWindow {
        spec,
        cols: 80,
        rows: 24
    }));
    let DaemonMsg::Created { window_id } =
        recv_until(&mut conn, |m| matches!(m, DaemonMsg::Created { .. })).await
    else {
        unreachable!()
    };
    assert!(conn.send(ClientMsg::Subscribe {
        window_id,
        cols: 80,
        rows: 24
    }));
    recv_until(&mut conn, |m| matches!(m, DaemonMsg::Snapshot { .. })).await;
    assert!(conn.send(ClientMsg::Input {
        window_id,
        bytes: b"echo conn-$((3+3))\n".to_vec()
    }));
    let mut seen = Vec::new();
    recv_until(&mut conn, |m| {
        if let DaemonMsg::Output { bytes, .. } = m {
            seen.extend_from_slice(bytes);
        }
        String::from_utf8_lossy(&seen).contains("conn-6")
    })
    .await;
    assert!(conn.send(ClientMsg::Remove {
        window_id,
        remove_worktree: false,
        force: false
    }));
}

#[tokio::test]
async fn recv_returns_none_after_daemon_shutdown() {
    let (_dir, socket, token) = start_daemon().await;
    let mut conn = Connection::connect(&socket).await.unwrap();
    token.cancel();
    recv_until(&mut conn, |m| matches!(m, DaemonMsg::Bye { .. })).await;
    let after = tokio::time::timeout(Duration::from_secs(5), conn.recv())
        .await
        .expect("timed out");
    assert!(after.is_none());
}

/// C1 (client half): `send` must never suspend the UI loop, and must report a failure
/// rather than wait once the connection is gone.
#[tokio::test]
async fn send_never_suspends_and_reports_a_dead_connection() {
    let (_dir, socket, token) = start_daemon().await;
    let mut conn = Connection::connect(&socket).await.unwrap();

    // Far more messages than the 256-slot outgoing queue holds, with no await in between.
    let started = std::time::Instant::now();
    for _ in 0..5_000 {
        let _ = conn.send(ClientMsg::ListWindows);
    }
    let elapsed = started.elapsed();
    assert!(
        elapsed < Duration::from_millis(500),
        "5000 sends took {elapsed:?}"
    );

    token.cancel();
    recv_until(&mut conn, |m| matches!(m, DaemonMsg::Bye { .. })).await;
    assert!(
        tokio::time::timeout(Duration::from_secs(5), conn.recv())
            .await
            .unwrap()
            .is_none()
    );

    // Once the writer task has noticed the closed socket it drops its receiver, and
    // every later send reports false instead of blocking.
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    while conn.send(ClientMsg::ListWindows) {
        assert!(
            std::time::Instant::now() < deadline,
            "send kept succeeding after the daemon went away"
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

/// Decision 29: a daemon that accepted the connection but never answers the handshake
/// must not hang the client forever either. A plain `UnixListener` that accepts and then
/// writes nothing stands in for that daemon.
#[tokio::test]
async fn connect_times_out_on_a_silent_server() {
    let dir = tempfile::tempdir().unwrap();
    let socket = dir.path().join("silent.sock");
    let listener = tokio::net::UnixListener::bind(&socket).unwrap();
    let _silent_server = tokio::spawn(async move {
        // Accept and hold the connection open for longer than the test can possibly take,
        // writing nothing back — never a `Welcome`.
        let (_stream, _) = listener.accept().await.unwrap();
        tokio::time::sleep(Duration::from_secs(30)).await;
    });

    let started = std::time::Instant::now();
    let err = tokio::time::timeout(Duration::from_secs(7), Connection::connect(&socket))
        .await
        .expect("connect did not return within 7 s")
        .unwrap_err();
    assert!(
        err.to_string().contains("timed out"),
        "unexpected error: {err}"
    );
    assert!(
        started.elapsed() < Duration::from_secs(7),
        "{:?}",
        started.elapsed()
    );
}

#[tokio::test]
async fn connect_fails_cleanly_without_a_daemon() {
    let dir = tempfile::tempdir().unwrap();
    let err = Connection::connect(&dir.path().join("missing.sock"))
        .await
        .unwrap_err();
    assert!(err.to_string().contains("missing.sock"));
}

/// Task M6.11's `reconnect::attempt`, with `daemon_exe: None` — the automatic-retry
/// path (decision 31), which must never spawn a daemon. It only ever connects.
#[tokio::test]
async fn attempt_fails_without_a_daemon_and_succeeds_with_one() {
    let dir = tempfile::tempdir().unwrap();
    let socket = dir.path().join("d.sock");

    let err = tui::reconnect::attempt(&socket, None).await.unwrap_err();
    assert!(!err.to_string().is_empty());

    let _token = start_daemon_at(&socket).await;
    let conn = tui::reconnect::attempt(&socket, None).await.unwrap();
    assert!(conn.windows.is_empty());
}
