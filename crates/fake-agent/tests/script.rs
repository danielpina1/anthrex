use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use serde_json::{Value, json};
use tempfile::TempDir;

fn fake_agent() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_fake-agent"))
}

fn script_file(temp: &TempDir, lines: &[Value]) -> PathBuf {
    let path = temp.path().join("script.jsonl");
    let contents = lines
        .iter()
        .map(Value::to_string)
        .collect::<Vec<_>>()
        .join("\n");
    fs::write(&path, format!("{contents}\n")).unwrap();
    path
}

fn command(script: &Path) -> Command {
    let mut command = Command::new(fake_agent());
    command
        .env("FAKE_AGENT_SCRIPT", script)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    command
}

fn run(mut command: Command) -> Output {
    let child = command.spawn().unwrap();
    child.wait_with_output().unwrap()
}

fn json_file(path: &Path) -> Value {
    serde_json::from_slice(&fs::read(path).unwrap()).unwrap()
}

#[test]
fn prints_and_exits_with_the_scripted_code() {
    let temp = tempfile::tempdir_in("/tmp").unwrap();
    let script = script_file(&temp, &[json!({"print": "hello"}), json!({"exit": 4})]);

    let output = run(command(&script));

    assert_eq!(output.status.code(), Some(4));
    assert_eq!(output.stdout, b"hello");
}

#[test]
fn runs_the_claude_hook_with_the_payload_on_stdin() {
    let temp = tempfile::tempdir_in("/tmp").unwrap();
    let payload_path = temp.path().join("p.json");
    let script = script_file(
        &temp,
        &[
            json!({"hook": "PreToolUse", "payload": {"tool_name": "Bash"}}),
            json!({"exit": 0}),
        ],
    );
    let settings = json!({
        "hooks": {"PreToolUse": [{"hooks": [{
            "command": format!("cat > {}", payload_path.display())
        }]}]}
    });
    let mut command = command(&script);
    command
        .env("ANTHREX_WINDOW_ID", "9")
        .arg("--settings")
        .arg(settings.to_string());

    let output = run(command);

    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let payload = json_file(&payload_path);
    assert_eq!(payload["hook_event_name"], "PreToolUse");
    assert_eq!(payload["session_id"], "fake-session-9");
    assert_eq!(payload["tool_name"], "Bash");
}

#[test]
fn script_keys_are_not_overwritten() {
    let temp = tempfile::tempdir_in("/tmp").unwrap();
    let payload_path = temp.path().join("p.json");
    let script = script_file(
        &temp,
        &[
            json!({"hook": "Stop", "payload": {"session_id": "mine", "cwd": "/mine"}}),
            json!({"exit": 0}),
        ],
    );
    let settings = json!({
        "hooks": {"Stop": [{"hooks": [{
            "command": format!("cat > {}", payload_path.display())
        }]}]}
    });
    let mut command = command(&script);
    command
        .env("ANTHREX_WINDOW_ID", "9")
        .arg("--settings")
        .arg(settings.to_string());

    let output = run(command);

    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let payload = json_file(&payload_path);
    assert_eq!(payload["session_id"], "mine");
    assert_eq!(payload["cwd"], "/mine");
}

#[test]
fn runs_codex_notify_with_the_payload_as_last_argument() {
    let temp = tempfile::tempdir_in("/tmp").unwrap();
    let payload_path = temp.path().join("n.json");
    let script = script_file(
        &temp,
        &[
            json!({"notify": {"last-assistant-message": "done"}}),
            json!({"exit": 0}),
        ],
    );
    let notify = json!([
        "/bin/sh",
        "-c",
        format!("printf '%s' \"$0\" > {}", payload_path.display())
    ]);
    let mut command = command(&script);
    command
        .env("ANTHREX_WINDOW_ID", "9")
        .arg("-c")
        .arg(format!("notify={notify}"));

    let output = run(command);

    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let payload = json_file(&payload_path);
    assert_eq!(payload["type"], "agent-turn-complete");
    assert_eq!(payload["thread-id"], "fake-session-9");
}

#[test]
fn runs_codex_hook_commands_from_config_overrides() {
    let temp = tempfile::tempdir_in("/tmp").unwrap();
    let payload_path = temp.path().join("p.json");
    let script = script_file(
        &temp,
        &[json!({"hook": "Stop", "payload": {}}), json!({"exit": 0})],
    );
    let hook_command = format!("cat > {}", payload_path.display());
    let config = format!(
        "hooks.Stop=[{{hooks=[{{type=\"command\",command={}}}]}}]",
        serde_json::to_string(&hook_command).unwrap()
    );
    let mut command = command(&script);
    command.env("ANTHREX_WINDOW_ID", "9").arg("-c").arg(config);

    let output = run(command);

    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let payload = json_file(&payload_path);
    assert_eq!(payload["hook_event_name"], "Stop");
    assert_eq!(payload["session_id"], "fake-session-9");
}

#[test]
fn steps_without_a_registered_hook_are_skipped() {
    let temp = tempfile::tempdir_in("/tmp").unwrap();
    let script = script_file(
        &temp,
        &[
            json!({"hook": "PreToolUse", "payload": {}}),
            json!({"exit": 0}),
        ],
    );

    let output = run(command(&script));

    assert!(output.status.success());
    assert!(output.stderr.is_empty());
}

#[test]
fn title_and_bell_write_their_bytes() {
    let temp = tempfile::tempdir_in("/tmp").unwrap();
    let script = script_file(
        &temp,
        &[
            json!({"title": "Working"}),
            json!({"bell": true}),
            json!({"exit": 0}),
        ],
    );

    let output = run(command(&script));

    assert!(output.status.success());
    assert_eq!(output.stdout, b"\x1b]0;Working\x1b\\\x07");
}

#[test]
fn read_line_waits_for_input() {
    let temp = tempfile::tempdir_in("/tmp").unwrap();
    let script = script_file(
        &temp,
        &[
            json!({"read_line": true}),
            json!({"print": "after"}),
            json!({"exit": 0}),
        ],
    );
    let mut child = command(&script).spawn().unwrap();
    let mut stdout = child.stdout.take().unwrap();
    let (tx, rx) = mpsc::channel();
    let reader = std::thread::spawn(move || {
        let mut byte = [0];
        while stdout.read_exact(&mut byte).is_ok() {
            tx.send(byte[0]).unwrap();
        }
    });

    assert_eq!(
        rx.recv_timeout(Duration::from_millis(200)),
        Err(mpsc::RecvTimeoutError::Timeout)
    );
    child.stdin.take().unwrap().write_all(b"go\n").unwrap();
    assert_eq!(rx.recv_timeout(Duration::from_secs(2)).unwrap(), b'a');
    assert!(child.wait().unwrap().success());
    reader.join().unwrap();
}

#[test]
fn writes_its_arguments_when_asked() {
    let temp = tempfile::tempdir_in("/tmp").unwrap();
    let script = script_file(&temp, &[json!({"exit": 0})]);
    let args_path = temp.path().join("args.json");
    let mut command = command(&script);
    command
        .env("FAKE_AGENT_ARGS_FILE", &args_path)
        .args(["--name", "agent", "--", "prompt"]);

    let output = run(command);

    assert!(output.status.success());
    assert_eq!(
        json_file(&args_path),
        json!(["--name", "agent", "--", "prompt"])
    );
}

#[test]
fn version_prints_a_codex_style_version() {
    let output = Command::new(fake_agent())
        .arg("--version")
        .output()
        .unwrap();

    assert!(output.status.success());
    assert_eq!(output.stdout, b"codex-cli 0.155.0\n");
}

#[test]
fn writes_arguments_before_the_version_short_circuit() {
    let temp = tempfile::tempdir_in("/tmp").unwrap();
    let args_path = temp.path().join("args.json");
    let output = Command::new(fake_agent())
        .env("FAKE_AGENT_ARGS_FILE", &args_path)
        .args(["--name", "agent", "--version", "--", "literal prompt"])
        .output()
        .unwrap();

    assert!(output.status.success());
    assert_eq!(output.stdout, b"codex-cli 0.155.0\n");
    assert_eq!(
        json_file(&args_path),
        json!(["--name", "agent", "--version", "--", "literal prompt"])
    );
}

#[test]
fn git_commit_creates_a_commit() {
    let temp = tempfile::tempdir_in("/tmp").unwrap();
    assert!(
        Command::new("git")
            .arg("init")
            .arg(temp.path())
            .status()
            .unwrap()
            .success()
    );
    let script = script_file(
        &temp,
        &[
            json!({"git_commit": {"file": "a.txt", "content": "x", "message": "made by fake"}}),
            json!({"exit": 0}),
        ],
    );
    let mut command = command(&script);
    command.current_dir(temp.path());

    let output = run(command);

    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let log = Command::new("git")
        .args(["log", "--oneline", "-1"])
        .current_dir(temp.path())
        .output()
        .unwrap();
    assert!(String::from_utf8_lossy(&log.stdout).contains("made by fake"));
    assert_eq!(fs::read_to_string(temp.path().join("a.txt")).unwrap(), "x");
}

#[test]
fn mcp_call_is_not_supported_yet() {
    let temp = tempfile::tempdir_in("/tmp").unwrap();
    let script = script_file(
        &temp,
        &[json!({"mcp_call": {"tool": "report_done", "args": {}}})],
    );

    let output = run(command(&script));

    assert_eq!(output.status.code(), Some(3));
    assert_eq!(
        output.stdout,
        b"fake-agent: mcp_call arrives in milestone 8\n"
    );
}

#[test]
fn hook_timeout_includes_blocked_stdin_delivery() {
    let temp = tempfile::tempdir_in("/tmp").unwrap();
    let large = "x".repeat(2 * 1024 * 1024);
    let script = script_file(
        &temp,
        &[json!({"hook": "Stop", "payload": {"large": large}})],
    );
    let settings = json!({
        "hooks": {"Stop": [{"hooks": [{"command": "sleep 30"}]}]}
    });
    let mut command = command(&script);
    command.arg("--settings").arg(settings.to_string());

    let started = Instant::now();
    let output = run(command);

    assert!(!output.status.success());
    assert!(
        started.elapsed() >= Duration::from_millis(4_500),
        "hook did not wait for its timeout: {:?}",
        started.elapsed()
    );
    assert!(
        started.elapsed() < Duration::from_secs(8),
        "elapsed: {:?}",
        started.elapsed()
    );
}
