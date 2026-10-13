//! Drives the real `ConnectionDriver` — the same connection-lifecycle state
//! `event_loop` selects on — with no terminal at all: superseding an in-flight attempt
//! and parking when every branch is disabled.
//!
//! `drives_a_real_reconnect_twice`, which loses the link to a real in-process daemon,
//! recovers and loses it again, is in `cli/tests/it/tui_reconnect.rs` since milestone
//! 9.12 (decision 11): the client no longer depends on the daemon. `event_loop` itself
//! cannot be driven directly in a test (`EventStream::new()` needs a real terminal),
//! which is why `ConnectionDriver` is its own struct, public for that test.

use super::*;
use std::time::Instant;

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
    supersedes_an_in_flight_attempt(Duration::ZERO).await;
}

/// How long the press's attempt may take to reach the stand-in's first line: a
/// `spawn_blocking` hop, a claim, a fork and a warm exec, tens of milliseconds idle. A
/// fresh file's first exec is not in it: macOS assesses it, ~0.44 s alone and serialised
/// to seconds across concurrent first execs (`docs/timing-budgets.md`, "First exec of a
/// freshly written executable"), so [`stand_in`] pays it before the press.
const MARKER_WAIT: Duration = Duration::from_secs(3);

/// The full-suite flake (the marker never came within [`MARKER_WAIT`], "daemon_exe was
/// lost"): the stand-in's first exec stalls past the wait, as macOS's first-exec
/// assessment did under a loaded run. Paid before the press, it costs the wait nothing.
#[tokio::test]
async fn a_slow_first_exec_of_the_stand_in_is_paid_before_the_press() {
    supersedes_an_in_flight_attempt(MARKER_WAIT + Duration::from_secs(1)).await;
}

/// Set only on [`stand_in`]'s warm-up run: the script exits before recording anything.
const WARM_UP_VAR: &str = "ANTHREX_TEST_WARM_UP";

/// Stands in for the real `anthrex` binary: records that it was launched as
/// `daemon start --foreground` in `marker`, which is the one observable proof that a
/// press's `daemon_exe` reached a real attempt, then exits without binding the socket.
/// The script is run once before this returns, so the fresh file's first-exec
/// assessment is paid here, never inside [`MARKER_WAIT`]. `first_exec_stall` injects
/// that cost: the script's first exec sleeps it.
fn stand_in(
    dir: &std::path::Path,
    marker: &std::path::Path,
    first_exec_stall: Duration,
) -> PathBuf {
    let script = dir.join("fake_daemon.sh");
    let stall = if first_exec_stall.is_zero() {
        String::new()
    } else {
        format!(
            "mkdir {:?} 2>/dev/null && sleep {}\n",
            dir.join("first-exec"),
            first_exec_stall.as_secs_f64()
        )
    };
    testexec::write_executable(
        &script,
        format!(
            "#!/bin/sh\n{stall}[ -n \"${WARM_UP_VAR}\" ] && exit 0\necho invoked >> {marker:?}\nexit 0\n"
        ),
    );
    let warm = std::process::Command::new(&script)
        .env(WARM_UP_VAR, "1")
        .output()
        .expect("the stand-in's warm-up");
    assert!(warm.status.success(), "the stand-in's warm-up: {warm:?}");
    assert!(!marker.exists(), "the warm-up recorded an invocation");
    script
}

async fn supersedes_an_in_flight_attempt(first_exec_stall: Duration) {
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

    // The stand-in never binds the socket, so `ensure_daemon`'s own poll just runs out
    // its `ENSURE_DAEMON_SOCKET_WAIT` budget harmlessly (10 s since milestone 9.11's
    // decision 31, 3 s before).
    let marker = dir.path().join("started.log");
    let script = stand_in(dir.path(), &marker, first_exec_stall);

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
    let marker_deadline = Instant::now() + MARKER_WAIT;
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

    // The attempt is ended before the isolation variable goes (stream S re-review N1):
    // nothing of it may run with `ANTHREX_DATA_DIR` unset, or it would resolve the user's
    // real data directory. The stub never binds the socket, so the attempt would poll for
    // `ENSURE_DAEMON_SOCKET_WAIT` (10 s); it has already spawned (the marker above), so it
    // never claims or spawns again and is only polling the socket, which an abort ends at
    // its next yield. Awaiting the handle afterwards makes sure it has.
    if let Some(inflight) = driver.inflight.take() {
        inflight.abort();
        let _ = inflight.await;
    }

    // SAFETY: see above.
    unsafe { std::env::remove_var("ANTHREX_DATA_DIR") };
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
