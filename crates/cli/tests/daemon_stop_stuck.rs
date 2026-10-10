//! Decision 30 (B2, task M9.11.27): a stuck blocking task never keeps the stopped
//! daemon's process alive. `daemon stop` itself already returned before this fix: the
//! lifetime lock is `lifecycle::run`'s local, released when `run` returns, before the
//! runtime's drop waits on blocking tasks (pre-flight P7). What the stuck task did was keep
//! the daemon **process** alive after it, with its threads and whatever the task held, so
//! this test's RED is the process assertion, not `daemon stop`'s exit.
//!
//! `ANTHREX_TEST_STALL_BLOCKING` is set on the daemon's own `Command` only, so this needs
//! no binary of its own. The daemon is this test's own child: it is reaped by `try_wait`
//! (a zombie would still answer `kill(pid, 0)`), and on any failure killed by its exact pid
//! through `Child::kill`, nothing else.
mod support;

use std::process::{Child, Stdio};
use std::time::{Duration, Instant};
use support::{RunningCommand, isolated_command, tempdir};

/// `anthrex daemon stop`'s own legal worst case, as `scripts/pty-smoke.py`'s
/// `DAEMON_STOP_CMD_TIMEOUT` derives it: `HANDSHAKE_TIMEOUT` (5 s, imported) for the
/// connect, then the daemon's shutdown, whose kills are `HUP_GRACE` (1 s) + `KILL_GRACE`
/// (3 s, `daemon::process`, private to the daemon crate so copied here), then
/// `wait_released`'s own 10 s cap (`daemon_cmd.rs`). 19 s; plus 10 s for the `anthrex`
/// spawn and the daemon's teardown on a loaded machine.
const STOP_LIMIT: Duration = proto::HANDSHAKE_TIMEOUT
    .saturating_add(Duration::from_secs(1 + 3 + 10))
    .saturating_add(Duration::from_secs(10));

/// From `daemon stop`'s return (the lock is free, so `run` has returned) to the process's
/// exit: `BLOCKING_SHUTDOWN_GRACE` (2 s, `cli/src/daemon_cmd.rs`, a private constant of the
/// binary, so copied here) for the runtime's `shutdown_timeout`, plus 2 s for the
/// process's exit and the reap (decision 30's own bound). Before the fix the stuck task
/// holds the process for its whole 3600 s.
const EXIT_LIMIT: Duration = Duration::from_secs(2 + 2);

/// Kills this test's own daemon child by its exact pid if the test ends with it alive.
struct KillOnDrop(Child);

impl Drop for KillOnDrop {
    fn drop(&mut self) {
        if self.0.try_wait().ok().flatten().is_none() {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }
}

#[test]
fn daemon_stop_returns_with_a_blocking_task_stuck() {
    let dir = tempdir();
    let log = std::fs::File::create(dir.path().join("daemon.log")).unwrap();
    let child = isolated_command(dir.path(), &["daemon", "start", "--foreground"])
        .env("ANTHREX_TEST_STALL_BLOCKING", "1")
        .stdin(Stdio::null())
        .stdout(log.try_clone().unwrap())
        .stderr(log)
        .spawn()
        .unwrap();
    let mut daemon = KillOnDrop(child);

    // The socket appears once the daemon has bound; the same bound the client's own
    // start waits (`ENSURE_DAEMON_SOCKET_WAIT`, decision 31).
    let socket = dir.path().join("daemon.sock");
    let deadline = Instant::now() + tui::spawn::ENSURE_DAEMON_SOCKET_WAIT;
    while !socket.exists() {
        assert!(
            daemon.0.try_wait().unwrap().is_none(),
            "the daemon exited before binding: {}",
            std::fs::read_to_string(dir.path().join("daemon.log")).unwrap_or_default()
        );
        assert!(
            Instant::now() < deadline,
            "the daemon never bound its socket: {}",
            std::fs::read_to_string(dir.path().join("daemon.log")).unwrap_or_default()
        );
        std::thread::sleep(Duration::from_millis(10));
    }
    let pid: u32 = std::fs::read_to_string(dir.path().join("data/daemon.pid"))
        .unwrap()
        .trim()
        .parse()
        .unwrap();
    assert_eq!(pid, daemon.0.id(), "the pid file names another process");

    let started = Instant::now();
    let stop = RunningCommand::start(&mut isolated_command(dir.path(), &["daemon", "stop"]))
        .finish(STOP_LIMIT);
    assert!(
        stop.status.success(),
        "daemon stop failed after {:?}: {}",
        started.elapsed(),
        String::from_utf8_lossy(&stop.stderr)
    );

    let stopped = Instant::now();
    let deadline = stopped + EXIT_LIMIT;
    while daemon.0.try_wait().unwrap().is_none() {
        assert!(
            Instant::now() < deadline,
            "the daemon (pid {pid}) was still running {:?} after `daemon stop` returned",
            stopped.elapsed()
        );
        std::thread::sleep(Duration::from_millis(10));
    }
}
