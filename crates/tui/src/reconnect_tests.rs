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

/// Fix wave 10, item 2: `C-b r` while an automatic retry (`daemon_exe: None`) is
/// already in flight used to just reset the schedule and leave that stale attempt
/// running, silently dropping this press's "start the daemon if it is not running"
/// intent for the whole cycle. `reconnect_now` must instead supersede it: abort the
/// stale attempt and spawn a fresh one that actually carries `daemon_exe`.
///
/// Deterministic, not timing luck: the stale attempt is controlled by a `oneshot`
/// receiver that is never sent to, so it is provably still in flight (not finished on
/// its own) at the moment `reconnect_now` is called — no race against when it happens
/// to complete.
#[tokio::test]
async fn reconnect_now_supersedes_an_in_flight_automatic_attempt() {
    let _env_guard = crate::ENV_LOCK.lock().await;
    let dir = tempfile::tempdir().unwrap();
    let data_dir = dir.path().join("data");
    // Nothing ever listens here; `reconnect::attempt`'s own `Connection::connect` must
    // fail so the only way `daemon_exe` can be observed is via `ensure_daemon` actually
    // trying to start it.
    let socket = dir.path().join("d.sock");

    // SAFETY: serialized by `ENV_LOCK`; nothing else in this process reads the
    // variable concurrently.
    unsafe { std::env::set_var("ANTHREX_DATA_DIR", &data_dir) };

    // Stands in for the real `anthrex` binary: records that it was launched as
    // `daemon start --foreground`, which is the one observable proof that this
    // press's `daemon_exe` reached a real attempt, then exits without binding the
    // socket (so `ensure_daemon`'s own poll just runs out its 3s budget harmlessly).
    let marker = dir.path().join("started.log");
    let script = dir.path().join("fake_daemon.sh");
    std::fs::write(
        &script,
        format!("#!/bin/sh\necho invoked >> {:?}\nexit 0\n", marker),
    )
    .unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
    }

    let mut driver = ConnectionDriver::new();

    // A stale automatic attempt, already in flight, that never resolves on its own —
    // the `_release_tx` sender is held but never sent on, so `release_rx.await` blocks
    // for as long as the task survives.
    let (_release_tx, release_rx) = tokio::sync::oneshot::channel::<()>();
    let stale = tokio::spawn(async move {
        let _ = release_rx.await;
        unreachable!("the stale attempt must be aborted, never run to completion");
    });
    let stale_abort = stale.abort_handle();
    driver.inflight = Some(stale);

    // The manual press.
    driver.reconnect_now(&socket, Some(script.clone()));

    // The stale attempt must have been superseded (aborted), not left running to
    // decide this cycle on its own. `abort()` takes effect at its next yield point
    // (AGENTS.md's own facts-learned-the-hard-way), so poll for it rather than assume
    // one `yield_now` is enough.
    let deadline = Instant::now() + Duration::from_secs(2);
    loop {
        if stale_abort.is_finished() {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "the stale in-flight attempt was never aborted"
        );
        tokio::task::yield_now().await;
    }

    // The fresh attempt this press spawned must actually try to start the daemon —
    // wait for its spawned child to prove it, not just for `reconnect_now` to return
    // (which only spawns the attempt task; it does not wait for it).
    let marker_deadline = Instant::now() + Duration::from_secs(3);
    loop {
        if marker.exists() {
            break;
        }
        assert!(
            Instant::now() < marker_deadline,
            "the manual press's own attempt never tried to start the daemon — \
             daemon_exe was lost"
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }

    // SAFETY: see above.
    unsafe { std::env::remove_var("ANTHREX_DATA_DIR") };

    // Let the fresh attempt finish on its own (the stub never binds the socket, so it
    // eventually times out) so nothing of this test's outlives it.
    if let Some(inflight) = driver.inflight.take() {
        let _ = tokio::time::timeout(Duration::from_secs(5), inflight).await;
    }
}

/// Decision 31's give-up path (`on_attempt_finished`'s `Err` arm when `after_failure`
/// returns false) leaves `conn = None`, `inflight = None` and `schedule = None` all at
/// once — every guard in `step`'s `select!` false simultaneously. Before this file's
/// fix, `tokio::select!` with no `else` branch and every arm disabled panics with
/// "all branches are disabled and there is no else branch" the instant `step` is
/// polled in that state, which is exactly what happened live 30s after a daemon
/// disappeared (the status bar showed `DISCONNECTED  C-b r to reconnect` for one frame
/// and then the process died, rc 101). `step` must instead park forever — never
/// resolving on its own — so the outer `event_loop` keeps servicing keyboard/tick
/// events until something (a manual `C-b r`) re-arms the driver.
#[tokio::test]
async fn step_parks_instead_of_panicking_when_every_branch_is_disabled() {
    let mut app = App::new(vec![], "/tmp".into(), UiSettings::default());
    let mut conn: Option<Connection> = None;
    let mut driver = ConnectionDriver::new();
    assert!(driver.schedule.is_none());
    assert!(driver.inflight.is_none());
    assert!(conn.is_none());

    let socket = PathBuf::from("/tmp/anthrex-give-up-test.sock");
    let result = tokio::time::timeout(
        Duration::from_millis(200),
        driver.step(&mut conn, &mut app, &socket),
    )
    .await;
    assert!(
        result.is_err(),
        "step must park rather than resolve (or panic) when every branch is disabled"
    );
}
