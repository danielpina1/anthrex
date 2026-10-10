//! Decision 31 (B7, task M9.11.28): a client's daemon start waits out the daemon's own
//! lock wait. `ensure_daemon` used to poll the socket for 3 s while a daemon may wait
//! `LOCK_WAIT` (5 s) for its lifetime lock before it binds, so a client said "did not
//! start" while the daemon came up a moment later.
//!
//! The reproduction holds the isolated data directory's lifetime lock, as a daemon still
//! stopping does, and releases it after `LOCK_WAIT - 1 s`. `ensure_daemon` sees the lock
//! held and spawns nothing (fix wave 10: a contended claim means somebody else is starting
//! a daemon), so the daemon it waits for is the one queued behind the lock: a `daemon
//! start --foreground` this test starts first, as another client's start would have. It
//! binds once the lock is free, about 4 s in: past the old 3 s, inside the new wait.
//!
//! Every daemon here is isolated under a `/tmp` tempdir and is this test's own child,
//! stopped with `daemon stop` and killed by its exact pid only if still alive at the end.
mod support;

use std::path::{Path, PathBuf};
use std::process::{Child, Stdio};
use std::time::{Duration, Instant};
use support::{RunningCommand, isolated_command, tempdir};

/// When the test gives the lock up: one second before a daemon queued behind it would
/// give up on it itself (`LOCK_WAIT`, imported), so that daemon binds, and later than the
/// old 3 s socket wait.
const HOLD: Duration = daemon::LOCK_WAIT.saturating_sub(Duration::from_secs(1));

/// `anthrex daemon start`'s own legal worst case: `SPAWN_HANDOFF_GRACE` (0.25 s, private to
/// `tui::spawn`, unpaid here since the claim is contended) + `ENSURE_DAEMON_SOCKET_WAIT`
/// (imported), plus 10 s for the `anthrex` spawn and its exit on a loaded machine.
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

/// Stops this test's daemon through its own socket, then kills it by its exact pid if it
/// is somehow still alive (hard rule 1: nothing of this test's survives it).
struct Daemon<'a> {
    dir: &'a Path,
    child: Child,
}

impl Drop for Daemon<'_> {
    fn drop(&mut self) {
        if self.child.try_wait().ok().flatten().is_none() {
            let _ = RunningCommand::start(&mut isolated_command(self.dir, &["daemon", "stop"]))
                .try_finish(STOP_LIMIT);
        }
        if self.child.try_wait().ok().flatten().is_none() {
            let _ = self.child.kill();
            let _ = self.child.wait();
        }
    }
}

/// Runs `anthrex` once before the reproduction starts its clock. The first `anthrex` this
/// test process spawns took 2.5-3 s to reach `main` on macOS (measured: `--version`
/// 2.65 s and 2.53 s, the next spawn at once), the first-exec cost of "Measured primitive
/// costs"; paid inside the reproduction it moves `daemon start`'s 3 s window past the
/// lock's release, and the old wait passes by accident.
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

#[test]
fn a_start_behind_a_stopping_daemon_waits_for_it() {
    let dir = tempdir();
    warm_up();
    let release = hold_the_lock(&dir.path().join("data"));

    let log = std::fs::File::create(dir.path().join("daemon.log")).unwrap();
    let child = isolated_command(dir.path(), &["daemon", "start", "--foreground"])
        .stdin(Stdio::null())
        .stdout(log.try_clone().unwrap())
        .stderr(log)
        .spawn()
        .unwrap();
    let _daemon = Daemon {
        dir: dir.path(),
        child,
    };

    let started = Instant::now();
    let start = RunningCommand::start(&mut isolated_command(dir.path(), &["daemon", "start"]))
        .finish(START_LIMIT);
    assert!(
        start.status.success(),
        "daemon start failed after {:?}: {}\n--- daemon log ---\n{}",
        started.elapsed(),
        String::from_utf8_lossy(&start.stderr),
        std::fs::read_to_string(dir.path().join("daemon.log")).unwrap_or_default()
    );

    let status = RunningCommand::start(&mut isolated_command(dir.path(), &["daemon", "status"]))
        .finish(STATUS_LIMIT);
    let said = String::from_utf8_lossy(&status.stdout);
    assert!(
        status.status.success() && said.starts_with("running: version"),
        "the started daemon did not answer: {said}"
    );
    release.join().unwrap();
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
