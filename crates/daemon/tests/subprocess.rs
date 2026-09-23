//! Pins `daemon::subprocess::run` and `run_captured` against `/bin/sh` scripts rather
//! than git, so these tests exercise the runner's own hardening (stdout/stderr capture,
//! the byte caps, the deadline, the spawn-error distinction) and not git's behaviour.

use daemon::subprocess::{HeadTail, Outcome, run, run_captured, run_captured_head_tail};
use std::fs;
use std::io;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};
use tempfile::tempdir;

fn write_script(dir: &Path, name: &str, body: &str) -> PathBuf {
    let script = dir.join(name);
    fs::write(&script, body).unwrap();
    let mut permissions = fs::metadata(&script).unwrap().permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(&script, permissions).unwrap();
    script
}

/// The three scripts `run_is_unchanged` re-runs through `run`, exactly as written here,
/// to pin that `run`'s own behaviour has not moved.
const WRITES_BOTH: &str =
    "#!/bin/sh\nprintf 'stdout-line\\n'\nprintf 'stderr-line\\n' 1>&2\nexit 0\n";
const FAILS_WITH_STDERR: &str = "#!/bin/sh\nprintf 'stderr-line\\n' 1>&2\nexit 3\n";
const MISSING_PROGRAM: &str = "/definitely/missing/program-xyz";

#[test]
fn run_captured_returns_stdout_and_stderr() {
    let scripts = tempdir().unwrap();
    let script = write_script(scripts.path(), "writes-both", WRITES_BOTH);

    let mut command = Command::new(&script);
    let captured = run_captured(&mut command, 64 * 1024, 64 * 1024, Duration::from_secs(5));

    match captured.outcome {
        Outcome::Complete(stdout) => assert_eq!(stdout, b"stdout-line\n"),
        other => panic!("expected Outcome::Complete, got {other:?}"),
    }
    assert_eq!(captured.stderr, "stderr-line\n");
    assert_eq!(captured.spawn_error, None);
}

#[test]
fn a_non_zero_exit_keeps_its_stderr() {
    let scripts = tempdir().unwrap();
    let script = write_script(scripts.path(), "fails-with-stderr", FAILS_WITH_STDERR);

    let mut command = Command::new(&script);
    let captured = run_captured(&mut command, 64 * 1024, 64 * 1024, Duration::from_secs(5));

    assert!(
        matches!(captured.outcome, Outcome::Failed),
        "expected Outcome::Failed, got {:?}",
        captured.outcome
    );
    assert_eq!(captured.stderr, "stderr-line\n");
    assert_eq!(captured.spawn_error, None);
}

#[test]
fn a_missing_program_reports_a_spawn_error() {
    let mut command = Command::new(MISSING_PROGRAM);
    let captured = run_captured(&mut command, 64 * 1024, 64 * 1024, Duration::from_secs(5));

    assert!(
        matches!(captured.outcome, Outcome::Failed),
        "expected Outcome::Failed, got {:?}",
        captured.outcome
    );
    assert_eq!(captured.spawn_error, Some(io::ErrorKind::NotFound));
}

#[test]
fn stderr_is_bounded_and_does_not_block_the_child() {
    let scripts = tempdir().unwrap();
    // Far more than the 4096-byte cap below: about 100 bytes/line * 50000 lines, ~5MB.
    let script = write_script(
        scripts.path(),
        "chatty-stderr",
        "#!/bin/sh\n\
         yes 'xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx' \
         | head -n 50000 1>&2\n\
         printf 'stdout-line\\n'\n\
         exit 0\n",
    );
    let max_stderr_bytes = 4096;
    let started = Instant::now();

    let mut command = Command::new(&script);
    let captured = run_captured(
        &mut command,
        64 * 1024,
        max_stderr_bytes,
        Duration::from_secs(10),
    );

    assert!(
        started.elapsed() < Duration::from_secs(10),
        "run_captured must not block on the child's stderr; took {:?}",
        started.elapsed()
    );
    match captured.outcome {
        Outcome::Complete(stdout) => assert_eq!(stdout, b"stdout-line\n"),
        other => panic!("expected Outcome::Complete, got {other:?}"),
    }
    assert!(
        captured.stderr.len() <= max_stderr_bytes,
        "stderr must be cut to the cap, got {} bytes",
        captured.stderr.len()
    );
    assert!(!captured.stderr.is_empty());
}

/// Same three scripts as `run_captured_returns_stdout_and_stderr`,
/// `a_non_zero_exit_keeps_its_stderr` and `a_missing_program_reports_a_spawn_error`,
/// through the unchanged `run` instead: it must keep giving exactly the `Outcome` it
/// gives today, with the child's stderr going nowhere (it is nulled, never captured).
#[test]
fn run_is_unchanged() {
    let scripts = tempdir().unwrap();
    let writes_both = write_script(scripts.path(), "writes-both-plain", WRITES_BOTH);
    let fails_with_stderr =
        write_script(scripts.path(), "fails-with-stderr-plain", FAILS_WITH_STDERR);

    let mut command = Command::new(&writes_both);
    match run(&mut command, 64 * 1024, Duration::from_secs(5)) {
        Outcome::Complete(stdout) => assert_eq!(stdout, b"stdout-line\n"),
        other => panic!("expected Outcome::Complete, got {other:?}"),
    }

    let mut command = Command::new(&fails_with_stderr);
    assert!(matches!(
        run(&mut command, 64 * 1024, Duration::from_secs(5)),
        Outcome::Failed
    ));

    let mut command = Command::new(MISSING_PROGRAM);
    assert!(matches!(
        run(&mut command, 64 * 1024, Duration::from_secs(5)),
        Outcome::Failed
    ));
}

/// `run_captured_head_tail` keeps the first and last bytes of an output of any size,
/// drains the rest, and never reports it as over a cap.
#[test]
fn head_tail_keeps_both_ends_of_a_large_output() {
    let scripts = tempdir().unwrap();
    // 2 000 000 numbered lines, about 15 MB: far past any head or tail kept here.
    let script = write_script(
        scripts.path(),
        "counts",
        "#!/bin/sh\nawk 'BEGIN{for(i=1;i<=2000000;i++)print i}'\nprintf 'err\\n' 1>&2\n",
    );
    let (outcome, kept, stderr, spawn_error) = run_captured_head_tail(
        &mut Command::new(&script),
        8,
        12,
        1024,
        Duration::from_secs(60),
    );
    assert!(matches!(outcome, Outcome::Complete(_)), "{outcome:?}");
    assert_eq!(spawn_error, None);
    assert_eq!(stderr, "err\n");
    assert_eq!(kept.head, b"1\n2\n3\n4\n");
    assert_eq!(kept.tail, b"999\n2000000\n");
    assert!(kept.dropped());
    let expected_total: u64 = (1..=2_000_000u64)
        .map(|n| n.to_string().len() as u64 + 1)
        .sum();
    assert_eq!(kept.total, expected_total);

    // A short output is all in the head, nothing dropped.
    let short = write_script(scripts.path(), "short", "#!/bin/sh\nprintf 'abcdef'\n");
    let (_, kept, _, _) =
        run_captured_head_tail(&mut Command::new(&short), 4, 4, 64, Duration::from_secs(5));
    assert_eq!(
        kept,
        HeadTail {
            head: b"abcd".to_vec(),
            tail: b"ef".to_vec(),
            total: 6
        }
    );
    assert!(!kept.dropped());
}
