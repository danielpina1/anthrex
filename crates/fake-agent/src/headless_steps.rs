//! The steps that run inside a turn (M8a.20), for `headless::Runner`.

use std::io::Read;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicI32, Ordering};
use std::thread;
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail};
use serde_json::Value;

use super::{Kind, Outcome, POLL, Runner, SH_TIMEOUT};

/// How often a `hang` looks for stdin's EOF.
const HANG_POLL: Duration = Duration::from_millis(100);
use crate::mcp;
use crate::script::Step;

impl Runner {
    pub(super) fn run_step(&mut self, step: Step) -> Result<Outcome> {
        match step {
            Step::Print(text) => self.events.text(&text)?,
            Step::Hook { event, mut payload } => {
                if let Some(command) = self.hooks.hook(&event) {
                    if let Some(object) = payload.as_object_mut() {
                        object
                            .entry("session_id")
                            .or_insert_with(|| self.session.clone().into());
                    }
                    crate::fill_hook_payload(&mut payload, &event)?;
                    crate::run_hook(command, &payload)?;
                }
            }
            Step::Notify(mut payload) => {
                if let Some(command) = self.hooks.notify() {
                    crate::fill_notify_payload(&mut payload)?;
                    crate::run_notify(command, &payload)?;
                }
            }
            // Terminal bytes have no place in a JSON stream.
            Step::Title(_) | Step::Bell => {}
            Step::WaitMs(ms) => {
                if let Some(id) = self.pause(Duration::from_millis(ms)) {
                    return Ok(Outcome::Interrupted(id));
                }
            }
            Step::ReadLine => bail!("read_line is a terminal step; use read_message"),
            Step::GitCommit {
                file,
                content,
                message,
            } => crate::git_commit(&file, &content, &message)?,
            Step::Transcript(entry) => crate::write_transcript(&entry)?,
            Step::McpCall {
                tool,
                args,
                expect_error,
            } => return self.mcp_call(&tool, &args, expect_error),
            Step::Sh(cmd) => match self.shell(&cmd)? {
                Err(id) => return Ok(Outcome::Interrupted(id)),
                Ok((out, code)) => self.events.command_finished(&cmd, &out.combined, code)?,
            },
            Step::Capture { name, sh } => match self.shell(&sh)? {
                Err(id) => return Ok(Outcome::Interrupted(id)),
                Ok((out, code)) => {
                    self.events.command_finished(&sh, &out.combined, code)?;
                    self.vars
                        .captures
                        .insert(name, out.stdout.trim().to_owned());
                    self.script.save_vars(&self.vars)?;
                }
            },
            Step::ApiRetry {
                error,
                delay_ms,
                times,
            } => {
                if self.kind == Kind::Claude {
                    // `delay_ms` apart (M8a.20): the step ends at its last event, so the
                    // silence after it is the script's next step's (M8a.24).
                    for attempt in 1..=times {
                        self.events.api_retry(&error, attempt, delay_ms)?;
                        if attempt == times {
                            break;
                        }
                        if let Some(id) = self.pause(Duration::from_millis(delay_ms)) {
                            return Ok(Outcome::Interrupted(id));
                        }
                    }
                }
            }
            Step::Deny { tool, reason } => self.events.deny(&tool, &reason)?,
            Step::Usage(usage) => self.usage = usage,
            Step::Hang => loop {
                if let Some(id) = self.pause(HANG_POLL) {
                    return Ok(Outcome::Interrupted(id));
                }
                // A Claude process exits at stdin's EOF, as the real CLI does.
                if self.input.as_ref().is_some_and(|input| input.eof) {
                    return Ok(Outcome::Exit(0));
                }
            },
            Step::ReadMessage { .. } | Step::EndTurn | Step::FailTurn(_) | Step::Exit(_) => {
                unreachable!("turn steps are handled by the loop")
            }
        }
        Ok(Outcome::Done)
    }

    fn mcp_call(&mut self, tool: &str, args: &Value, expect_error: bool) -> Result<Outcome> {
        let server = self
            .server
            .clone()
            .context("mcp_call: this session has no anthrex MCP server")?;
        let args = mcp::fill(args, &self.vars.captures);
        self.events.mcp_started(tool, &args)?;
        let reply = mcp::call(&server, tool, &args)?;
        self.events.mcp_finished(tool, &args, &reply)?;
        self.vars.result = reply.text.clone();
        self.script.save_vars(&self.vars)?;
        if reply.ok == expect_error {
            let wanted = if expect_error { "an error" } else { "success" };
            eprintln!(
                "fake-agent: mcp_call {tool} expected {wanted}, got {:?}",
                reply.text
            );
            return Ok(Outcome::Exit(3));
        }
        Ok(Outcome::Done)
    }

    /// `/bin/sh -c cmd` in the cwd with the message and the last result in its
    /// environment, announced as a tool before it runs. `Err` holds an interrupt's
    /// request id; the command's process tree is then killed.
    ///
    /// The command stays in the agent's process group (ruling T20-I1), so the daemon's
    /// group kill reaches it. An interrupt, the timeout, or a signal to the agent alone
    /// kills its tree before the agent goes on or dies.
    fn shell(&mut self, cmd: &str) -> Result<std::result::Result<(ShOutput, i32), String>> {
        self.events.command_started(cmd)?;
        let mut command = Command::new("/bin/sh");
        command
            .args(["-c", cmd])
            .env("FAKE_AGENT_MESSAGE", &self.message)
            .env("FAKE_AGENT_RESULT", &self.vars.result)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        let mut child = command.spawn().context("spawn sh")?;
        let pid = child.id() as libc::pid_t;
        CHILD.store(pid, Ordering::SeqCst);
        let stdout = drain(child.stdout.take().context("sh stdout")?);
        let stderr = drain(child.stderr.take().context("sh stderr")?);
        let deadline = Instant::now() + SH_TIMEOUT;
        let ended = loop {
            if let Some(status) = child.try_wait().context("wait for sh")? {
                break Ok(status);
            }
            if SIGNAL.load(Ordering::SeqCst) != 0 {
                kill_tree(pid);
                let _ = child.wait();
                reraise();
            }
            if let Some(id) = self.pause(POLL) {
                break Err(Ended::Interrupted(id));
            }
            if Instant::now() >= deadline {
                break Err(Ended::TimedOut);
            }
        };
        let status = match ended {
            Ok(status) => status,
            Err(ended) => {
                kill_tree(pid);
                let _ = child.wait();
                CHILD.store(0, Ordering::SeqCst);
                match ended {
                    Ended::Interrupted(id) => return Ok(Err(id)),
                    Ended::TimedOut => bail!("sh timed out after {SH_TIMEOUT:?}: {cmd}"),
                }
            }
        };
        CHILD.store(0, Ordering::SeqCst);
        if SIGNAL.load(Ordering::SeqCst) != 0 {
            reraise();
        }
        let stdout = stdout.join().unwrap_or_default();
        let stderr = stderr.join().unwrap_or_default();
        let combined = format!("{stdout}{stderr}");
        let code = status.code().unwrap_or(-1);
        Ok(Ok((ShOutput { stdout, combined }, code)))
    }
}

enum Ended {
    Interrupted(String),
    TimedOut,
}

/// The running `sh` child's pid, 0 when none.
static CHILD: AtomicI32 = AtomicI32::new(0);
/// A terminating signal that arrived while `CHILD` ran, 0 when none.
static SIGNAL: AtomicI32 = AtomicI32::new(0);

/// SIGINT, SIGTERM and SIGHUP end the agent as before, but only after the running
/// `sh`'s tree is killed. A signal to the agent alone (Codex's interrupt is SIGINT to
/// the leader) would otherwise leave it behind, and a background job in `sh` ignores
/// SIGINT even when the whole group gets it.
pub(super) fn install_signal_handlers() {
    let handler = on_signal as extern "C" fn(libc::c_int) as libc::sighandler_t;
    for signal in [libc::SIGINT, libc::SIGTERM, libc::SIGHUP] {
        // SAFETY: `on_signal` only touches atomics and async-signal-safe calls.
        unsafe {
            libc::signal(signal, handler);
        }
    }
}

extern "C" fn on_signal(signal: libc::c_int) {
    if CHILD.load(Ordering::SeqCst) == 0 {
        // SAFETY: signal and raise are async-signal-safe.
        unsafe {
            libc::signal(signal, libc::SIG_DFL);
            libc::raise(signal);
        }
    } else {
        // The `sh` loop sees it within `POLL`, kills the tree, then re-raises.
        SIGNAL.store(signal, Ordering::SeqCst);
    }
}

/// Dies by the signal that arrived while `sh` ran.
fn reraise() -> ! {
    let signal = SIGNAL.load(Ordering::SeqCst);
    // SAFETY: restoring the default action and raising the signal ends the process.
    unsafe {
        libc::signal(signal, libc::SIG_DFL);
        libc::raise(signal);
    }
    std::process::exit(128 + signal);
}

/// Stops `root`, then kills it and every descendant `ps` lists. The tree shares the
/// agent's process group, so the group cannot be signalled without the agent.
fn kill_tree(root: libc::pid_t) {
    // SAFETY: `root` is our own unreaped child, so its pid cannot have been reused.
    unsafe {
        libc::kill(root, libc::SIGSTOP);
    }
    let mut tree = vec![root];
    if let Ok(output) = Command::new("ps")
        .args(["-A", "-o", "pid=", "-o", "ppid="])
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
    {
        let pairs: Vec<(libc::pid_t, libc::pid_t)> = String::from_utf8_lossy(&output.stdout)
            .lines()
            .filter_map(|line| {
                let mut fields = line.split_whitespace().map(str::parse);
                Some((fields.next()?.ok()?, fields.next()?.ok()?))
            })
            .collect();
        let mut index = 0;
        while index < tree.len() {
            let parent = tree[index];
            let children = pairs.iter().filter(|(_, p)| *p == parent);
            tree.extend(children.map(|(pid, _)| *pid).collect::<Vec<_>>());
            index += 1;
        }
    }
    for pid in tree {
        // SAFETY: SIGKILL to the tree just listed, all descendants of our own child.
        unsafe {
            libc::kill(pid, libc::SIGKILL);
        }
    }
}

struct ShOutput {
    stdout: String,
    combined: String,
}

fn drain(mut pipe: impl Read + Send + 'static) -> thread::JoinHandle<String> {
    thread::spawn(move || {
        let mut bytes = Vec::new();
        let _ = pipe.read_to_end(&mut bytes);
        String::from_utf8_lossy(&bytes).into_owned()
    })
}
