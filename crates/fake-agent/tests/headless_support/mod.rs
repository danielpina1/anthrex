//! Shared by `headless_modes.rs`: owned `fake-agent` processes on real pipes, the argv
//! the daemon's headless launcher builds (`daemon::headless::argv`), a temporary git
//! repository with per-role scripts, a stub daemon for `anthrex mcp`, and decision 51's
//! shape check against M8a.1's recorded fixtures. No real daemon, no real agent.

#![allow(dead_code)]

use std::fs;
use std::io::{BufRead, BufReader, Read, Write};
use std::os::unix::net::UnixListener;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, ExitStatus, Stdio};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError};
use std::sync::{Arc, Mutex, OnceLock};
use std::thread;
use std::time::{Duration, Instant};

mod shape;
pub use shape::assert_conforms;

use daemon::headless::argv;
use daemon::headless::{HeadlessSpec, McpTarget, SessionArg};
use proto::{AgentRole, ClientMsg, DaemonMsg, Effort, RunReply, RunRequest, Runtime, ToolCall};
use serde_json::{Value, json};

/// A bound on one `fake-agent` process that makes no MCP call: its own steps are all
/// scripted in milliseconds here, and its only other bound is `STEP_TIMEOUT` (5 s) on
/// a hook, which these tests never run.
pub const RUN: Duration = Duration::from_secs(20);
/// A bound on a process that makes an MCP call: `fake-agent`'s `MCP_CALL_TIMEOUT`
/// (120 s, Codex's own `tool_timeout_sec`) plus slack. The stub answers at once.
pub const MCP_RUN: Duration = Duration::from_secs(150);

pub fn fake_agent() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_fake-agent"))
}

/// The real `anthrex` binary for `anthrex mcp`. `cargo test --workspace` builds it next
/// to `fake-agent`; `cargo test -p anthrex-fake-agent` does not, so it is built once,
/// into its own target directory (the outer `cargo` may hold the shared one's lock).
pub fn anthrex() -> PathBuf {
    static BIN: OnceLock<PathBuf> = OnceLock::new();
    BIN.get_or_init(|| {
        let sibling = fake_agent().with_file_name("anthrex");
        if sibling.is_file() {
            return sibling;
        }
        let manifest = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../Cargo.toml");
        let target = fake_agent()
            .parent()
            .and_then(Path::parent)
            .expect("fake-agent lives in <target>/<profile>/")
            .join("fake-agent-anthrex");
        let status = Command::new(option_env!("CARGO").unwrap_or("cargo"))
            .args([
                "build",
                "-p",
                "anthrex",
                "--bin",
                "anthrex",
                "--manifest-path",
            ])
            .arg(&manifest)
            .arg("--target-dir")
            .arg(&target)
            .stdin(Stdio::null())
            .status()
            .expect("run cargo build for anthrex");
        assert!(status.success(), "building anthrex failed: {status}");
        let built = target.join("debug/anthrex");
        assert!(built.is_file(), "{} was not built", built.display());
        built
    })
    .clone()
}

pub fn tempdir() -> tempfile::TempDir {
    tempfile::Builder::new()
        .prefix("ax-fa-")
        .tempdir_in("/tmp")
        .unwrap()
}

/// `git` with `--no-optional-locks`, a scrubbed environment and no user or system
/// config (AGENTS.md rule 11). Panics on failure; returns trimmed stdout.
pub fn git(dir: &Path, args: &[&str]) -> String {
    let output = Command::new("git")
        .arg("--no-optional-locks")
        .args(["-c", "user.name=t", "-c", "user.email=t@example.invalid"])
        .args(["-c", "commit.gpgsign=false"])
        .args(args)
        .current_dir(dir)
        .env_remove("GIT_DIR")
        .env_remove("GIT_WORK_TREE")
        .env_remove("GIT_COMMON_DIR")
        .env_remove("GIT_INDEX_FILE")
        .env_remove("GIT_PREFIX")
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .stdin(Stdio::null())
        .output()
        .unwrap();
    assert!(output.status.success(), "git {args:?}: {output:?}");
    String::from_utf8(output.stdout).unwrap().trim().to_string()
}

/// A repository at `<dir>/repo` with one commit.
pub fn repo(dir: &Path) -> PathBuf {
    let repo = dir.join("repo");
    fs::create_dir(&repo).unwrap();
    git(&repo, &["init", "-q", "-b", "main"]);
    fs::write(repo.join("README"), "hi\n").unwrap();
    git(&repo, &["add", "README"]);
    git(&repo, &["commit", "-q", "-m", "first"]);
    repo
}

/// Writes `<repo>/.git/fake-agent/<name>.jsonl`, the per-role script directory.
pub fn role_script(repo: &Path, name: &str, steps: &[Value]) -> PathBuf {
    let dir = repo.join(".git/fake-agent");
    fs::create_dir_all(&dir).unwrap();
    write_steps(&dir.join(format!("{name}.jsonl")), steps)
}

pub fn write_steps(path: &Path, steps: &[Value]) -> PathBuf {
    let text: String = steps.iter().map(|s| format!("{s}\n")).collect();
    fs::write(path, text).unwrap();
    path.to_path_buf()
}

/// Where `anthrex mcp` is told to find its daemon, and whose role and task it serves.
#[derive(Clone)]
pub struct Mcp {
    pub exe: PathBuf,
    pub role: &'static str,
    pub task: Option<&'static str>,
    pub socket: PathBuf,
}

impl Mcp {
    /// A role and task for script claiming, with a server that is never started.
    pub fn unused(role: &'static str, task: &'static str) -> Self {
        Self {
            exe: PathBuf::from("/nonexistent/anthrex"),
            role,
            task: Some(task),
            socket: PathBuf::from("/tmp/nonexistent.sock"),
        }
    }

    fn target(&self) -> McpTarget {
        McpTarget {
            role: match self.role {
                "worker" => AgentRole::Worker,
                "reviewer" => AgentRole::Reviewer,
                _ => AgentRole::Orchestrator,
            },
            run_id: "r1".into(),
            task_id: self.task.map(String::from),
        }
    }
}

/// The window id every argv here is built for.
pub const WINDOW: u32 = 7;

/// `SessionArg` of the argv builders.
pub enum Session<'a> {
    New(&'a str),
    Resume(&'a str),
}

/// A worker's spec, as the engine builds one, with `mcp` as its MCP target.
fn spec(runtime: Runtime, mcp: Option<&Mcp>) -> HeadlessSpec {
    HeadlessSpec {
        runtime,
        model: "sonnet".into(),
        effort: Effort::Low,
        cwd: PathBuf::from("/tmp/unused"),
        instructions: "be brief -- and -p \"quoted\"\nsecond line".into(),
        mcp: mcp.map(Mcp::target),
        allowed_tools: vec!["mcp__anthrex__task_done".into(), "Bash".into()],
        claude_permission_mode: Some("acceptEdits".into()),
        claude_disallowed_tools: vec![],
        claude_sandbox: None,
        codex_sandbox: "workspace-write".into(),
        codex_writable_roots: vec![],
        env: vec![],
        claude_auth: config::ClaudeAuth::Login,
        api_key_helper: None,
        run_ref: None,
        codex_config_guard: None,
    }
}

fn exe_and_socket(mcp: Option<&Mcp>) -> (PathBuf, PathBuf) {
    match mcp {
        Some(mcp) => (mcp.exe.clone(), mcp.socket.clone()),
        None => (
            "/nonexistent/anthrex".into(),
            "/tmp/nonexistent.sock".into(),
        ),
    }
}

/// The daemon's own Claude argv (`daemon::headless::argv::claude_args`, M8a.1's caps).
pub fn claude_argv(session: Session, mcp: Option<&Mcp>) -> Vec<String> {
    let session = match session {
        Session::New(id) => SessionArg::New {
            uuid: Some(id.into()),
        },
        Session::Resume(id) => SessionArg::Resume {
            session_id: id.into(),
        },
    };
    let (exe, socket) = exe_and_socket(mcp);
    let spec = spec(Runtime::Claude, mcp);
    argv::claude_args(&spec, &session, &exe, WINDOW, &socket, &argv::CLI_CAPS)
}

/// The daemon's own Codex argv (`daemon::headless::argv::codex_args`). With `exclude`,
/// the caps carry decision 53's test placeholder flag.
pub fn codex_argv(
    resume: Option<&str>,
    mcp: Option<&Mcp>,
    exclude: bool,
    msg: &str,
) -> Vec<String> {
    let session = match resume {
        Some(id) => SessionArg::Resume {
            session_id: id.into(),
        },
        None => SessionArg::New { uuid: None },
    };
    let mut caps = argv::CLI_CAPS;
    if exclude {
        caps.codex_user_config_only = Some(&["--anthrex-test-exclude-project-config"]);
    }
    let (exe, socket) = exe_and_socket(mcp);
    let spec = spec(Runtime::Codex, mcp);
    argv::codex_args(&spec, &session, msg, &exe, WINDOW, &socket, &caps)
}

/// M8a.1's accepted stream-json user message (`claude_stream::user_message`).
pub fn user_message(text: &str, session: &str) -> String {
    json!({
        "type": "user",
        "message": {"role": "user", "content": [{"type": "text", "text": text}]},
        "parent_tool_use_id": null,
        "session_id": session,
    })
    .to_string()
}

pub fn interrupt(id: &str) -> String {
    json!({"type": "control_request", "request_id": id, "request": {"subtype": "interrupt"}})
        .to_string()
}

/// One owned `fake-agent` process in its own process group, killed on drop.
pub struct Agent {
    child: Child,
    group: libc::pid_t,
    stdin: Option<ChildStdin>,
    lines: Receiver<String>,
    pub seen: Vec<Value>,
    stderr: Arc<Mutex<Vec<u8>>>,
}

impl Agent {
    pub fn spawn(args: &[String], cwd: &Path, env: &[(&str, &Path)]) -> Self {
        let mut command = Command::new(fake_agent());
        command
            .args(args)
            .current_dir(cwd)
            .env_remove("FAKE_AGENT_SCRIPT")
            .env_remove("FAKE_AGENT_ARGS_FILE")
            .env_remove("FAKE_AGENT_STDIN_FILE")
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("ANTHREX_SOCKET", cwd.join("never.sock"))
            .env("ANTHREX_DATA_DIR", cwd.join("never-data"))
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        for (key, value) in env {
            command.env(key, value);
        }
        // SAFETY: setpgid is async-signal-safe and the closure captures no Rust state.
        unsafe {
            command.pre_exec(|| {
                if libc::setpgid(0, 0) == -1 {
                    return Err(std::io::Error::last_os_error());
                }
                Ok(())
            });
        }
        let mut child = command.spawn().unwrap();
        let group = child.id() as libc::pid_t;
        let stdout = child.stdout.take().unwrap();
        let (tx, lines) = mpsc::channel();
        thread::spawn(move || {
            for line in BufReader::new(stdout).lines() {
                let Ok(line) = line else { return };
                if tx.send(line).is_err() {
                    return;
                }
            }
        });
        let stderr = Arc::new(Mutex::new(Vec::new()));
        let mut pipe = child.stderr.take().unwrap();
        let sink = stderr.clone();
        thread::spawn(move || {
            let mut buf = [0u8; 4096];
            while let Ok(n) = pipe.read(&mut buf) {
                if n == 0 {
                    return;
                }
                sink.lock().unwrap().extend_from_slice(&buf[..n]);
            }
        });
        Self {
            stdin: child.stdin.take(),
            child,
            group,
            lines,
            seen: Vec::new(),
            stderr,
        }
    }

    /// A Codex turn's process, with stdin closed at once as the daemon's driver does.
    pub fn codex(args: &[String], cwd: &Path, env: &[(&str, &Path)]) -> Self {
        let mut agent = Self::spawn(args, cwd, env);
        agent.close_stdin();
        agent
    }

    pub fn send(&mut self, line: &str) {
        let stdin = self.stdin.as_mut().expect("stdin still open");
        stdin.write_all(format!("{line}\n").as_bytes()).unwrap();
        stdin.flush().unwrap();
    }

    pub fn close_stdin(&mut self) {
        self.stdin.take();
    }

    /// The next stdout line within `timeout`, if any; every line must be JSON.
    pub fn next_within(&mut self, timeout: Duration) -> Option<Value> {
        match self.lines.recv_timeout(timeout) {
            Ok(line) => {
                let value: Value = serde_json::from_str(&line)
                    .unwrap_or_else(|e| panic!("stdout line is not JSON: {line:?}: {e}"));
                self.seen.push(value.clone());
                Some(value)
            }
            Err(RecvTimeoutError::Timeout | RecvTimeoutError::Disconnected) => None,
        }
    }

    /// Reads lines until one satisfies `pred`, within `timeout`.
    pub fn until(&mut self, timeout: Duration, pred: impl Fn(&Value) -> bool) -> Value {
        let deadline = Instant::now() + timeout;
        loop {
            let left = deadline.saturating_duration_since(Instant::now());
            match self.next_within(left.min(Duration::from_millis(200))) {
                Some(value) if pred(&value) => return value,
                Some(_) => {}
                None if Instant::now() >= deadline => panic!(
                    "no matching line within {timeout:?}; seen {:#?}; stderr {}",
                    self.seen,
                    self.stderr()
                ),
                None => {}
            }
        }
    }

    /// Waits for the exit, reading every remaining stdout line into `seen`.
    pub fn wait(&mut self, timeout: Duration) -> ExitStatus {
        let deadline = Instant::now() + timeout;
        loop {
            while self.next_within(Duration::from_millis(0)).is_some() {}
            if let Some(status) = self.child.try_wait().unwrap() {
                // Drain what the reader thread still holds.
                while self.next_within(Duration::from_millis(200)).is_some() {}
                return status;
            }
            assert!(
                Instant::now() < deadline,
                "fake-agent did not exit within {timeout:?}; seen {:#?}; stderr {}",
                self.seen,
                self.stderr()
            );
            thread::sleep(Duration::from_millis(10));
        }
    }

    /// Sends `signal` to the agent's whole process group, as the daemon's kill does.
    pub fn signal_group(&self, signal: libc::c_int) {
        // SAFETY: the negative PID targets only the fresh process group made at spawn.
        unsafe {
            libc::kill(-self.group, signal);
        }
    }

    /// Sends `signal` to the agent alone, as the daemon's Codex interrupt does.
    pub fn signal(&self, signal: libc::c_int) {
        // SAFETY: the pid of our own unreaped child.
        unsafe {
            libc::kill(self.group, signal);
        }
    }

    pub fn is_running(&mut self) -> bool {
        self.child.try_wait().unwrap().is_none()
    }

    pub fn stderr(&self) -> String {
        String::from_utf8_lossy(&self.stderr.lock().unwrap()).into_owned()
    }

    pub fn of_type(&self, kind: &str) -> Vec<Value> {
        self.seen
            .iter()
            .filter(|v| v["type"] == kind)
            .cloned()
            .collect()
    }
}

impl Drop for Agent {
    fn drop(&mut self) {
        // SAFETY: the negative PID targets only the fresh process group made at spawn.
        unsafe {
            libc::kill(-self.group, libc::SIGKILL);
        }
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// A stub daemon on a `/tmp` socket: `Welcome` to every `Hello`, then the scripted
/// `ToolResult { ok, text }` for each `Run(Tool(..))`, recording every call.
pub struct StubDaemon {
    pub socket: PathBuf,
    calls: Arc<Mutex<Vec<ToolCall>>>,
    _dir: tempfile::TempDir,
}

impl StubDaemon {
    pub fn start(ok: bool, text: &str) -> Self {
        let dir = tempdir();
        let socket = dir.path().join("d.sock");
        let listener = UnixListener::bind(&socket).unwrap();
        let calls = Arc::new(Mutex::new(Vec::new()));
        let record = calls.clone();
        let text = text.to_string();
        thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(mut stream) = stream else { return };
                let Some(ClientMsg::Hello { .. }) = read_frame(&mut stream) else {
                    continue;
                };
                write_frame(
                    &mut stream,
                    &DaemonMsg::Welcome {
                        daemon_version: "stub".into(),
                        windows: vec![],
                    },
                );
                if let Some(ClientMsg::Run(RunRequest::Tool(call))) = read_frame(&mut stream) {
                    record.lock().unwrap().push(call);
                    let reply = RunReply::ToolResult {
                        ok,
                        text: text.clone(),
                    };
                    write_frame(&mut stream, &DaemonMsg::Run(reply));
                }
            }
        });
        Self {
            socket,
            calls,
            _dir: dir,
        }
    }

    pub fn calls(&self) -> Vec<ToolCall> {
        self.calls.lock().unwrap().clone()
    }

    pub fn mcp(&self, role: &'static str, task: &'static str) -> Mcp {
        Mcp {
            exe: anthrex(),
            role,
            task: Some(task),
            socket: self.socket.clone(),
        }
    }
}

fn read_frame(stream: &mut impl Read) -> Option<ClientMsg> {
    let mut header = [0u8; 4];
    stream.read_exact(&mut header).ok()?;
    let mut body = vec![0u8; u32::from_be_bytes(header) as usize];
    stream.read_exact(&mut body).ok()?;
    proto::decode(&body).ok()
}

fn write_frame(stream: &mut impl Write, msg: &DaemonMsg) {
    let _ = stream.write_all(&proto::encode(msg).unwrap());
}

/// The assistant texts, Claude's or Codex's.
pub fn texts(agent: &Agent) -> Vec<String> {
    agent
        .seen
        .iter()
        .filter_map(|v| {
            if v["type"] == "item.completed" && v["item"]["type"] == "agent_message" {
                return v["item"]["text"].as_str().map(String::from);
            }
            if v["type"] == "assistant" && v["message"]["model"] != "<synthetic>" {
                let block = &v["message"]["content"][0];
                if block["type"] == "text" {
                    return block["text"].as_str().map(String::from);
                }
            }
            None
        })
        .collect()
}

pub fn is_result(v: &Value) -> bool {
    v["type"] == "result"
}

pub fn thread_id(agent: &Agent) -> String {
    agent.of_type("thread.started")[0]["thread_id"]
        .as_str()
        .unwrap()
        .to_string()
}

pub fn lines(path: &Path) -> Vec<String> {
    fs::read_to_string(path)
        .unwrap_or_else(|e| panic!("{}: {e}", path.display()))
        .lines()
        .map(String::from)
        .collect()
}

/// A recorded fixture file of M8a.1.
pub fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../daemon/tests/fixtures/headless")
        .join(name)
}

/// Whether `pid` still exists (signal 0).
pub fn alive(pid: libc::pid_t) -> bool {
    // SAFETY: signal 0 only checks that the pid exists.
    unsafe { libc::kill(pid, 0) == 0 }
}

/// Waits for `path` to hold a pid, within `RUN`.
pub fn wait_pid(path: &Path) -> libc::pid_t {
    let deadline = Instant::now() + RUN;
    loop {
        if let Some(pid) = fs::read_to_string(path)
            .ok()
            .and_then(|text| text.trim().parse().ok())
        {
            return pid;
        }
        assert!(
            Instant::now() < deadline,
            "{} never held a pid",
            path.display()
        );
        thread::sleep(Duration::from_millis(10));
    }
}

/// Waits, within `RUN`, for `pid` to be gone; kills it first if it is not, so a
/// failing test leaves nothing behind.
pub fn assert_gone(pid: libc::pid_t, what: &str) {
    let deadline = Instant::now() + RUN;
    while alive(pid) {
        if Instant::now() >= deadline {
            // SAFETY: cleanup of the leftover process the assertion is about.
            unsafe {
                libc::kill(pid, libc::SIGKILL);
            }
            panic!("{what}: pid {pid} outlived the agent");
        }
        thread::sleep(Duration::from_millis(10));
    }
}
