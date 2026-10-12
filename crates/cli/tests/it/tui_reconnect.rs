//! `drives_a_real_reconnect_twice`: the client's real `ConnectionDriver` loses its link
//! to a real in-process daemon, recovers, and loses it again. Moved from
//! `tui/src/reconnect_tests.rs` (milestone 9.12 decision 11), which keeps the driver's
//! tests that need no daemon.

use daemon::manager::{ManagerConfig, WindowManager};
use daemon::run::driver::RunService;
use daemon::server::{GitWiring, serve};
use std::path::Path;
use std::time::{Duration, Instant};
use tokio_util::sync::CancellationToken;
use tui::app::{App, Link};
use tui::connection::Connection;
use tui::connection_driver::ConnectionDriver;
use tui::settings::UiSettings;

/// Starts a daemon listening at exactly `socket`, so a dropped connection can be
/// reconnected to a fresh daemon at the same path.
async fn start_daemon_at(socket: &Path) -> CancellationToken {
    let listener = tokio::net::UnixListener::bind(socket).unwrap();
    let (manager, mut events) = WindowManager::new(ManagerConfig::for_tests(
        socket.to_path_buf(),
        "/bin/sh".into(),
    ));
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

#[tokio::test]
async fn drives_a_real_reconnect_twice() {
    let dir = tempfile::tempdir().unwrap();
    let socket = dir.path().join("d.sock");
    let mut token = start_daemon_at(&socket).await;

    let first = Connection::connect(&socket).await.unwrap();
    let mut app = App::new(first.windows.clone(), "/tmp".into(), UiSettings::default());
    let mut conn = Some(first);
    let mut driver = ConnectionDriver::new();

    for cycle in 0..2u8 {
        assert!(app.connected(), "cycle {cycle}: must start Connected");

        // Kill the daemon this connection is talking to; automatic attempts (decision
        // 31) never start one (`daemon_exe: None`), so nothing reconnects until a new
        // daemon exists at `socket` again.
        let cycle_started = Instant::now();
        token.cancel();

        let mut restarted = false;
        // `Bye` arrives (and is processed by a `step`) before the socket actually
        // closes, and `Bye` alone must not change `app.link` (the Critical this
        // task's report covers) — so the loop cannot just break on the first
        // `app.connected()` it sees; it must first see the link actually leave
        // `Connected`, or a `Bye`-only step would look indistinguishable from a
        // reconnect that never had to happen at all.
        let mut disconnect_seen = false;
        tokio::time::timeout(Duration::from_secs(45), async {
            loop {
                let _ = driver.step(&mut conn, &mut app, &socket).await;
                eprintln!(
                    "cycle {cycle} +{:?}: link={:?} restarted={restarted}",
                    cycle_started.elapsed(),
                    app.link
                );
                if !app.connected() {
                    disconnect_seen = true;
                }
                // Bring the daemon back only *after* the drop has actually been
                // observed — starting it immediately would race whether the very
                // first automatic attempt sees it, which is not what this test is
                // pinning down; the retry schedule already has its own
                // synthetic-time coverage in `reconnect::tests`.
                if !restarted && disconnect_seen {
                    // A cancelled `serve` leaves the socket file itself on disk (Unix
                    // domain sockets are unlinked by path, not by the listening fd
                    // closing), so a fresh `bind` at the same path needs it removed
                    // first, the same way a real daemon's own startup path does.
                    let _ = std::fs::remove_file(&socket);
                    token = start_daemon_at(&socket).await;
                    restarted = true;
                }
                if disconnect_seen && app.connected() {
                    break;
                }
            }
        })
        .await
        .unwrap_or_else(|_| panic!("cycle {cycle}: did not reconnect within 45s"));

        assert!(
            disconnect_seen,
            "cycle {cycle}: the link must actually have dropped"
        );

        assert!(
            matches!(app.link, Link::Connected),
            "cycle {cycle}: expected Link::Connected, got {:?}",
            app.link
        );
        assert!(
            conn.is_some(),
            "cycle {cycle}: a live connection must be installed"
        );
    }
}
