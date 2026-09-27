//! Milestone 8b decision 27: `anthrex filter-run`, run as the built binary. It only
//! ever runs `/bin/sh -c <command>` with the test's own commands; every runtime command
//! points at a nonexistent path all the same.

mod support;

use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::time::Duration;

use support::{ANTHREX, RunningCommand, tempdir};

/// A `/bin/sh` and at most a few thousand lines of output: well under a second.
const LIMIT: Duration = Duration::from_secs(20);

fn filter_run(dir: &Path, log_dir: &Path, mode: &str, command: &str) -> Command {
    let mut cmd = Command::new(ANTHREX);
    cmd.args(["filter-run", "--mode", mode, "--log-dir"])
        .arg(log_dir)
        .args(["-c", command])
        .current_dir(dir)
        .env("ANTHREX_SOCKET", dir.join("daemon.sock"))
        .env("ANTHREX_DATA_DIR", dir.join("data"))
        .env("ANTHREX_CLAUDE_BIN", "/nonexistent/anthrex-test/claude")
        .env("ANTHREX_CODEX_BIN", "/nonexistent/anthrex-test/codex")
        // filter-run runs `$SHELL -c` for bash or zsh; pin plain sh so no startup file of
        // the user's can print into the exact output asserted below.
        .env("SHELL", "/bin/sh")
        .env_remove("BASH_ENV")
        .env_remove("ENV");
    cmd
}

fn run(dir: &Path, log_dir: &Path, mode: &str, command: &str) -> Output {
    RunningCommand::start(&mut filter_run(dir, log_dir, mode, command)).finish(LIMIT)
}

fn logs(log_dir: &Path) -> Vec<PathBuf> {
    let mut logs: Vec<PathBuf> = std::fs::read_dir(log_dir)
        .map(|entries| entries.map(|e| e.unwrap().path()).collect())
        .unwrap_or_default();
    logs.sort();
    logs
}

fn text(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}

#[test]
fn filter_run_preserves_exit_status_and_signals() {
    let dir = tempdir();
    let log_dir = dir.path().join("logs");
    let out = run(dir.path(), &log_dir, "failures-only", "echo before; exit 3");
    assert_eq!(out.status.code(), Some(3), "{}", text(&out.stderr));
    assert!(
        text(&out.stdout).ends_with("(1 lines, exit 3)\n"),
        "{}",
        text(&out.stdout)
    );
    // The child signals itself (`$$` is the exact `/bin/sh` filter-run spawned): the test
    // itself sends no signal to anything.
    let out = run(dir.path(), &log_dir, "tail", "kill -TERM $$");
    assert_eq!(out.status.code(), Some(128 + 15), "{}", text(&out.stderr));
    assert!(
        text(&out.stdout).ends_with("(0 lines, exit 143)\n"),
        "{}",
        text(&out.stdout)
    );
    let out = run(dir.path(), &log_dir, "none", "true");
    assert_eq!(out.status.code(), Some(0));
    assert_eq!(logs(&log_dir).len(), 3);
}

#[test]
fn filter_run_writes_the_full_log_and_prints_the_filtered_view() {
    let dir = tempdir();
    let log_dir = dir.path().join("task-tmp/anthrex-logs");
    let command = r#"awk 'BEGIN { for (i = 1; i <= 5000; i++) print (i == 2500 ? "test foo ... FAILED" : "line " i); exit 1 }'"#;
    let out = run(dir.path(), &log_dir, "failures-only", command);
    assert_eq!(out.status.code(), Some(1), "{}", text(&out.stderr));
    let logs = logs(&log_dir);
    assert_eq!(logs.len(), 1, "{logs:?}");
    let name = logs[0].file_name().unwrap().to_str().unwrap().to_string();
    let (ms, pid) = name
        .strip_suffix(".log")
        .and_then(|stem| stem.split_once('-'))
        .expect("<unix ms>-<pid>.log");
    assert!(ms.parse::<u64>().unwrap() > 1_600_000_000_000, "{name}");
    assert!(pid.parse::<u32>().is_ok(), "{name}");
    let expected: String = (1..=5000)
        .map(|i| match i {
            2500 => "test foo ... FAILED\n".to_string(),
            _ => format!("line {i}\n"),
        })
        .collect();
    assert_eq!(std::fs::read_to_string(&logs[0]).unwrap(), expected);

    let stdout = text(&out.stdout);
    let printed: Vec<&str> = stdout.lines().collect();
    assert!(printed.len() <= 32, "{stdout}");
    assert!(printed.contains(&"test foo ... FAILED"), "{stdout}");
    assert!(printed.contains(&"line 5000"), "{stdout}");
    assert_eq!(
        printed.last().copied(),
        Some(
            format!(
                "[anthrex] full output: {} (5000 lines, exit 1)",
                logs[0].display()
            )
            .as_str()
        )
    );
    assert!(out.stderr.is_empty(), "{}", text(&out.stderr));
}

#[test]
fn filter_run_passes_stdin_through() {
    let dir = tempdir();
    let log_dir = dir.path().join("logs");
    let mut child = RunningCommand::start(&mut filter_run(dir.path(), &log_dir, "none", "cat"));
    child.input(b"from stdin\nsecond\n".to_vec());
    let out = child.finish(LIMIT);
    assert_eq!(out.status.code(), Some(0), "{}", text(&out.stderr));
    let logs = logs(&log_dir);
    assert_eq!(
        std::fs::read_to_string(&logs[0]).unwrap(),
        "from stdin\nsecond\n"
    );
    assert!(text(&out.stdout).starts_with("from stdin\nsecond\n[anthrex] full output: "));
}

#[test]
fn filter_run_still_runs_when_the_log_cannot_be_written() {
    let dir = tempdir();
    let locked = dir.path().join("locked");
    std::fs::create_dir(&locked).unwrap();
    std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o555)).unwrap();
    let log_dir = locked.join("anthrex-logs");
    let out = run(
        dir.path(),
        &log_dir,
        "failures-only",
        "echo one; echo error two; touch ran; exit 4",
    );
    std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o755)).unwrap();
    assert_eq!(out.status.code(), Some(4));
    // Unfiltered: every line, and no log line.
    assert_eq!(text(&out.stdout), "one\nerror two\n");
    assert!(dir.path().join("ran").exists());
    let stderr = text(&out.stderr);
    assert!(
        stderr.starts_with("[anthrex] could not write the log: "),
        "{stderr}"
    );
    assert!(!log_dir.exists());
}
