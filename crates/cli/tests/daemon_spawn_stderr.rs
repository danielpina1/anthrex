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

/// Re-review Minor #5: `open_stderr_sink()` creates `daemon.stderr.log` as a side effect
/// in the CLI *parent*, before the child is even spawned — so the old version of this
/// test (asserting only that the file existed) stayed green even when `spawn_detached`
/// set `.stderr(Stdio::null())` outright, discarding the child's real stderr entirely.
/// Verified by reverting that one line and watching this test fail: see the task report.
///
/// A production daemon almost always starts *detached* — `anthrex`'s auto-start path
/// (`spawn::ensure_daemon` -> `spawn::spawn_detached` in `crates/cli/src/spawn.rs`) is
/// what every ordinary `anthrex ls`, `anthrex attach`, etc. goes through, not
/// `--foreground` (that flag exists for exactly this test suite and for `daemon start
/// --foreground` run by hand). `logfile::RotatingFile` can only report a failed
/// rotation via a direct write to its own stderr — nothing else on that path can
/// surface it, per its own module doc — so that diagnostic reaches a real user only if
/// the detached child's stderr goes somewhere other than `/dev/null`.
///
/// This does not try to force an actual rotation failure (that needs a read-only
/// directory or a full disk); instead it exercises the same fd the diagnostic would
/// use, via `ANTHREX_TEST_STDERR_PROBE` — a startup-only, test-gated write
/// `daemon::lifecycle::run` makes to its own real stderr, inert in every real
/// deployment — and asserts the known text actually lands in the file, not just that
/// the file exists.
#[test]
fn detached_daemon_captures_its_stderr_to_a_file() {
    let dir = tempdir();

    let status = isolated_command(dir.path(), &["daemon", "start"])
        .env("ANTHREX_TEST_STDERR_PROBE", "1")
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
    // The probe write races the socket becoming visible only in the sense that both
    // happen during the same startup; the probe runs at the very top of `run()`,
    // before the socket is even prepared, so by the time the socket file exists the
    // probe has already returned — but give the filesystem a moment regardless rather
    // than asserting on the very first read.
    let probe_deadline = Instant::now() + Duration::from_secs(3);
    let contents = loop {
        let contents = std::fs::read_to_string(&stderr_log).unwrap_or_default();
        if contents.contains("anthrex-test-stderr-probe") || Instant::now() >= probe_deadline {
            break contents;
        }
        std::thread::sleep(Duration::from_millis(10));
    };
    assert!(
        contents.contains("anthrex-test-stderr-probe"),
        "a detached daemon must capture the child's own stderr writes to this file, not \
         just have the file exist ({}): got {contents:?}",
        stderr_log.display()
    );
}
