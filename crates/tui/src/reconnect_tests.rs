//! Drives the real `ConnectionDriver` — the same connection-lifecycle state
//! `event_loop` selects on — against a real daemon socket, with no terminal at all.
//!
//! Every other test that exercises decisions 30-35 does so through `App`'s pure
//! functions directly (`app_tests/reconnect.rs`) or through `reconnect::attempt` in
//! isolation (`tests/connection.rs`). Neither one proves the actual `tokio::select!`
//! wiring in this file's `ConnectionDriver::step` — the guards, the `async {}`
//! deferral around each `Option`-shaped branch, the "unless one is in flight" check —
//! ever runs correctly end to end. `event_loop` itself cannot be driven directly in a
//! test (`EventStream::new()` needs a real terminal), which is exactly why
//! `ConnectionDriver` was split out as its own struct in the first place: this is the
//! piece that can be tested for real, and this file is that test.
//!
//! `drives_a_real_reconnect_twice` is this task's answer to "what does this change
//! let a user do that nothing in the suite has ever done": lose the link, recover,
//! and lose it *again* — the second use `docs/timing-budgets.md`'s standing rules
//! warn a green suite can otherwise never have exercised.

use super::*;
use crate::app::Link;
use daemon::manager::{ManagerConfig, WindowManager};
use daemon::server::serve;
use tokio_util::sync::CancellationToken;

/// Starts a daemon listening at exactly `socket`, so a dropped connection can be
/// reconnected to a fresh daemon at the same path.
async fn start_daemon_at(socket: &Path) -> CancellationToken {
    let listener = tokio::net::UnixListener::bind(socket).unwrap();
    let (manager, mut events) =
        WindowManager::new(ManagerConfig::new(socket.to_path_buf(), "/bin/sh".into()));
    let pump = manager.clone();
    tokio::spawn(async move {
        while let Some((id, ev)) = events.recv().await {
            pump.handle_event(id, ev);
        }
    });
    let token = CancellationToken::new();
    tokio::spawn(serve(listener, manager, true, token.clone()));
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
