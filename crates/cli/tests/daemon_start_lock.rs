//! Decision 31 (B7, task M9.11.28): a client's daemon start waits out a daemon that is
//! still stopping. `ensure_daemon` used to poll the socket for 3 s, and when it found the
//! lifetime lock held (a stopping daemon tearing down) it spawned nothing at all, so a
//! client alone behind a stopping daemon could never succeed, at 3 s or 10 s (stream S
//! review, S28-I1). It now retries the claim while it waits and spawns once it wins.
//!
//! Each reproduction holds the isolated data directory's lifetime lock, as a stopping
//! daemon does, and releases it after `LOCK_WAIT - 1 s`; only `anthrex daemon start`
//! runs behind it. Every daemon here is isolated under a `/tmp` tempdir and stopped with
//! `daemon stop` through that tempdir's own socket (hard rule 1); nothing is signalled.
mod support;

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};
use support::{RunningCommand, isolated_command, tempdir};

/// When the test gives the lock up: one second before `LOCK_WAIT` (imported), the wait the
/// brief sets for this reproduction, and later than the old 3 s socket wait.
const HOLD: Duration = daemon::LOCK_WAIT.saturating_sub(Duration::from_secs(1));

/// `anthrex daemon start`'s own legal worst case: `ENSURE_DAEMON_SOCKET_WAIT` (imported),
/// within which the claim, the spawn and `SPAWN_HANDOFF_GRACE` (0.25 s, private to
/// `tui::spawn`) all fall, plus the grace once more in case the spawn starts at the very
/// end, plus 10 s for the `anthrex` spawn and its exit on a loaded machine.
const START_LIMIT: Duration = tui::spawn::ENSURE_DAEMON_SOCKET_WAIT
    .saturating_add(Duration::from_millis(250))
    .saturating_add(Duration::from_secs(10));

/// `anthrex daemon status`: one connect and handshake (`HANDSHAKE_TIMEOUT`, imported) plus
/// 10 s for the spawn on a loaded machine.
const STATUS_LIMIT: Duration = proto::HANDSHAKE_TIMEOUT.saturating_add(Duration::from_secs(10));

/// `daemon stop`'s legal worst case, as `daemon_stop_stuck.rs` derives it: the handshake
/// (5 s) + `HUP_GRACE` (1 s) + `KILL_GRACE` (3 s) + `wait_released`'s 10 s cap, plus 10 s.
const STOP_LIMIT: Duration = proto::HANDSHAKE_TIMEOUT
    .saturating_add(Duration::from_secs(1 + 3 + 10))
    .saturating_add(Duration::from_secs(10));

/// Stops whatever daemon this test's `daemon start` left on the tempdir's socket, through
/// that socket (`daemon stop` with the same isolated variables). On a failure path the
/// detached daemon may still be on its way to binding (stream S re-review N4), so a
/// missing socket is waited for, at most `ENSURE_DAEMON_SOCKET_WAIT` (what the client
/// itself allows a start), before the guard concludes nothing came up; a passing test
/// finds the socket at once.
struct StopOnDrop<'a>(&'a Path);

impl Drop for StopOnDrop<'_> {
    fn drop(&mut self) {
        let socket = self.0.join("daemon.sock");
        let deadline = Instant::now() + tui::spawn::ENSURE_DAEMON_SOCKET_WAIT;
        while !socket.exists() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(20));
        }
        if socket.exists() {
            let _ = RunningCommand::start(&mut isolated_command(self.0, &["daemon", "stop"]))
                .try_finish(STOP_LIMIT);
        }
    }
}

/// Runs `anthrex` once before the reproduction starts its clock. The first `anthrex` this
/// test process spawns took 2.5-3 s to reach `main` on macOS (measured: `--version`
/// 2.65 s and 2.53 s, the next spawn at once), the first-exec cost of "Measured primitive
/// costs"; paid inside the reproduction it moves `daemon start`'s window past the lock's
/// release, and an old wait passes by accident.
fn warm_up() {
    let warm = std::process::Command::new(support::ANTHREX)
        .arg("--version")
        .output()
        .unwrap();
    assert!(warm.status.success(), "anthrex --version failed");
}

/// Holds `data`'s lifetime lock on a thread of its own and gives it up after `HOLD`. The
/// sleep is the injected stall (the reproduction), not a synchronisation.
fn hold_the_lock(data: &Path) -> std::thread::JoinHandle<()> {
    std::fs::create_dir_all(data).unwrap();
    let lock = daemon::lockfile::try_claim(data)
        .unwrap()
        .expect("a fresh data directory's lock is free");
    std::thread::spawn(move || {
        std::thread::sleep(HOLD);
        drop(lock);
    })
}

/// `anthrex daemon start` behind the held lock, with the stderr probe on: every daemon
/// process it (or its detached child) starts writes one probe line to
/// `data/daemon.stderr.log` at the very top of `lifecycle::run`, before its lock.
fn start_command(dir: &Path) -> RunningCommand {
    RunningCommand::start(
        isolated_command(dir, &["daemon", "start"]).env("ANTHREX_TEST_STDERR_PROBE", "1"),
    )
}

fn assert_started(dir: &Path, start: std::process::Output, started: Instant) {
    assert!(
        start.status.success(),
        "daemon start failed after {:?}: {}\n--- daemon log ---\n{}",
        started.elapsed(),
        String::from_utf8_lossy(&start.stderr),
        std::fs::read_to_string(dir.join("data/daemon.log")).unwrap_or_default()
    );
}

fn assert_running(dir: &Path) {
    let status = RunningCommand::start(&mut isolated_command(dir, &["daemon", "status"]))
        .finish(STATUS_LIMIT);
    let said = String::from_utf8_lossy(&status.stdout);
    assert!(
        status.status.success() && said.starts_with("running: version"),
        "the started daemon did not answer: {said}"
    );
}

/// How many daemon processes ran in `dir`, counted by their probe lines.
fn daemons_started(dir: &Path) -> usize {
    std::fs::read_to_string(dir.join("data/daemon.stderr.log"))
        .unwrap_or_default()
        .matches("anthrex-test-stderr-probe")
        .count()
}

#[test]
fn a_start_behind_a_stopping_daemon_waits_for_it() {
    let dir = tempdir();
    let _stop = StopOnDrop(dir.path());
    warm_up();
    let release = hold_the_lock(&dir.path().join("data"));

    let started = Instant::now();
    let start = start_command(dir.path()).finish(START_LIMIT);
    assert_started(dir.path(), start, started);
    assert_running(dir.path());
    assert_eq!(daemons_started(dir.path()), 1);
    release.join().unwrap();
}

/// Two clients behind the same stopping daemon: both wait, both retry the claim, and the
/// lock decides which one spawns. Exactly one daemon process ever starts (its probe line),
/// and both clients report it running.
#[test]
fn two_starts_behind_a_stopping_daemon_spawn_one_daemon() {
    let dir = tempdir();
    let _stop = StopOnDrop(dir.path());
    warm_up();
    let release = hold_the_lock(&dir.path().join("data"));

    let started = Instant::now();
    let first = start_command(dir.path());
    let second = start_command(dir.path());
    assert_started(dir.path(), first.finish(START_LIMIT), started);
    assert_started(dir.path(), second.finish(START_LIMIT), started);
    assert_running(dir.path());
    assert_eq!(
        daemons_started(dir.path()),
        1,
        "{}",
        std::fs::read_to_string(dir.path().join("data/daemon.stderr.log")).unwrap_or_default()
    );
    release.join().unwrap();
}

/// Stream S re-review N2: a spawn marker whose mtime is in the future (a clock stepped
/// back, or a data directory restored with future times) is not provably younger than
/// `SPAWN_MARKER_FRESH`, so it is stale: it must not hold back a start. Before the fix
/// it read as fresh until the wall clock passed it, and every start failed.
#[test]
fn a_spawn_marker_from_the_future_does_not_block_a_start() {
    let dir = tempdir();
    let _stop = StopOnDrop(dir.path());
    warm_up();
    let data = dir.path().join("data");
    std::fs::create_dir_all(&data).unwrap();
    let marker = std::fs::File::create(data.join(daemon::lifecycle::SPAWN_MARKER)).unwrap();
    marker
        .set_modified(std::time::SystemTime::now() + Duration::from_secs(24 * 3600))
        .unwrap();
    drop(marker);

    let started = Instant::now();
    let start = start_command(dir.path()).finish(START_LIMIT);
    assert_started(dir.path(), start, started);
    assert_running(dir.path());
}

/// Task M9.11.28b: `support::TestDaemon::start` waits for its foreground daemon's socket
/// with the client's own start bound, `ENSURE_DAEMON_SOCKET_WAIT` (it waited a literal 3 s,
/// which failed once under load). The reproduction is the same held lock: the daemon binds
/// about `HOLD` (4 s) in, past the old 3 s and inside the new 10 s. The lock is taken in
/// `configure`, which runs before the spawn, from the data directory the harness gave the
/// daemon; `TestDaemon`'s own drop stops the daemon.
#[test]
fn the_test_daemon_waits_out_a_slow_start() {
    warm_up();
    let mut release = None;
    let daemon = support::TestDaemon::start_configured(&[], |command| {
        let data = command
            .get_envs()
            .find(|(name, _)| *name == "ANTHREX_DATA_DIR")
            .and_then(|(_, value)| value)
            .map(PathBuf::from)
            .expect("the harness isolates the data directory");
        release = Some(hold_the_lock(&data));
    });
    assert!(daemon.socket().exists());
    release.expect("configure ran").join().unwrap();
}
