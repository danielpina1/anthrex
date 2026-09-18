mod support;

use std::os::unix::fs::PermissionsExt;
use std::time::{Duration, Instant};
use support::{RunningCommand, isolated_command, tempdir};

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
