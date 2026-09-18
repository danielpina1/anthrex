use std::fs;
use std::io::{self, Read, Write};
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitStatus, Output, Stdio};
use std::sync::mpsc;
use std::thread;
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

fn run_with_deadline(mut command: Command, timeout: Duration) -> Result<Output, String> {
    command.stdout(Stdio::piped()).stderr(Stdio::piped());
    let (mut child, process_group) = spawn_owned(&mut command)?;
    drop(child.stdin.take());
    let Some(stdout) = child.stdout.take() else {
        terminate_owned(&mut child, process_group)?;
        return Err("child stdout was not piped".into());
    };
    let Some(stderr) = child.stderr.take() else {
        terminate_owned(&mut child, process_group)?;
        return Err("child stderr was not piped".into());
    };
    let stdout_rx = drain(stdout);
    let stderr_rx = drain(stderr);
    let deadline = Instant::now() + timeout;
    let mut status = None;
    let mut stdout = None;
    let mut stderr = None;

    loop {
        if status.is_none() {
            match child.try_wait() {
                Ok(next_status) => status = next_status,
                Err(error) => {
                    terminate_owned(&mut child, process_group)?;
                    return Err(error.to_string());
                }
            }
        }
        if stdout.is_none()
            && let Ok(result) = stdout_rx.try_recv()
        {
            match result {
                Ok(bytes) => stdout = Some(bytes),
                Err(error) => {
                    terminate_owned(&mut child, process_group)?;
                    return Err(error);
                }
            }
        }
        if stderr.is_none()
            && let Ok(result) = stderr_rx.try_recv()
        {
            match result {
                Ok(bytes) => stderr = Some(bytes),
                Err(error) => {
                    terminate_owned(&mut child, process_group)?;
                    return Err(error);
                }
            }
        }
        match (status.take(), stdout.take(), stderr.take()) {
            (Some(status), Some(stdout), Some(stderr)) => {
                return Ok(Output {
                    status,
                    stdout,
                    stderr,
                });
            }
            (next_status, next_stdout, next_stderr) => {
                status = next_status;
                stdout = next_stdout;
                stderr = next_stderr;
            }
        }
        if Instant::now() >= deadline {
            terminate_owned(&mut child, process_group)?;
            return Err(format!("owned child timed out after {timeout:?}"));
        }
        thread::sleep(Duration::from_millis(5));
    }
}

fn run(command: Command) -> Output {
    run_with_deadline(command, Duration::from_secs(10)).unwrap()
}

fn spawn_owned(command: &mut Command) -> Result<(Child, libc::pid_t), String> {
    // SAFETY: setpgid is async-signal-safe and the closure captures no Rust state.
    unsafe {
        command.pre_exec(|| {
            if libc::setpgid(0, 0) == -1 {
                return Err(io::Error::last_os_error());
            }
            Ok(())
        });
    }
    let child = command.spawn().map_err(|error| error.to_string())?;
    let process_group = child.id() as libc::pid_t;
    Ok((child, process_group))
}

fn drain(mut pipe: impl Read + Send + 'static) -> mpsc::Receiver<Result<Vec<u8>, String>> {
    let (tx, rx) = mpsc::channel();
    thread::spawn(move || {
        let mut bytes = Vec::new();
        let result = pipe
            .read_to_end(&mut bytes)
            .map(|_| bytes)
            .map_err(|error| error.to_string());
        let _ = tx.send(result);
    });
    rx
}

fn wait_owned(
    child: &mut Child,
    process_group: libc::pid_t,
    timeout: Duration,
) -> Result<ExitStatus, String> {
    let deadline = Instant::now() + timeout;
    loop {
        match child.try_wait() {
            Ok(Some(status)) => return Ok(status),
            Ok(None) => {}
            Err(error) => {
                terminate_owned(child, process_group)?;
                return Err(error.to_string());
            }
        }
        if Instant::now() >= deadline {
            terminate_owned(child, process_group)?;
            return Err(format!("owned child timed out after {timeout:?}"));
        }
        thread::sleep(Duration::from_millis(5));
    }
}

fn terminate_owned(child: &mut Child, process_group: libc::pid_t) -> Result<(), String> {
    // SAFETY: the negative PID targets only the fresh process group created by spawn_owned.
    unsafe {
        libc::kill(-process_group, libc::SIGKILL);
    }
    let _ = child.kill();
    let deadline = Instant::now() + Duration::from_millis(500);
    while Instant::now() < deadline {
        match child.try_wait() {
            Ok(Some(_)) => return Ok(()),
            Ok(None) => thread::sleep(Duration::from_millis(5)),
            Err(error) => return Err(format!("failed to reap owned child: {error}")),
        }
    }
    Err("failed to reap owned child within 500ms".into())
}

fn json_file(path: &Path) -> Value {
    serde_json::from_slice(&fs::read(path).unwrap()).unwrap()
}

#[test]
fn subprocess_deadline_stops_a_hung_owned_child() {
    let temp = tempfile::tempdir_in("/tmp").unwrap();
    let pid_file = temp.path().join("child.pid");
    let mut command = Command::new("/bin/sh");
    command.args(["-c", &format!("echo $$ > {}; sleep 1", pid_file.display())]);
    let started = Instant::now();

    let error = run_with_deadline(command, Duration::from_millis(200)).unwrap_err();

    assert!(error.contains("timed out"), "{error}");
    assert!(
        started.elapsed() < Duration::from_secs(1),
        "elapsed: {:?}",
        started.elapsed()
    );
    let pid: libc::pid_t = fs::read_to_string(pid_file)
        .unwrap()
        .trim()
        .parse()
        .unwrap();
    // SAFETY: signal 0 performs a read-only existence check on the exact owned PID.
    assert_eq!(unsafe { libc::kill(pid, 0) }, -1);
    assert_eq!(io::Error::last_os_error().raw_os_error(), Some(libc::ESRCH));
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
fn noninteractive_runner_closes_stdin_for_script_eof() {
    let temp = tempfile::tempdir_in("/tmp").unwrap();
    let script = script_file(&temp, &[json!({"print": "done"})]);

    let output = run_with_deadline(command(&script), Duration::from_millis(500)).unwrap();

    assert!(output.status.success());
    assert_eq!(output.stdout, b"done");
    assert!(output.stderr.is_empty());
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
    let mut child_command = command(&script);
    let (mut child, process_group) = spawn_owned(&mut child_command).unwrap();
    let mut stdout = child.stdout.take().unwrap();
    let stderr_rx = drain(child.stderr.take().unwrap());
    let (tx, rx) = mpsc::channel();
    let (done_tx, done_rx) = mpsc::channel();
    let reader = thread::spawn(move || {
        let mut byte = [0];
        while stdout.read_exact(&mut byte).is_ok() {
            tx.send(byte[0]).unwrap();
        }
        let _ = done_tx.send(());
    });

    let before_input = rx.recv_timeout(Duration::from_millis(200));
    let input_result = child.stdin.take().unwrap().write_all(b"go\n");
    let after_input = rx.recv_timeout(Duration::from_secs(2));
    let status = wait_owned(&mut child, process_group, Duration::from_secs(2));
    let stderr = stderr_rx.recv_timeout(Duration::from_secs(2));
    let reader_done = done_rx.recv_timeout(Duration::from_secs(2));
    if reader_done.is_ok() {
        reader.join().unwrap();
    }

    assert_eq!(
        before_input,
        Err(mpsc::RecvTimeoutError::Timeout),
        "fake agent produced output before input"
    );
    input_result.unwrap();
    assert_eq!(after_input.unwrap(), b'a');
    assert!(status.unwrap().success());
    assert!(stderr.unwrap().unwrap().is_empty());
    reader_done.unwrap();
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
    let mut command = Command::new(fake_agent());
    command.arg("--version");
    let output = run(command);

    assert!(output.status.success());
    assert_eq!(output.stdout, b"codex-cli 0.155.0\n");
}

#[test]
fn writes_arguments_before_the_version_short_circuit() {
    let temp = tempfile::tempdir_in("/tmp").unwrap();
    let args_path = temp.path().join("args.json");
    let mut command = Command::new(fake_agent());
    command.env("FAKE_AGENT_ARGS_FILE", &args_path).args([
        "--name",
        "agent",
        "--version",
        "--",
        "literal prompt",
    ]);
    let output = run(command);

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
    let mut init = Command::new("git");
    init.arg("init").arg(temp.path());
    assert!(run(init).status.success());
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
    let mut log_command = Command::new("git");
    log_command
        .args(["log", "--oneline", "-1"])
        .current_dir(temp.path());
    let log = run(log_command);
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
    let output = run_with_deadline(command, Duration::from_secs(8)).unwrap();

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
