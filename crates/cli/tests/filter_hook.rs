//! Milestone 8b decision 28: `anthrex filter-hook`, the `PreToolUse` hook command, run
//! as the built binary. The wrapped commands it produces run `anthrex filter-run` with
//! the test's own shell commands only; every runtime command points at a nonexistent
//! path all the same.

mod support;

use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::time::{Duration, Instant};

use daemon::output_filter::FILTER_HOOK_DEADLINE;
use serde_json::{Value, json};
use support::{ANTHREX, RunningCommand, assert_silent_success, tempdir};

/// A process spawn on top of the hook's own deadline (docs/timing-budgets.md).
const LIMIT: Duration = Duration::from_secs(20);

fn isolate(cmd: &mut Command, dir: &Path) {
    cmd.current_dir(dir)
        .env("ANTHREX_SOCKET", dir.join("daemon.sock"))
        .env("ANTHREX_DATA_DIR", dir.join("data"))
        .env("ANTHREX_CLAUDE_BIN", "/nonexistent/anthrex-test/claude")
        .env("ANTHREX_CODEX_BIN", "/nonexistent/anthrex-test/codex")
        // zsh and bash read no startup file of the user's.
        .env("ZDOTDIR", dir)
        .env_remove("BASH_ENV")
        .env_remove("ENV");
}

fn hook_cmd(dir: &Path, log_dir: &Path, prefixes: &[&str]) -> Command {
    let mut cmd = Command::new(ANTHREX);
    cmd.args(["filter-hook", "--mode", "failures-only", "--log-dir"])
        .arg(log_dir);
    for prefix in prefixes {
        cmd.args(["--prefix", prefix]);
    }
    isolate(&mut cmd, dir);
    cmd
}

fn hook(dir: &Path, log_dir: &Path, prefixes: &[&str], payload: Vec<u8>) -> Output {
    let mut child = RunningCommand::start(&mut hook_cmd(dir, log_dir, prefixes));
    child.input(payload);
    child.finish(LIMIT)
}

fn bash(command: &str) -> Vec<u8> {
    json!({
        "session_id": "s1",
        "hook_event_name": "PreToolUse",
        "tool_name": "Bash",
        "tool_input": {"command": command, "description": "Run it"},
        "tool_use_id": "toolu_1"
    })
    .to_string()
    .into_bytes()
}

fn exe() -> PathBuf {
    std::fs::canonicalize(ANTHREX).unwrap()
}

#[test]
fn hook_wraps_a_matching_command() {
    let dir = tempdir();
    let out = hook(
        dir.path(),
        Path::new("/tmp/ax-t1/anthrex-logs"),
        &["cargo test", "cargo build"],
        bash("cd crates/daemon && cargo test -q"),
    );
    let expected = format!(
        r#"{{"hookSpecificOutput":{{"hookEventName":"PreToolUse","updatedInput":{{"command":"cd crates/daemon && '{}' filter-run --mode failures-only --log-dir '/tmp/ax-t1/anthrex-logs' -c 'cargo test -q'","description":"Run it"}}}}}}"#,
        exe().display()
    );
    assert_eq!(
        String::from_utf8_lossy(&out.stdout),
        format!("{expected}\n")
    );
    assert!(out.status.success());
    assert!(
        out.stderr.is_empty(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
}

#[test]
fn hook_is_silent_for_other_commands_and_tools() {
    let dir = tempdir();
    let logs = dir.path().join("logs");
    for payload in [
        bash("cargo testing"),
        bash("echo cargo test"),
        bash("'/x/anthrex' filter-run --mode tail --log-dir '/l' -c 'cargo test'"),
        json!({"tool_name": "Read", "tool_input": {"command": "cargo test"}})
            .to_string()
            .into_bytes(),
        json!({"tool_name": "Bash", "tool_input": {"cmd": "cargo test"}})
            .to_string()
            .into_bytes(),
    ] {
        assert_silent_success(&hook(dir.path(), &logs, &["cargo test"], payload));
    }
    // No prefix at all: nothing matches.
    assert_silent_success(&hook(dir.path(), &logs, &[], bash("cargo test")));
}

/// Runs `command` through `<shell> -c` in `cwd`, with `SHELL` set to `shell` as the
/// Bash tool's environment has it, and stdout and stderr on one file, as filter-run's
/// log has them; returns the bytes and the exit code.
fn sh(shell: &str, cwd: &Path, command: &str, out: &Path) -> (Vec<u8>, Option<i32>) {
    let file = std::fs::File::create(out).unwrap();
    let mut cmd = Command::new(shell);
    cmd.args(["-c", command])
        .stdin(Stdio::null())
        .stdout(file.try_clone().unwrap())
        .stderr(file);
    isolate(&mut cmd, cwd);
    cmd.env("SHELL", shell);
    let mut child = cmd.spawn().unwrap();
    let deadline = Instant::now() + LIMIT;
    let status = loop {
        if let Some(status) = child.try_wait().unwrap() {
            break status;
        }
        assert!(Instant::now() < deadline, "{command:?} outlived {LIMIT:?}");
        std::thread::sleep(Duration::from_millis(5));
    };
    (std::fs::read(out).unwrap(), status.code())
}

#[test]
fn wrapped_commands_behave_exactly_like_the_original() {
    let dir = tempdir();
    let work = dir.path().join("work");
    std::fs::create_dir_all(work.join("sub")).unwrap();
    std::fs::write(work.join("sub/listed.txt"), "x").unwrap();
    std::fs::create_dir_all(work.join("a/b")).unwrap();
    std::fs::write(work.join("a/test_y.py"), "").unwrap();
    std::fs::write(work.join("a/b/test_x.py"), "").unwrap();
    let prefixes = ["printf", "echo", "cat", "false", "sh", "ls"];
    let commands = [
        "printf 'a b\\n'",
        "echo \"it's\"",
        "cat <<'EOF'\nfirst line\nsecond 'quoted' $HOME line\nEOF",
        "false || echo ok",
        "FOO=1 sh -c 'echo $FOO'",
        "cd sub && ls",
        "echo 'ü\tafter a tab'; echo \"ü\"",
        // Under zsh `**` recurses; under sh it is `*`. Either way the wrapped command
        // must glob as the original does (review I1).
        "ls **/test_*.py",
    ];
    let shells: Vec<&str> = ["/bin/sh", "/bin/bash", "/bin/zsh"]
        .into_iter()
        .filter(|shell| Path::new(shell).exists())
        .collect();
    if !shells.contains(&"/bin/zsh") {
        println!("skipping the zsh runs: /bin/zsh does not exist");
    }
    for shell in shells {
        let tag = shell.rsplit('/').next().unwrap();
        for (i, command) in commands.into_iter().enumerate() {
            let log_dir = dir.path().join(format!("logs-{tag}-{i}"));
            let original = run_both(&dir, &work, shell, command, &prefixes, &log_dir);
            if shell == "/bin/zsh" && command.contains("**") {
                assert!(
                    String::from_utf8_lossy(&original).contains("a/b/test_x.py"),
                    "zsh's ** did not recurse"
                );
            }
        }
    }
}

/// Wraps `command` through the hook, runs the original and the wrapped command under
/// `shell`, checks the exit codes and that the log holds exactly the original's
/// output, and returns that output.
fn run_both(
    dir: &tempfile::TempDir,
    work: &Path,
    shell: &str,
    command: &str,
    prefixes: &[&str],
    log_dir: &Path,
) -> Vec<u8> {
    let tag = log_dir.file_name().unwrap().to_str().unwrap();
    let out = hook(dir.path(), log_dir, prefixes, bash(command));
    assert!(out.status.success(), "{command:?}");
    let answer: Value = serde_json::from_slice(&out.stdout)
        .unwrap_or_else(|e| panic!("{command:?}: {e}: {:?}", out.stdout));
    let wrapped = answer["hookSpecificOutput"]["updatedInput"]["command"]
        .as_str()
        .expect("wrapped")
        .to_string();
    assert!(wrapped.contains(" filter-run "), "{wrapped}");

    let (original, code) = sh(
        shell,
        work,
        command,
        &dir.path().join(format!("orig-{tag}")),
    );
    assert!(!original.is_empty(), "{command:?} printed nothing");
    let (_, wrapped_code) = sh(
        shell,
        work,
        &wrapped,
        &dir.path().join(format!("wrap-{tag}")),
    );
    assert_eq!(wrapped_code, code, "{shell} {command:?}");
    let logs: Vec<PathBuf> = std::fs::read_dir(log_dir)
        .unwrap()
        .map(|e| e.unwrap().path())
        .collect();
    assert_eq!(logs.len(), 1, "{command:?}");
    assert_eq!(
        String::from_utf8_lossy(&std::fs::read(&logs[0]).unwrap()),
        String::from_utf8_lossy(&original),
        "{shell} {command:?}"
    );
    original
}

/// Review I2: the leading `cd` stays outside the wrapper, so the agent's shell still
/// moves, and the command still runs in the new directory.
#[test]
fn a_wrapped_cd_still_moves_the_agents_shell() {
    let dir = tempdir();
    let work = dir.path().join("work");
    std::fs::create_dir_all(work.join("sub")).unwrap();
    std::fs::write(work.join("sub/listed.txt"), "x").unwrap();
    let log_dir = dir.path().join("logs");
    let out = hook(dir.path(), &log_dir, &["ls"], bash("cd sub && ls"));
    let answer: Value = serde_json::from_slice(&out.stdout).unwrap();
    let wrapped = answer["hookSpecificOutput"]["updatedInput"]["command"]
        .as_str()
        .unwrap()
        .to_string();
    assert_eq!(
        wrapped,
        format!(
            "cd sub && '{}' filter-run --mode failures-only --log-dir '{}' -c 'ls'",
            exe().display(),
            log_dir.display()
        )
    );
    let (moved, code) = sh(
        "/bin/sh",
        &work,
        &format!("{wrapped}; pwd"),
        &dir.path().join("out"),
    );
    assert_eq!(code, Some(0));
    let moved = String::from_utf8_lossy(&moved);
    let sub = std::fs::canonicalize(work.join("sub")).unwrap();
    assert_eq!(moved.lines().last(), Some(sub.to_str().unwrap()), "{moved}");
    assert!(
        moved.starts_with("listed.txt\n[anthrex] full output: "),
        "{moved}"
    );
}

#[test]
fn hook_exits_zero_silently_on_garbage_and_oversized_input() {
    let dir = tempdir();
    let logs = dir.path().join("logs");
    for payload in [
        b"not json {".to_vec(),
        b"[1, 2]".to_vec(),
        Vec::new(),
        vec![0xff, 0xfe, 0x00],
    ] {
        assert_silent_success(&hook(dir.path(), &logs, &["cargo test"], payload));
    }
    // A matching payload over the 1 MiB read limit is left alone.
    let big = json!({
        "tool_name": "Bash",
        "tool_input": {"command": "cargo test", "description": "x".repeat(2 * 1024 * 1024)}
    });
    let out = hook(
        dir.path(),
        &logs,
        &["cargo test"],
        big.to_string().into_bytes(),
    );
    assert_silent_success(&out);
    // A valid matching payload followed by more than 1 MiB of whitespace (which JSON
    // allows) is over the read limit too; the same payload with a little whitespace is
    // wrapped.
    let mut padded = bash("cargo test");
    padded.extend(b" \n".repeat(64));
    let out = hook(dir.path(), &logs, &["cargo test"], padded.clone());
    assert!(!out.stdout.is_empty(), "the control was not wrapped");
    padded.extend(vec![b' '; 1024 * 1024 + 1]);
    assert_silent_success(&hook(dir.path(), &logs, &["cargo test"], padded));
    // Bad arguments: silent too.
    for args in [
        vec!["filter-hook"],
        vec!["filter-hook", "--mode", "bogus", "--log-dir", "/tmp/x"],
        vec![
            "filter-hook",
            "--mode",
            "bogus",
            "--log-dir",
            "/tmp/x",
            "--prefix",
            "cargo test",
        ],
        vec!["filter-hook", "--help"],
    ] {
        let mut cmd = Command::new(ANTHREX);
        cmd.args(&args);
        isolate(&mut cmd, dir.path());
        let mut child = RunningCommand::start(&mut cmd);
        child.input(bash("cargo test"));
        assert_silent_success(&child.finish(LIMIT));
    }
}

#[test]
fn hook_finishes_within_its_deadline() {
    let dir = tempdir();
    let logs = dir.path().join("logs");
    let started = Instant::now();
    let mut child = RunningCommand::start(&mut hook_cmd(dir.path(), &logs, &["cargo test"]));
    // Held open and never written: the hook must give up at its own deadline.
    let stdin = child.stdin();
    let bound = FILTER_HOOK_DEADLINE + Duration::from_secs(2);
    while child.is_running() {
        assert!(started.elapsed() < LIMIT, "filter-hook outlived {LIMIT:?}");
        std::thread::sleep(Duration::from_millis(5));
    }
    let elapsed = started.elapsed();
    let out = child.finish(LIMIT);
    drop(stdin);
    assert!(elapsed <= bound, "{elapsed:?} > {bound:?}");
    assert_silent_success(&out);
}

#[test]
fn help_lists_neither_filter_command() {
    let dir = tempdir();
    let mut cmd = Command::new(ANTHREX);
    cmd.arg("--help");
    isolate(&mut cmd, dir.path());
    let out = RunningCommand::start(&mut cmd).finish(LIMIT);
    let help = String::from_utf8_lossy(&out.stdout);
    assert!(out.status.success() && help.contains("Commands:"), "{help}");
    assert!(
        !help.contains("filter-run") && !help.contains("filter-hook"),
        "{help}"
    );
}
