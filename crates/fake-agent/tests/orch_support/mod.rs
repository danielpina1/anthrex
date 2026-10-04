//! Shared by `orch_modes.rs` (M9.12): the orchestrator's argv from the daemon's own
//! launcher, a stand-in `anthrex` that logs hooks and runs the real `anthrex mcp`, and
//! an owned `fake-agent` on a real PTY.

use std::fs;
use std::io::{Read, Write};
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};
use std::thread;
use std::time::{Duration, Instant};

use crate::headless_support::*;
use daemon::headless::McpTarget;
use daemon::headless::argv::CLI_CAPS;
use daemon::launch::role::{ORCHESTRATOR_ALLOWED_TOOLS, ORCHESTRATOR_DISALLOWED_TOOLS, RoleLaunch};
use daemon::launch::{LaunchContext, plan};
use portable_pty::{CommandBuilder, MasterPty, PtySize, native_pty_system};
use proto::{AgentRole, Effort, RunRef, Runtime, WindowSpec};
use serde_json::Value;

pub const RUN_ID: &str = "r-7a2c";
pub const ORCH_WINDOW: u32 = 4;

pub fn strings(values: &[&str]) -> Vec<String> {
    values.iter().map(|v| (*v).to_string()).collect()
}

/// The orchestrator's whole argv as the daemon launches it (M9.10's
/// `claude_orchestrator_argv_is_exact` and `codex_orchestrator_argv_is_exact` shapes):
/// its `anthrex mcp` server is `exe`, on `socket`.
pub fn orch_argv(runtime: Runtime, exe: &Path, socket: &Path, cwd: &Path) -> Vec<String> {
    orch_argv_with(runtime, exe, socket, cwd, Some("plan the goal"), false)
}

/// [`orch_argv`] with `prompt` as the window's initial prompt; `None` is how the
/// daemon launches an orchestrator since milestone 9.5 decision 38. `codex_hooks`
/// configures Codex's lifecycle hooks, as the daemon always does
/// (`ManagerConfig::from_env`'s `codex_hook_source`).
pub fn orch_argv_with(
    runtime: Runtime,
    exe: &Path,
    socket: &Path,
    cwd: &Path,
    prompt: Option<&str>,
    codex_hooks: bool,
) -> Vec<String> {
    let role = RoleLaunch {
        run_ref: RunRef {
            run_id: RUN_ID.into(),
            task_id: None,
            role: AgentRole::Orchestrator,
            session: 1,
            lane: None,
        },
        mcp: McpTarget {
            role: AgentRole::Orchestrator,
            run_id: RUN_ID.into(),
            task_id: None,
            scout_id: None,
            epic: None,
            chain: None,
            lane: None,
            agent_label: None,
        },
        instructions: "THE CONTRACT".into(),
        effort: Effort::High,
        claude_allowed_tools: strings(ORCHESTRATOR_ALLOWED_TOOLS),
        claude_disallowed_tools: strings(ORCHESTRATOR_DISALLOWED_TOOLS),
        env: Vec::new(),
        remove_env: Vec::new(),
    };
    let model = match runtime {
        Runtime::Codex => "gpt-5.6",
        _ => "opus",
    };
    let spec = WindowSpec {
        name: Some("7a2c/orchestrator".into()),
        runtime,
        cwd: cwd.to_path_buf(),
        worktree_branch: None,
        model: Some(model.into()),
        initial_prompt: prompt.map(str::to_string),
    };
    let ctx = LaunchContext {
        window_id: ORCH_WINDOW,
        name: "7a2c/orchestrator",
        socket_path: socket,
        shell: "/bin/sh",
        exe,
        claude_bin: "/nonexistent/claude",
        codex_bin: "/nonexistent/codex",
        codex_hook_source: codex_hooks.then_some("/<session-flags>/config.toml"),
        codex_bypass_hook_trust: false,
        resume: None,
        caps: &CLI_CAPS,
        role: Some(&role),
    };
    let args = plan(&spec, &ctx).args;
    match runtime {
        Runtime::Claude => assert!(args.contains(&"--mcp-config".to_string()), "{args:?}"),
        _ => assert!(
            args.iter()
                .any(|a| a.starts_with("mcp_servers.anthrex.args="))
        ),
    }
    args
}

/// Stands in for the `anthrex` executable the daemon names: `hook` appends its argv
/// and payload as one line to `$FA_HOOK_LOG`; anything else is the real `anthrex`
/// (`anthrex mcp`).
///
/// One file for every test, never one per test. macOS assesses a newly written
/// executable on its first exec (about 0.5 s) and serialises those assessments, so
/// twelve fresh stand-ins execed at once took up to about 5 s at their first hook:
/// fake-agent's whole `STEP_TIMEOUT` for a hook (M9.0.5's whole-branch review). A
/// shared file is assessed once. It lives in Cargo's per-target scratch directory, is
/// rewritten only when its text changes (so an assessed file is reused across runs),
/// and is replaced by a rename, so a concurrent test binary never runs half a file.
/// Its per-test values come from the environment each spawn sets.
pub fn wrapper() -> &'static Path {
    static WRAPPER: OnceLock<PathBuf> = OnceLock::new();
    WRAPPER.get_or_init(|| {
        let script = "#!/bin/sh\nif [ \"$1\" = hook ]; then\n  { printf '%s\\t' \"$*\"; cat; printf '\\n'; } >> \"$FA_HOOK_LOG\"\n  exit 0\nfi\nexec \"$FA_ANTHREX\" \"$@\"\n";
        let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join("fake-agent-orch");
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("anthrex-wrap");
        if fs::read_to_string(&path).ok().as_deref() != Some(script) {
            let staged = dir.join(format!("anthrex-wrap.{}", std::process::id()));
            fs::write(&staged, script).unwrap();
            fs::set_permissions(&staged, fs::Permissions::from_mode(0o755)).unwrap();
            fs::rename(&staged, &path).unwrap();
        }
        path
    })
}

/// The hook log's events: (the event or notify type, its payload).
pub fn hooks(log: &Path) -> Vec<(String, Value)> {
    let Ok(text) = fs::read_to_string(log) else {
        return Vec::new();
    };
    text.lines()
        .filter_map(|line| {
            let payload: Value = serde_json::from_str(line[line.find('{')?..].trim()).ok()?;
            let event = payload["hook_event_name"]
                .as_str()
                .or(payload["type"].as_str())?
                .to_string();
            Some((event, payload))
        })
        .collect()
}

pub fn wait_for(what: &str, timeout: Duration, mut done: impl FnMut() -> bool) {
    let deadline = Instant::now() + timeout;
    while !done() {
        assert!(Instant::now() < deadline, "{what} within {timeout:?}");
        thread::sleep(Duration::from_millis(10));
    }
}

/// One owned `fake-agent` on a real PTY (its own session, so its own process group),
/// killed on drop only while it has not been reaped.
pub struct Pty {
    child: Box<dyn portable_pty::Child + Send + Sync>,
    pid: libc::pid_t,
    exited: bool,
    writer: Box<dyn Write + Send>,
    screen: Arc<Mutex<Vec<u8>>>,
    _master: Box<dyn MasterPty + Send>,
}

impl Pty {
    pub fn spawn(args: &[String], cwd: &Path, env: &[(&str, &Path)]) -> Self {
        let pair = native_pty_system()
            .openpty(PtySize {
                rows: 40,
                cols: 120,
                pixel_width: 0,
                pixel_height: 0,
            })
            .unwrap();
        let mut command = CommandBuilder::new(fake_agent());
        command.args(args);
        command.cwd(cwd);
        for var in [
            "FAKE_AGENT_SCRIPT",
            "FAKE_AGENT_ARGS_FILE",
            "FAKE_AGENT_STDIN_FILE",
            "FAKE_AGENT_MCP_LOG",
            "FAKE_AGENT_DECIDER_DIR",
            "FAKE_AGENT_BASH_LOG",
        ] {
            command.env_remove(var);
        }
        command.env("GIT_CONFIG_GLOBAL", "/dev/null");
        command.env("GIT_CONFIG_NOSYSTEM", "1");
        command.env("ANTHREX_WINDOW_ID", ORCH_WINDOW.to_string());
        command.env("ANTHREX_SOCKET", cwd.join("never.sock"));
        command.env("ANTHREX_DATA_DIR", cwd.join("never-data"));
        command.env("FA_ANTHREX", anthrex());
        for (key, value) in env {
            command.env(key, value);
        }
        let child = pair.slave.spawn_command(command).unwrap();
        drop(pair.slave);
        let pid = child.process_id().unwrap() as libc::pid_t;
        let screen = Arc::new(Mutex::new(Vec::new()));
        let mut reader = pair.master.try_clone_reader().unwrap();
        let sink = screen.clone();
        thread::spawn(move || {
            let mut buf = [0u8; 4096];
            while let Ok(n) = reader.read(&mut buf) {
                if n == 0 {
                    return;
                }
                sink.lock().unwrap().extend_from_slice(&buf[..n]);
            }
        });
        Self {
            child,
            pid,
            exited: false,
            writer: pair.master.take_writer().unwrap(),
            screen,
            _master: pair.master,
        }
    }

    pub fn write(&mut self, bytes: &[u8]) {
        self.writer.write_all(bytes).unwrap();
        self.writer.flush().unwrap();
    }

    pub fn screen(&self) -> String {
        String::from_utf8_lossy(&self.screen.lock().unwrap()).into_owned()
    }

    /// The exit code, within `timeout`.
    pub fn wait(&mut self, timeout: Duration) -> u32 {
        let deadline = Instant::now() + timeout;
        loop {
            if let Some(status) = self.child.try_wait().unwrap() {
                self.exited = true;
                return status.exit_code();
            }
            assert!(
                Instant::now() < deadline,
                "fake-agent did not exit within {timeout:?}; screen {:?}",
                self.screen()
            );
            thread::sleep(Duration::from_millis(10));
        }
    }
}

impl Drop for Pty {
    fn drop(&mut self) {
        if self.exited {
            return;
        }
        // SAFETY: the negative pid is the process group of our own unreaped child, the
        // leader of the session portable-pty made for it.
        unsafe {
            libc::kill(-self.pid, libc::SIGKILL);
        }
        let _ = self.child.wait();
    }
}

/// A repository with `orchestrator-run-1.jsonl` holding `steps`, and a stub daemon.
pub struct Orch {
    pub dir: tempfile::TempDir,
    pub repo: PathBuf,
    pub exe: PathBuf,
    pub stub: StubDaemon,
}

impl Orch {
    pub fn new(steps: &[Value], replies: &[(bool, &str)]) -> Self {
        let dir = tempdir();
        let repo = repo(dir.path());
        role_script(&repo, "orchestrator-run-1", steps);
        let exe = wrapper().to_path_buf();
        Self {
            stub: StubDaemon::replies(replies),
            dir,
            repo,
            exe,
        }
    }

    pub fn path(&self, name: &str) -> PathBuf {
        self.dir.path().join(name)
    }

    pub fn spawn(&self, runtime: Runtime, env: &[(&str, &Path)]) -> Pty {
        let args = orch_argv(runtime, &self.exe, &self.stub.socket, &self.repo);
        self.spawn_args(&args, env)
    }

    /// [`Orch::spawn`] with `prompt` as the initial prompt (`None`: none after `--`)
    /// and Codex's hooks configured, as the daemon launches it. Only
    /// `pty_first_message.rs` uses it.
    #[allow(dead_code)]
    pub fn spawn_with(&self, runtime: Runtime, env: &[(&str, &Path)], prompt: Option<&str>) -> Pty {
        let args = orch_argv_with(
            runtime,
            &self.exe,
            &self.stub.socket,
            &self.repo,
            prompt,
            true,
        );
        self.spawn_args(&args, env)
    }

    fn spawn_args(&self, args: &[String], env: &[(&str, &Path)]) -> Pty {
        let log = self.path("hooks.log");
        let mut env = env.to_vec();
        env.push(("FA_HOOK_LOG", &log));
        Pty::spawn(args, &self.repo, &env)
    }

    /// Runs the Claude orchestrator to its exit.
    pub fn run(&self, env: &[(&str, &Path)]) -> u32 {
        self.spawn(Runtime::Claude, env).wait(MCP_RUN)
    }
}
