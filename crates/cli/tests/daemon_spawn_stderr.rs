mod support;

use std::path::Path;
use std::time::{Duration, Instant};
use support::{isolated_command, tempdir};

/// Stops the daemon this test started, even if an assertion panics first — a bare
/// `assert!` between start and stop would otherwise leak a detached daemon process
/// (AGENTS.md hard rule 1: nothing of this test's may still be running when it exits).
/// Declared *after* `dir` in the test so it drops (and so stops the daemon, while the
/// socket path inside `dir` still exists) before `dir` itself is removed.
struct StopOnDrop<'a> {
    dir: &'a Path,
}

impl Drop for StopOnDrop<'_> {
    fn drop(&mut self) {
        let _ = isolated_command(self.dir, &["daemon", "stop"]).status();
    }
}

/// A production daemon almost always starts *detached* — `anthrex`'s auto-start path
/// (`spawn::ensure_daemon` -> `spawn::spawn_detached` in `crates/cli/src/spawn.rs`) is
/// what every ordinary `anthrex ls`, `anthrex attach`, etc. goes through, not
/// `--foreground` (that flag exists for exactly this test suite and for `daemon start
/// --foreground` run by hand). `logfile::RotatingFile` can only report a failed
/// rotation via `eprintln!` — nothing else on that path can surface it, per its own
/// module doc — so that diagnostic reaches a real user only if the detached child's
/// stderr goes somewhere other than `/dev/null`. Before this test's fix,
/// `spawn_detached` set `.stderr(Stdio::null())` unconditionally, so the message was
/// lost for every detached daemon, which is nearly all of them.
///
/// This does not try to force an actual rotation failure (that needs a read-only
/// directory or a full disk); it verifies the wiring the diagnostic depends on: a
/// detached daemon must capture its stderr to a real file, not discard it.
#[test]
fn detached_daemon_captures_its_stderr_to_a_file() {
    let dir = tempdir();

    let status = isolated_command(dir.path(), &["daemon", "start"])
        .status()
        .unwrap();
    assert!(status.success(), "anthrex daemon start failed");
    let _stop_on_drop = StopOnDrop { dir: dir.path() };

    let socket = dir.path().join("daemon.sock");
    let deadline = Instant::now() + Duration::from_secs(3);
    while !socket.exists() {
        assert!(Instant::now() < deadline, "daemon did not start in time");
        std::thread::sleep(Duration::from_millis(10));
    }

    let stderr_log = dir.path().join("data/daemon.stderr.log");
    assert!(
        stderr_log.exists(),
        "a detached daemon must capture its stderr to a file instead of discarding it: {}",
        stderr_log.display()
    );
}
