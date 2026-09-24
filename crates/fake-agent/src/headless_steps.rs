//! The steps that run inside a turn (M8a.20), for `headless::Runner`.

use std::io::Read;
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail};
use serde_json::Value;

use super::{Kind, Outcome, POLL, Runner, SH_TIMEOUT};
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
                Ok((out, code)) => self.events.command(&cmd, &out.combined, code)?,
            },
            Step::Capture { name, sh } => match self.shell(&sh)? {
                Err(id) => return Ok(Outcome::Interrupted(id)),
                Ok((out, code)) => {
                    self.events.command(&sh, &out.combined, code)?;
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
                    for attempt in 1..=times {
                        self.events.api_retry(&error, attempt, delay_ms)?;
                        if let Some(id) = self.pause(Duration::from_millis(delay_ms)) {
                            return Ok(Outcome::Interrupted(id));
                        }
                    }
                }
            }
            Step::Deny { tool, reason } => self.events.deny(&tool, &reason)?,
            Step::Usage(usage) => self.usage = usage,
            Step::Hang => loop {
                if let Some(id) = self.pause(Duration::from_secs(3_600)) {
                    return Ok(Outcome::Interrupted(id));
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
    /// environment. `Err` holds an interrupt's request id; the command is then killed.
    fn shell(&mut self, cmd: &str) -> Result<std::result::Result<(ShOutput, i32), String>> {
        let mut command = Command::new("/bin/sh");
        command
            .args(["-c", cmd])
            .env("FAKE_AGENT_MESSAGE", &self.message)
            .env("FAKE_AGENT_RESULT", &self.vars.result)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        crate::isolate_process_group(&mut command);
        let mut child = command.spawn().context("spawn sh")?;
        let group = child.id() as libc::pid_t;
        let stdout = drain(child.stdout.take().context("sh stdout")?);
        let stderr = drain(child.stderr.take().context("sh stderr")?);
        let deadline = Instant::now() + SH_TIMEOUT;
        let status = loop {
            if let Some(status) = child.try_wait().context("wait for sh")? {
                break status;
            }
            if let Some(id) = self.pause(POLL) {
                crate::kill_process_group(group);
                let _ = child.wait();
                return Ok(Err(id));
            }
            if Instant::now() >= deadline {
                crate::kill_process_group(group);
                let _ = child.wait();
                bail!("sh timed out after {SH_TIMEOUT:?}: {cmd}");
            }
        };
        let stdout = stdout.join().unwrap_or_default();
        let stderr = stderr.join().unwrap_or_default();
        let combined = format!("{stdout}{stderr}");
        let code = status.code().unwrap_or(-1);
        Ok(Ok((ShOutput { stdout, combined }, code)))
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
