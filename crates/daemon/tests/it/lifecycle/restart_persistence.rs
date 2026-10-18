//! State survival across a real daemon restart, as opposed to the parent module's own
//! startup/lock/socket lifecycle. A window created and renamed in one daemon lifetime
//! must still be there, exited, after a real stop and start (M6.5's headline test). A
//! restored window's `Runtime::Claude`/`Runtime::Codex` sessions resuming is
//! `lifecycle_resume.rs`, alone in its binary because it sets environment variables. A
//! submodule of `lifecycle.rs` (AGENTS.md rule 8), sharing its `opts`,
//! `shell_spec`, `wait_accepting` and `Client` fixtures via `use super::*`.

use super::*;

/// M6.5's headline lifecycle test (decisions 9, 11, 12 and 14 together): a window created
/// and renamed in one daemon lifetime is listed, exited, in the next one, over a *real*
/// stop and start — not a direct call to `restore` — so decision 11's shutdown order (the
/// persister stopped and awaited, then one final flush, then the socket unlinked) is
/// exactly what is under test, not assumed.
#[tokio::test]
async fn windows_survive_a_daemon_restart_as_exited() {
    let dir = tempfile::tempdir().unwrap();
    let socket = dir.path().join("d.sock");
    let data_dir = dir.path().join("data");

    let handle = tokio::spawn(run(opts(socket.clone(), data_dir.clone())));
    wait_accepting(&socket, Duration::from_secs(5)).await;

    let (mut c, _welcome) = Client::connect(&socket).await;
    c.send(ClientMsg::CreateWindow {
        spec: shell_spec("keep"),
        cols: 80,
        rows: 24,
    })
    .await;
    let window_id = match c
        .recv_until(|m| matches!(m, DaemonMsg::Created { .. }))
        .await
    {
        DaemonMsg::Created { window_id } => window_id,
        other => panic!("expected Created, got {other:?}"),
    };

    c.send(ClientMsg::Rename {
        window_id,
        name: "kept".into(),
    })
    .await;
    c.recv_until(|m| matches!(m, DaemonMsg::Ack { request } if request == "rename"))
        .await;

    c.send(ClientMsg::Shutdown).await;
    c.recv_until(|m| matches!(m, DaemonMsg::Bye { .. })).await;
    tokio::time::timeout(Duration::from_secs(10), handle)
        .await
        .expect("first daemon::run did not return within 10s")
        .expect("the daemon task panicked")
        .expect("first run returned an error");
    assert_lock_released(&data_dir);

    // The socket is gone only after decision 11's whole sequence, so by this point the
    // rename must already be on disk — checked directly, ahead of the second `run`, so a
    // failure here points at the shutdown flush rather than at restore.
    let state_path = data_dir.join("state.json");
    assert!(
        state_path.exists(),
        "state.json must exist after a clean shutdown"
    );
    let (loaded, problems) = daemon::state::load(&state_path);
    assert!(problems.is_empty(), "{problems:?}");
    let record = loaded
        .windows
        .iter()
        .find(|w| w.id == window_id)
        .expect("state.json holds the created window");
    assert_eq!(record.name, "kept");

    // Second daemon, same data dir: the window comes back exited, with decision 14's
    // reason and placeholder screen. It is a successor, so it gets the CLI's own
    // `lock_wait` (`daemon_cmd.rs`), not `ZERO`: under libtest a sibling thread's forked
    // child may still hold a copy of the first daemon's lock description until it execs,
    // and a successor waiting out exactly that is the product's contract.
    // A live first daemon would still be refused at once (`acquire_or_yield` probes the
    // socket on every failed attempt); `assert_lock_released` above pins that the first
    // `run` itself let go.
    let handle2 = tokio::spawn(run(daemon::DaemonOptions {
        lock_wait: daemon::LOCK_WAIT,
        ..opts(socket.clone(), data_dir.clone())
    }));
    wait_accepting(&socket, Duration::from_secs(5)).await;

    let (mut c2, welcome) = Client::connect(&socket).await;
    let windows = match welcome {
        DaemonMsg::Welcome { windows, .. } => windows,
        other => panic!("expected Welcome, got {other:?}"),
    };
    let restored = windows
        .iter()
        .find(|w| w.id == window_id)
        .expect("restored window is listed in Welcome");
    assert_eq!(restored.name, "kept");
    assert_eq!(restored.status, Status::Exited);
    assert_eq!(
        restored.exit.as_ref().map(|e| e.reason.as_str()),
        Some("daemon restarted")
    );

    c2.send(ClientMsg::Subscribe {
        window_id,
        cols: 80,
        rows: 24,
    })
    .await;
    let bytes = match c2
        .recv_until(|m| matches!(m, DaemonMsg::Snapshot { .. }))
        .await
    {
        DaemonMsg::Snapshot { bytes, .. } => bytes,
        other => panic!("expected Snapshot, got {other:?}"),
    };
    let mut parser = vt100::Parser::new(24, 80, 0);
    parser.process(&bytes);
    let screen = parser.screen().contents();
    assert!(
        screen.contains("this window stopped when the daemon restarted"),
        "{screen}"
    );

    c2.send(ClientMsg::Shutdown).await;
    c2.recv_until(|m| matches!(m, DaemonMsg::Bye { .. })).await;
    tokio::time::timeout(Duration::from_secs(10), handle2)
        .await
        .expect("second daemon::run did not return within 10s")
        .expect("the daemon task panicked")
        .expect("second run returned an error");
}
