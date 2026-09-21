mod support;

use std::os::unix::fs::PermissionsExt;
use std::process::Command;
use std::time::{Duration, Instant};
use support::{RunningCommand, isolated_command, tempdir};

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
    let deadline = Instant::now() + Duration::from_secs(7);
    // The normal client handshake must wait until the one startup probe has finished.
    while !dir.path().join("daemon.sock").exists() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(10));
    }
    let ls = RunningCommand::start(&mut isolated_command(dir.path(), &["ls", "--json"]))
        .finish(Duration::from_secs(7));
    let elapsed = started.elapsed();
    let stop = RunningCommand::start(&mut isolated_command(dir.path(), &["daemon", "stop"]))
        .finish(Duration::from_secs(3));
    let output = daemon.finish(Duration::from_secs(3));
    assert!(ls.status.success() && stop.status.success() && output.status.success());
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
    assert!(elapsed < Duration::from_secs(6), "{elapsed:?}");
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
    assert!(elapsed < Duration::from_secs(6), "{elapsed:?}");
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
        let _ = RunningCommand::start(&mut isolated_command(dir.path(), &["daemon", "stop"]))
            .finish(Duration::from_secs(6));
    }

    assert!(
        output.status.success(),
        "daemon start failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        elapsed < Duration::from_secs(3),
        "daemon start took {elapsed:?}, meaning the socket bind waited on the codex probe"
    );
}
