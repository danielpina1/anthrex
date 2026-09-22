mod runtime;
mod script;

use std::env;
use std::fs::{self, File, OpenOptions};
use std::io::{self, BufRead, BufReader, Write};
use std::os::unix::process::CommandExt;
use std::process::{Child, Command, ExitStatus, Stdio};
use std::sync::mpsc::{self, Receiver};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail};
use serde_json::{Map, Value};

use runtime::Runtime;
use script::Step;

const STEP_TIMEOUT: Duration = Duration::from_secs(5);

fn main() {
    let code = match run() {
        Ok(code) => code,
        Err(error) => {
            eprintln!("fake-agent: {error:#}");
            1
        }
    };
    std::process::exit(code);
}

fn run() -> Result<i32> {
    let args: Vec<String> = env::args().skip(1).collect();
    write_args(&args)?;
    if args.iter().any(|arg| arg == "--version") {
        println!("codex-cli 0.155.0");
        return Ok(0);
    }

    let runtime = runtime::discover(&args)?;
    let Some(script_path) = env::var_os("FAKE_AGENT_SCRIPT") else {
        println!("fake-agent: no script (FAKE_AGENT_SCRIPT is unset)");
        wait_for_eof()?;
        return Ok(0);
    };
    let script_file = match File::open(&script_path) {
        Ok(file) => file,
        Err(error) => {
            println!(
                "fake-agent: no script ({}: {error})",
                script_path.to_string_lossy()
            );
            wait_for_eof()?;
            return Ok(0);
        }
    };
    let steps = match script::parse_script(BufReader::new(script_file)) {
        Ok(steps) => steps,
        Err(error) => {
            eprintln!("fake-agent: {error:#}");
            return Ok(2);
        }
    };

    let stdin = io::stdin();
    let mut input = BufReader::new(stdin.lock());
    let stdout = io::stdout();
    let mut output = stdout.lock();
    for step in steps {
        if let Some(code) = run_step(step, &runtime, &mut input, &mut output)? {
            return Ok(code);
        }
    }
    io::copy(&mut input, &mut io::sink()).context("read stdin to EOF")?;
    Ok(0)
}

fn write_args(args: &[String]) -> Result<()> {
    let Some(path) = env::var_os("FAKE_AGENT_ARGS_FILE") else {
        return Ok(());
    };
    let encoded = serde_json::to_vec(args).context("encode argv")?;
    fs::write(&path, encoded).with_context(|| format!("write argv to {}", path.to_string_lossy()))
}

fn wait_for_eof() -> Result<()> {
    io::copy(&mut io::stdin().lock(), &mut io::sink()).context("read stdin to EOF")?;
    Ok(())
}

fn run_step(
    step: Step,
    runtime: &Runtime,
    input: &mut impl BufRead,
    output: &mut impl Write,
) -> Result<Option<i32>> {
    match step {
        Step::Print(text) => {
            output
                .write_all(text.as_bytes())
                .context("write print step")?;
            output.flush().context("flush print step")?;
        }
        Step::Hook { event, mut payload } => {
            if let Some(command) = runtime.hook(&event) {
                fill_hook_payload(&mut payload, &event)?;
                run_hook(command, &payload)?;
            }
        }
        Step::Notify(mut payload) => {
            if let Some(command) = runtime.notify() {
                fill_notify_payload(&mut payload)?;
                run_notify(command, &payload)?;
            }
        }
        Step::Title(title) => {
            write!(output, "\x1b]0;{title}\x1b\\").context("write title step")?;
            output.flush().context("flush title step")?;
        }
        Step::Bell => {
            output.write_all(b"\x07").context("write bell step")?;
            output.flush().context("flush bell step")?;
        }
        Step::WaitMs(milliseconds) => thread::sleep(Duration::from_millis(milliseconds)),
        Step::ReadLine => {
            let mut line = String::new();
            input.read_line(&mut line).context("read line from stdin")?;
        }
        Step::GitCommit {
            file,
            content,
            message,
        } => git_commit(&file, &content, &message)?,
        Step::Transcript(entry) => write_transcript(&entry)?,
        Step::Exit(code) => return Ok(Some(code)),
        Step::McpCall { tool, args } => {
            let _ = (tool, args);
            writeln!(output, "fake-agent: mcp_call arrives in milestone 8")
                .context("write unsupported mcp_call message")?;
            output.flush().context("flush mcp_call message")?;
            return Ok(Some(3));
        }
    }
    Ok(None)
}

fn write_transcript(entry: &Value) -> Result<()> {
    let Some(path) = env::var_os("FAKE_AGENT_TRANSCRIPT") else {
        return Ok(());
    };
    let mut line = serde_json::to_string(entry).context("encode transcript entry")?;
    line.push('\n');
    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .with_context(|| format!("open transcript file {}", path.to_string_lossy()))?;
    file.write_all(line.as_bytes())
        .context("write transcript entry")?;
    file.flush().context("flush transcript entry")?;
    Ok(())
}

fn default_session_id() -> String {
    let suffix = env::var("ANTHREX_WINDOW_ID").unwrap_or_else(|_| std::process::id().to_string());
    format!("fake-session-{suffix}")
}

fn payload_object(payload: &mut Value) -> Result<&mut Map<String, Value>> {
    payload.as_object_mut().context("payload must be an object")
}

fn fill_hook_payload(payload: &mut Value, event: &str) -> Result<()> {
    let object = payload_object(payload)?;
    object
        .entry("hook_event_name")
        .or_insert_with(|| Value::String(event.to_owned()));
    object
        .entry("session_id")
        .or_insert_with(|| Value::String(default_session_id()));
    if !object.contains_key("cwd") {
        let cwd = env::current_dir()
            .context("read current directory")?
            .to_string_lossy()
            .into_owned();
        object.insert("cwd".into(), Value::String(cwd));
    }
    if let Some(transcript_path) = env::var_os("FAKE_AGENT_TRANSCRIPT") {
        object
            .entry("transcript_path")
            .or_insert_with(|| Value::String(transcript_path.to_string_lossy().into_owned()));
    }
    Ok(())
}

fn fill_notify_payload(payload: &mut Value) -> Result<()> {
    let object = payload_object(payload)?;
    object
        .entry("type")
        .or_insert_with(|| Value::String("agent-turn-complete".into()));
    object
        .entry("thread-id")
        .or_insert_with(|| Value::String(default_session_id()));
    Ok(())
}

fn run_hook(command: &str, payload: &Value) -> Result<()> {
    let deadline = Instant::now() + STEP_TIMEOUT;
    let bytes = serde_json::to_vec(payload).context("encode hook payload")?;
    let mut child_command = Command::new("/bin/sh");
    child_command.args(["-c", command]).stdin(Stdio::piped());
    isolate_process_group(&mut child_command);
    let mut child = child_command.spawn().context("spawn hook command")?;
    let process_group = child.id() as libc::pid_t;
    let mut stdin = child.stdin.take().context("hook stdin was not piped")?;
    let (writer_tx, writer_rx) = mpsc::channel();
    let writer = thread::spawn(move || {
        let result = stdin.write_all(&bytes).context("write hook payload");
        let _ = writer_tx.send(result);
    });
    wait_for_child(
        &mut child,
        process_group,
        deadline,
        Some((writer_rx, writer)),
    )
}

fn run_notify(command: &[String], payload: &Value) -> Result<()> {
    let (program, args) = command.split_first().context("notify argv is empty")?;
    let deadline = Instant::now() + STEP_TIMEOUT;
    let mut child_command = Command::new(program);
    child_command
        .args(args)
        .arg(serde_json::to_string(payload).context("encode notify payload")?);
    child_command.stdin(Stdio::null());
    isolate_process_group(&mut child_command);
    let mut child = child_command.spawn().context("spawn notify command")?;
    let process_group = child.id() as libc::pid_t;
    wait_for_child(&mut child, process_group, deadline, None)
}

type Writer = (Receiver<Result<()>>, JoinHandle<()>);

fn wait_for_child(
    child: &mut Child,
    process_group: libc::pid_t,
    deadline: Instant,
    writer: Option<Writer>,
) -> Result<()> {
    let mut status: Option<ExitStatus> = None;
    let mut writer_result = if writer.is_none() { Some(Ok(())) } else { None };
    while Instant::now() < deadline {
        if status.is_none() {
            status = child.try_wait().context("wait for step command")?;
        }
        if writer_result.is_none()
            && let Some((receiver, _)) = writer.as_ref()
            && let Ok(result) = receiver.try_recv()
        {
            writer_result = Some(result);
        }
        if status.is_some() && writer_result.is_some() {
            break;
        }
        thread::sleep(Duration::from_millis(10));
    }

    if status.is_none() || writer_result.is_none() {
        kill_process_group(process_group);
        let _ = child.kill();
        let cleanup_deadline = Instant::now() + Duration::from_millis(500);
        while status.is_none() && Instant::now() < cleanup_deadline {
            status = child.try_wait().context("reap timed-out step command")?;
            if status.is_none() {
                thread::sleep(Duration::from_millis(10));
            }
        }
        // Do not join a writer that may still be blocked in write(2). Returning
        // makes the fake agent exit, which closes its copy of the pipe.
        drop(writer);
        bail!("step timed out after 5 seconds");
    }

    if let Some((_, handle)) = writer {
        handle
            .join()
            .map_err(|_| anyhow::anyhow!("hook payload writer panicked"))?;
    }
    writer_result.context("writer result missing")??;
    let status = status.context("child status missing")?;
    if !status.success() {
        bail!("step command exited with {status}");
    }
    Ok(())
}

fn isolate_process_group(command: &mut Command) {
    // SAFETY: setpgid is async-signal-safe and the closure captures no Rust state.
    unsafe {
        command.pre_exec(|| {
            if libc::setpgid(0, 0) == -1 {
                return Err(io::Error::last_os_error());
            }
            Ok(())
        });
    }
}

fn kill_process_group(process_group: libc::pid_t) {
    // SAFETY: the negative PID targets only the fresh process group created above.
    unsafe {
        libc::kill(-process_group, libc::SIGKILL);
    }
}

fn git_commit(file: &str, content: &str, message: &str) -> Result<()> {
    fs::write(file, content).with_context(|| format!("write {file}"))?;
    let add = Command::new("git")
        .args(["add", "--", file])
        .status()
        .context("run git add")?;
    if !add.success() {
        bail!("git add exited with {add}");
    }
    let commit = Command::new("git")
        .args([
            "-c",
            "user.name=fake-agent",
            "-c",
            "user.email=fake-agent@example.invalid",
            "commit",
            "-m",
            message,
        ])
        .status()
        .context("run git commit")?;
    if !commit.success() {
        bail!("git commit exited with {commit}");
    }
    Ok(())
}
