mod support;

use std::os::unix::fs::PermissionsExt;
use std::path::Path;
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
/// answers its handshake.
///
/// Since the launch-gate fix this is *not* gated on the version probe at all — `serve`
/// runs beside it, so the real cost is an accept and one frame each way. The bound is
/// still derived from `CODEX_PROBE_TIMEOUT` rather than shrunk to match the new real
/// cost, and deliberately: these tests all run against a deliberately slow `codex`, and
/// if the gating ever came back this bound is what keeps `ls.finish()` reporting it as
/// a named assertion failure instead of as a test-harness deadline. The assertions that
/// actually *detect* the gating are the structural ones in
/// `a_client_handshake_is_answered_while_the_codex_probe_is_still_running` below.
///
///     CODEX_PROBE_TIMEOUT (5s) + WALL_CLOCK_SLACK (10s) = 15s
const LS_BOUND: Duration = daemon::lifecycle::CODEX_PROBE_TIMEOUT.saturating_add(WALL_CLOCK_SLACK);

/// `anthrex new`'s own legal worst case against a daemon that is already listening, plus
/// slack: the launch gate (`CODEX_PROBE_TIMEOUT`, if the startup probe is still running),
/// then project detection (`DETECT_TIMEOUT`), then a PTY spawn. That is the same chain
/// `crates/cli/src/client.rs`'s `CREATE_WINDOW_REPLY_TIMEOUT` is derived from; it cannot
/// be imported here, because `anthrex` is a binary-only crate with no `lib.rs` (the same
/// reason `WAIT_RELEASED_CAP` below is a commented literal), so the two daemon-side terms
/// are imported directly instead.
///
///     CODEX_PROBE_TIMEOUT (5s) + DETECT_TIMEOUT (5s) + WALL_CLOCK_SLACK (10s) = 20s
const NEW_BOUND: Duration = daemon::lifecycle::CODEX_PROBE_TIMEOUT
    .saturating_add(daemon::project::DETECT_TIMEOUT)
    .saturating_add(WALL_CLOCK_SLACK);

/// How long the stub `codex` below holds the `--version` probe, in the tests that need
/// the probe to still be running while they observe something else.
///
/// Not a margin over a production constant — it *is* the subject of those tests, the slow
/// `codex` they simulate — but it has two bounds of its own:
///
/// - it must comfortably exceed the real cost of whatever is being observed racing it:
///   spawning `anthrex ls` or `anthrex new` and getting it answered by a daemon that is
///   already listening, which `docs/timing-budgets.md`'s idle figures put at tens of
///   milliseconds for the spawn plus a socket round trip, and which this test's own
///   `elapsed` prints at ~0.3 s end to end. 3 s is about 10x that;
/// - it must stay *under* `CODEX_PROBE_TIMEOUT`, so the probe ends by finishing rather
///   than by being killed at its budget. A killed probe never writes `version-end`, which
///   would satisfy "the probe had not finished yet" for the wrong reason — the
///   `elapsed < PROBE_STALL` assertions are the second half of each of those tests
///   precisely so that case cannot pass silently.
const PROBE_STALL: Duration = Duration::from_secs(3);

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
    // Every caller of this helper asserts on what the *probe* did, and since the probe
    // runs beside `serve` rather than ahead of it, `ls` can now finish while it is still
    // running — which is the whole point of the fix, and would otherwise make these tests
    // race a `daemon stop` that cancels the probe before it reaches any conclusion. Taken
    // after `elapsed` above, so it does not enter any of the timing assertions. Bounded by
    // the probe's own budget plus slack: a probe that has not finished by then is stuck in
    // a way its own timeout was supposed to prevent.
    wait_for_probe_to_finish(
        dir.path(),
        daemon::lifecycle::CODEX_PROBE_TIMEOUT.saturating_add(WALL_CLOCK_SLACK),
    );
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

/// Blocks until the daemon logs [`daemon::lifecycle::PROBE_FINISHED`], the one line the
/// probe emits whatever it concluded. Waiting for a specific outcome instead would mean
/// every test guessing which of `warn`/`debug`/silence its own stub produces.
fn wait_for_probe_to_finish(dir: &Path, bound: Duration) {
    let log = dir.join("data/daemon.log");
    let deadline = Instant::now() + bound;
    loop {
        if std::fs::read_to_string(&log)
            .unwrap_or_default()
            .contains(daemon::lifecycle::PROBE_FINISHED)
        {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "the codex probe did not finish within {bound:?}: {}",
            std::fs::read_to_string(&log).unwrap_or_default()
        );
        std::thread::sleep(Duration::from_millis(10));
    }
}

/// A `codex` stub that records what the daemon does with it, in order, and holds the
/// `--version` probe open for `stall` so that order is observable from outside the daemon.
///
/// Every invocation appends one line to `<PROBE_DIR>/order`: `version-start` and
/// `version-end` bracket the probe's own stall, and `launch` is written by a window's
/// child before it settles into a long sleep. Reading that file is what lets the tests
/// below assert an *ordering* — "the client was answered before the probe finished", "the
/// window launched only after it did" — instead of a wall-clock margin against a constant
/// that a later change could move out from under them.
fn ordering_codex(dir: &Path, stall: Duration) -> std::path::PathBuf {
    let bin = dir.join("codex");
    let seconds = stall.as_secs();
    std::fs::write(
        &bin,
        format!(
            "#!/bin/sh\n\
             if [ \"$1\" = \"--version\" ]; then\n\
             printf 'version-start\\n' >> \"$PROBE_DIR/order\"\n\
             sleep {seconds}\n\
             printf 'version-end\\n' >> \"$PROBE_DIR/order\"\n\
             printf 'codex-cli 0.155.0\\n'\n\
             exit 0\n\
             fi\n\
             printf 'launch\\n' >> \"$PROBE_DIR/order\"\n\
             exec sleep 600\n"
        ),
    )
    .unwrap();
    std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o700)).unwrap();
    bin
}

/// What [`ordering_codex`] has recorded so far. A missing file means nothing has run yet.
fn order(dir: &Path) -> Vec<String> {
    std::fs::read_to_string(dir.join("order"))
        .unwrap_or_default()
        .lines()
        .map(str::to_string)
        .collect()
}

fn wait_for_order(dir: &Path, line: &str, bound: Duration) -> Vec<String> {
    let deadline = Instant::now() + bound;
    loop {
        let recorded = order(dir);
        if recorded.iter().any(|recorded| recorded == line) {
            return recorded;
        }
        assert!(
            Instant::now() < deadline,
            "the codex stub never recorded '{line}' within {bound:?}: {recorded:?}"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
}

/// Starts a `--foreground` daemon whose `codex` is [`ordering_codex`], and returns only
/// once the probe has actually begun — so a test that goes on to connect knows it is
/// racing a probe *in flight*, not one that has not started or has already finished.
fn daemon_with_a_stalled_probe(dir: &Path, stall: Duration) -> RunningCommand {
    let bin = ordering_codex(dir, stall);
    let mut command = isolated_command(dir, &["daemon", "start", "--foreground"]);
    command
        .env("ANTHREX_CODEX_BIN", &bin)
        .env("ANTHREX_CONFIG", dir.join("config.toml"))
        .env("PROBE_DIR", dir);
    let daemon = RunningCommand::start(&mut command);
    // `bind_socket` runs before the probe starts, so `version-start` implies the socket
    // file is there too — one wait covers both. Bounded by the probe's own budget plus
    // slack: a probe that has not started by then makes nothing below meaningful.
    wait_for_order(
        dir,
        "version-start",
        daemon::lifecycle::CODEX_PROBE_TIMEOUT.saturating_add(WALL_CLOCK_SLACK),
    );
    daemon
}

/// Stops the daemon and waits for the process, so no test can leave one running.
fn stop_daemon(dir: &Path, daemon: RunningCommand) -> std::process::Output {
    let _ =
        RunningCommand::start(&mut isolated_command(dir, &["daemon", "stop"])).finish(STOP_BOUND);
    daemon.finish(DAEMON_EXIT_BOUND)
}

/// The defect in its sharpest form: a client that connects while the startup probe is
/// still running must be **answered**, not queued behind it.
///
/// `bind_socket` creates the socket file before the probe runs, so the connection is
/// accepted into the listen backlog at once — but nothing read or answered it until
/// `server::serve` started, and `server::serve` started only after
/// `codex_version::check` returned. That left `proto::HANDSHAKE_TIMEOUT` (5 s) racing
/// `CODEX_PROBE_TIMEOUT` (5 s) with nothing between them: measured on this host with a
/// `codex` that burns the whole probe budget, `anthrex ls` came back at 4.89-4.94 s
/// against a hard 5.00 s client deadline, a 60-140 ms margin between working and exiting
/// non-zero with "timed out waiting for the daemon's handshake".
///
/// The assertion is deliberately not a margin against either of those constants: a test
/// written that way stops testing anything the moment somebody changes one. It is an
/// ordering — when `ls` came back, the probe had not finished — and `elapsed <
/// PROBE_STALL` is its companion, there so that "the probe had not finished" cannot pass
/// because the probe was *killed* at its budget rather than outrun.
#[test]
fn a_client_handshake_is_answered_while_the_codex_probe_is_still_running() {
    let dir = tempdir();
    let daemon = daemon_with_a_stalled_probe(dir.path(), PROBE_STALL);

    let started = Instant::now();
    let ls =
        RunningCommand::start(&mut isolated_command(dir.path(), &["ls", "--json"])).finish(LS_BOUND);
    let elapsed = started.elapsed();
    let order_when_answered = order(dir.path());

    let daemon = stop_daemon(dir.path(), daemon);

    assert!(
        ls.status.success(),
        "anthrex ls exited with {:?} while the codex probe was running\n--- stderr ---\n{}",
        ls.status,
        String::from_utf8_lossy(&ls.stderr),
    );
    assert!(
        !order_when_answered.contains(&"version-end".to_string()),
        "the handshake was answered only after the codex probe finished, which is the \
         defect: {order_when_answered:?}",
    );
    assert!(
        elapsed < PROBE_STALL,
        "anthrex ls took {elapsed:?}, which is the stub's whole stall ({PROBE_STALL:?}) — \
         the probe was killed at its budget rather than outrun, so the assertion above \
         proved nothing",
    );
    assert!(daemon.status.success(), "{:?}", daemon.status);
}

/// The other half of the same guard, and the reason it is a gate rather than a deletion:
/// a window launch must still not happen until the probe is done with the `codex` binary
/// (milestone 3's "finish its startup version probe before accepting window launches",
/// which exists to keep the probe's own `codex --version` from overlapping a real
/// launch of the same program).
///
/// Asserted as the exact recorded order rather than as a wall-clock wait, so it cannot
/// pass by the launch merely being slow. Under a fix that dropped the ordering entirely —
/// spawn the probe, gate nothing — this reads `version-start, launch, version-end`,
/// because `anthrex new` reaches the daemon in tens of milliseconds against a stub that
/// holds the probe for `PROBE_STALL`.
#[test]
fn a_window_launch_waits_for_the_codex_probe_to_finish() {
    let dir = tempdir();
    let daemon = daemon_with_a_stalled_probe(dir.path(), PROBE_STALL);

    let mut command = isolated_command(
        dir.path(),
        &["new", "--runtime", "codex", "--name", "gated"],
    );
    command
        .env("ANTHREX_CONFIG", dir.path().join("config.toml"))
        .current_dir(dir.path());
    let new = RunningCommand::start(&mut command).finish(NEW_BOUND);
    // The `launch` line is written by the window's own child after `Window::spawn`
    // returns, so it can trail the `Created` reply `new` waits for.
    let recorded = wait_for_order(dir.path(), "launch", NEW_BOUND);

    let daemon = stop_daemon(dir.path(), daemon);

    assert!(
        new.status.success(),
        "anthrex new exited with {:?}\n--- stderr ---\n{}",
        new.status,
        String::from_utf8_lossy(&new.stderr),
    );
    assert_eq!(
        recorded,
        ["version-start", "version-end", "launch"],
        "a window launched before the codex version probe had finished",
    );
    assert!(daemon.status.success(), "{:?}", daemon.status);
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
///
/// Strengthened since: for most of this test's life it asserted only that the socket
/// *file* appeared and that `daemon start` exited 0, neither of which says the daemon is
/// usable — `bind_socket` creates that file before the probe even starts. A `daemon
/// start` that "succeeds" and hands back a daemon that will not answer for another four
/// seconds has not done what this test's name claims. So it now does what a user does
/// next, immediately: it runs a client, and requires that client to be answered while the
/// probe is still stalled. Ordering, not a margin — see
/// `a_client_handshake_is_answered_while_the_codex_probe_is_still_running` for why.
#[test]
fn daemon_start_does_not_wait_on_a_slow_codex_probe() {
    let dir = tempdir();
    // Longer than `ENSURE_DAEMON_SOCKET_WAIT` (3 s), which is the wait this test's first
    // assertion is about, and still inside `CODEX_PROBE_TIMEOUT` (5 s), so the probe ends
    // by finishing rather than by being killed (see `PROBE_STALL`).
    const SLOW_PROBE_STALL: Duration = Duration::from_secs(4);
    let bin = ordering_codex(dir.path(), SLOW_PROBE_STALL);

    let mut command = isolated_command(dir.path(), &["daemon", "start"]);
    command
        .env("ANTHREX_CODEX_BIN", &bin)
        .env("ANTHREX_CONFIG", dir.path().join("config.toml"))
        .env("PROBE_DIR", dir.path());
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
    let answered = sock.exists().then(|| {
        let at = Instant::now();
        let ls = RunningCommand::start(&mut isolated_command(dir.path(), &["ls", "--json"]))
            .finish(LS_BOUND);
        (ls, at.elapsed(), order(dir.path()))
    });
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
    let (ls, ls_elapsed, order_when_answered) =
        answered.expect("daemon start reported success but never bound its socket");
    assert!(
        ls.status.success(),
        "the daemon `daemon start` reported as started did not answer a client: {:?}\n\
         --- stderr ---\n{}",
        ls.status,
        String::from_utf8_lossy(&ls.stderr),
    );
    assert!(
        !order_when_answered.contains(&"version-end".to_string()),
        "`daemon start` returned, but the first client was answered only once the slow \
         probe had finished: {order_when_answered:?}",
    );
    assert!(
        ls_elapsed < SLOW_PROBE_STALL,
        "anthrex ls took {ls_elapsed:?}, the stub's whole stall — the probe was killed at \
         its budget rather than outrun, so the assertion above proved nothing",
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
