mod support;

use std::os::unix::fs::PermissionsExt;
use std::process::Command;
use std::time::{Duration, Instant};
use support::{RunningCommand, isolated_command, tempdir};

/// Generous margin over a derived bound for a test that spawns several real processes
/// (the daemon, `ls`, `daemon stop`) end to end, matching the constant of the same name
/// and purpose in `crates/daemon/tests/git_registry.rs` (`docs/timing-budgets.md` standing
/// rule 1: derive a bound from the constants it wraps, then add slack for real subprocess
/// spawns under load — not a literal that can coincide with the thing it bounds).
const WALL_CLOCK_SLACK: Duration = Duration::from_secs(10);

/// What a freshly connecting client (`ls`, here) can legitimately wait before the daemon
/// answers its handshake: `bind_socket` runs before `codex_version::check`, so the socket
/// exists and queues the connection immediately, but nothing reads or answers it until
/// `server::serve` starts — which happens only after the version probe returns. The
/// probe's own outer timeout is `daemon::lifecycle::CODEX_PROBE_TIMEOUT` (an async timer, so it
/// fires close to on schedule regardless of how contended the blocking pool the probe's
/// child-process handling runs on is); `serve` then still has to bind, accept the queued
/// connection and answer the Hello. `WALL_CLOCK_SLACK` covers that plus ordinary
/// contention.
///
///     CODEX_PROBE_TIMEOUT (5s) + WALL_CLOCK_SLACK (10s) = 15s
const LS_BOUND: Duration = daemon::lifecycle::CODEX_PROBE_TIMEOUT.saturating_add(WALL_CLOCK_SLACK);

/// `daemon stop`'s own worst case: a handshake (`proto::HANDSHAKE_TIMEOUT`) to connect,
/// then `wait_released`'s own explicit cap (`crates/cli/src/main.rs`) before it bails.
/// Both are named constants the CLI test can import directly, so this is exact rather
/// than padded further — `WALL_CLOCK_SLACK` is still added for the real process spawn and
/// scheduling on top of the two waits themselves.
///
///     HANDSHAKE_TIMEOUT (5s) + WAIT_RELEASED_CAP (10s) + WALL_CLOCK_SLACK (10s) = 25s
const STOP_BOUND: Duration = proto::HANDSHAKE_TIMEOUT
    .saturating_add(WAIT_RELEASED_CAP)
    .saturating_add(WALL_CLOCK_SLACK);

/// `daemon stop`'s own bound on how long it waits for the daemon's lifetime lock to be
/// released (`crates/cli/src/main.rs`'s `daemon_command`, decision 27) before bailing with
/// a "did not exit within 10 s" error. Not exported by `main.rs` (`anthrex` is a
/// binary-only crate with no `lib.rs` — the same reason `hook_command.rs`'s `LIMIT` can
/// only comment-couple to `HOOK_DEADLINE` rather than import it), so this is a named,
/// commented literal rather than a bare one.
const WAIT_RELEASED_CAP: Duration = Duration::from_secs(10);

/// How long the daemon process itself may take to actually exit after `Shutdown`: the
/// same handshake and lock-release wait `STOP_BOUND` derives (the CLI's `daemon stop`
/// cannot report "stopped" before the daemon actually released its lock, so the daemon's
/// own exit is bounded by the same terms), plus its own final-state-save work, which is
/// not separately budgeted anywhere and is covered by `WALL_CLOCK_SLACK`.
const DAEMON_EXIT_BOUND: Duration = STOP_BOUND;

#[test]
fn running_command_finishes_when_a_descendant_keeps_output_handles_open() {
    let mut command = Command::new("sh");
    command.args([
        "-c",
        "printf 'known stdout'; printf 'known stderr' >&2; sleep 1 &",
    ]);

    let started = Instant::now();
    let output = RunningCommand::start(&mut command).finish(Duration::from_millis(250));

    assert!(output.status.success(), "{:?}", output.status);
    assert_eq!(output.stdout, b"known stdout");
    assert_eq!(output.stderr, b"known stderr");
    assert!(started.elapsed() < Duration::from_millis(750));
}

fn probe(script: &str) -> (String, Duration, tempfile::TempDir) {
    let dir = tempdir();
    let bin = dir.path().join("codex");
    std::fs::write(&bin, format!("#!/bin/sh\n{script}\n")).unwrap();
    std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o700)).unwrap();
    let started = Instant::now();
    let mut command = isolated_command(dir.path(), &["daemon", "start", "--foreground"]);
    command
        .env("ANTHREX_CODEX_BIN", &bin)
        .env("ANTHREX_LOG", "debug")
        .env("PROBE_DIR", dir.path());
    let daemon = RunningCommand::start(&mut command);
    // Only the socket *file* has to exist here (`bind_socket` runs before the version
    // probe); nothing reads or answers a connection queued on it until `server::serve`
    // starts, which is what `LS_BOUND` below budgets for. 7s is far past the near-instant
    // real cost of creating the file, so it is not itself an instance of this file's
    // defect shape — it just is not the wait that gates the handshake.
    let deadline = Instant::now() + Duration::from_secs(7);
    while !dir.path().join("daemon.sock").exists() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(10));
    }
    let ls = RunningCommand::start(&mut isolated_command(dir.path(), &["ls", "--json"]))
        .finish(LS_BOUND);
    let elapsed = started.elapsed();
    let stop = RunningCommand::start(&mut isolated_command(dir.path(), &["daemon", "stop"]))
        .finish(STOP_BOUND);
    let output = daemon.finish(DAEMON_EXIT_BOUND);
    // Named, not combined: a combined boolean cannot say which of the three commands
    // failed, its exit status, or its stderr — which is exactly what could not be told
    // apart from the CI log that motivated this split.
    for (name, out) in [
        ("anthrex ls --json", &ls),
        ("anthrex daemon stop", &stop),
        (
            "daemon start --foreground (the daemon process itself)",
            &output,
        ),
    ] {
        assert!(
            out.status.success(),
            "{name} exited with {:?}\n--- stderr ---\n{}\n--- stdout ---\n{}",
            out.status,
            String::from_utf8_lossy(&out.stderr),
            String::from_utf8_lossy(&out.stdout),
        );
    }
    assert!(
        output.stderr.is_empty(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let log = std::fs::read_to_string(dir.path().join("data/daemon.log")).unwrap();
    (log, elapsed, dir)
}

#[test]
fn unsupported_codex_version_warns_once_at_startup() {
    let (log, _, dir) =
        probe("printf '%s\\n' \"$*\" >> \"$PROBE_DIR/calls\"\nprintf 'codex-cli 0.134.9\\n'");
    assert!(
        log.contains("WARN") && log.contains("below supported minimum"),
        "{log}"
    );
    assert_eq!(
        std::fs::read_to_string(dir.path().join("calls")).unwrap(),
        "--version\n"
    );
}

#[test]
fn unreadable_codex_version_is_debug_only() {
    let (log, _, _) = probe("printf 'not a version\\n'; printf 'private stderr\\n' >&2");
    assert!(
        log.contains("DEBUG") && log.contains("could not read Codex version"),
        "{log}"
    );
    assert!(!log.contains("WARN"));
}

#[test]
fn codex_probe_timeout_kills_and_reaps_its_owned_process() {
    let (log, elapsed, dir) =
        probe("printf '%s' \"$$\" > \"$PROBE_DIR/pid\"\ntrap '' TERM\nwhile :; do :; done");
    // `elapsed` is `started.elapsed()` taken right after `probe`'s own `ls.finish(LS_BOUND)`
    // returns, so its legal worst case is exactly `LS_BOUND` — the same bound `ls` was
    // just held to, not a second, independently guessed number. The old `< 6s` was one
    // second over the probe's bare `CODEX_PROBE_TIMEOUT` (5s) with no room for the accept/reply
    // epsilon or scheduling jitter `LS_BOUND` accounts for; reproduced failing at 6.32s
    // and 6.42s (this assertion, pre-fix) under deliberate CPU contention (`timeout N yes
    // > /dev/null &` spinners), confirming the margin was too thin rather than merely
    // imprecise.
    assert!(elapsed < LS_BOUND, "{elapsed:?}");
    assert!(log.contains("could not read Codex version"), "{log}");
    let pid: i32 = std::fs::read_to_string(dir.path().join("pid"))
        .unwrap()
        .parse()
        .unwrap();
    // SAFETY: signal zero only queries the owned stub process, without signalling it.
    assert_eq!(unsafe { libc::kill(pid, 0) }, -1, "probe child survived");
    assert_eq!(
        std::io::Error::last_os_error().raw_os_error(),
        Some(libc::ESRCH)
    );
}

#[test]
fn codex_probe_does_not_wait_forever_for_inherited_stdout() {
    let (log, elapsed, _) = probe("sleep 30 &\nprintf 'codex-cli 0.155.0\\n'");
    // See the identical comment in `codex_probe_timeout_kills_and_reaps_its_owned_process`.
    assert!(elapsed < LS_BOUND, "{elapsed:?}");
    assert!(log.contains("could not read Codex version"), "{log}");
}

/// Fix wave 4, item 1 (M6.5 review, Critical 1): the socket bind must never wait on the
/// codex version probe. `spawn::ensure_daemon` gives up waiting for the socket after 3 s;
/// the probe's own budget is 5 s. A `codex` that answers `--version` anywhere in that gap
/// — a real shim, a cold page cache, a slow filesystem — must not fail `anthrex daemon
/// start`, because the daemon is in fact starting up fine.
///
/// Regresses the state in which `lifecycle::run` moved `codex_version::check` ahead of
/// `bind_socket` to get decision 12's state-file load before the bind, taking the probe
/// along with it by accident: `anthrex daemon start` against a `codex` that sleeps 4 s
/// (inside the probe's 5 s allowance) failed with "the daemon did not start within 3 s"
/// even though the daemon came up and bound its socket a moment later.
#[test]
fn daemon_start_does_not_wait_on_a_slow_codex_probe() {
    let dir = tempdir();
    let bin = dir.path().join("codex");
    std::fs::write(&bin, "#!/bin/sh\nsleep 4\nprintf 'codex-cli 0.155.0\\n'\n").unwrap();
    std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o700)).unwrap();

    let mut command = isolated_command(dir.path(), &["daemon", "start"]);
    command.env("ANTHREX_CODEX_BIN", &bin);
    let started = Instant::now();
    let output = RunningCommand::start(&mut command).finish(Duration::from_secs(6));
    let elapsed = started.elapsed();

    // Whatever the assertions below find, do not leave a detached daemon behind: even
    // when `daemon start` itself times out waiting for the socket, the daemon process it
    // spawned keeps running and eventually binds once the slow probe finishes. Wait for
    // that (bounded, generously past the stub's 4 s sleep) and stop it before asserting.
    let sock = dir.path().join("daemon.sock");
    let deadline = Instant::now() + Duration::from_secs(6);
    while !sock.exists() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(20));
    }
    if sock.exists() {
        // Same shape, same fix as `probe`'s own `stop`: `daemon stop`'s legal worst case
        // (`STOP_BOUND`, above) is 15s before any slack, well past the `6s` this used to
        // be pinned to — a bound that happened not to matter yet only because nothing
        // asserts on this particular `finish()`'s result, not because it was wide enough.
        let _ = RunningCommand::start(&mut isolated_command(dir.path(), &["daemon", "stop"]))
            .finish(STOP_BOUND);
    }

    assert!(
        output.status.success(),
        "daemon start failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    // Whole-branch-review m19: `< 3s` used to be a bare literal, independently typed
    // from `spawn::ensure_daemon`'s own socket-poll deadline (`crates/tui/src/spawn.rs`)
    // that it happens to equal. The property under test is "it did not wait out the
    // stub's 4 s sleep", which any bound comfortably under 4 s proves — but the
    // assertion *is* that deadline (the appendix's own verdict: correct as written to
    // equal it, not a margin question), so a change to the real constant should change
    // this assertion too. Now imports the same constant instead of a second literal
    // that could silently drift from it.
    assert!(
        elapsed < tui::spawn::ENSURE_DAEMON_SOCKET_WAIT,
        "daemon start took {elapsed:?}, meaning the socket bind waited on the codex probe"
    );
}
