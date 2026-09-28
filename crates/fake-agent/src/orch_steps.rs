//! Milestone 9 task M9.12: a run's orchestrator in a PTY, and the steps that script it.
//!
//! In PTY mode (no `-p`, no `exec`), an argv that carries the anthrex MCP server (Claude's
//! `--mcp-config`, Codex's `-c mcp_servers.anthrex.*`) is a run role's: its script is
//! claimed by [`roles::key`] as a headless session's is (`orchestrator-run-<n>.jsonl`,
//! else M3's `FAKE_AGENT_SCRIPT`), `mcp_call` works as there, and `read_message` ends the
//! turn with the `Stop` hook (Codex: its `notify`), then reads a bracketed paste or
//! typed bytes up to `\r` from the terminal, as a real TUI does. M3's steps run as in
//! M3. `mcp_until`, `capture_json`, `expect` and `expect_error_contains` run here in
//! both modes, through [`Host`].

use std::fs::File;
use std::io::{self, BufRead, BufReader, Stdin};
use std::os::fd::AsRawFd;
use std::thread;
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::mcp::{self, Reply};
use crate::roles::{self, Script, Vars};
use crate::runtime::{self, McpServer, Runtime};
use crate::script::Step;

/// How long `mcp_until` waits between two calls.
const UNTIL_POLL: Duration = Duration::from_millis(200);

const PASTE_START: &[u8] = b"\x1b[200~";
const PASTE_END: &[u8] = b"\x1b[201~";

/// `{pointer, equals}`: the JSON pointer into `FAKE_AGENT_RESULT` and the value it must
/// hold.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Match {
    pub pointer: String,
    pub equals: Value,
}

impl Match {
    /// Whether `result`, parsed as JSON, holds `equals` at `pointer`.
    fn holds(&self, result: &str) -> bool {
        serde_json::from_str::<Value>(result)
            .ok()
            .and_then(|value| value.pointer(&self.pointer).cloned())
            .is_some_and(|value| value == self.equals)
    }
}

/// `mcp_until {tool, args, until, timeout_ms}`.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Until {
    pub tool: String,
    pub args: Value,
    pub until: Match,
    pub timeout_ms: u64,
}

/// `capture_json {name, pointer}`.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CaptureJson {
    pub name: String,
    pub pointer: String,
}

/// `expect_error_contains {text}`.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Text {
    pub text: String,
}

/// What a session gives the steps: its MCP calls, its waits and its variables.
pub trait Host {
    /// One `mcp_call` of `tool` with `args` (captures not yet filled): logged to
    /// `FAKE_AGENT_MCP_LOG`, and its text kept as `FAKE_AGENT_RESULT`.
    fn call(&mut self, tool: &str, args: &Value) -> Result<Reply>;
    /// Waits `duration`; an interrupt's request id ends the wait.
    fn wait(&mut self, duration: Duration) -> Option<String>;
    fn vars(&mut self) -> &mut Vars;
    fn save_vars(&mut self) -> Result<()>;
}

/// How one of this module's steps ended.
pub enum Flow {
    Next,
    Exit(i32),
    Interrupted(String),
}

/// Whether `step` is one of this module's.
pub fn owns(step: &Step) -> bool {
    matches!(
        step,
        Step::McpUntil(_)
            | Step::CaptureJson { .. }
            | Step::Expect(_)
            | Step::ExpectErrorContains(_)
    )
}

fn fail(text: String) -> Result<Flow> {
    eprintln!("fake-agent: {text}");
    Ok(Flow::Exit(3))
}

/// Runs a step [`owns`] accepts. Exit 3 for a result that differs from what the step
/// expects (an error reply to `mcp_until` included, as for `mcp_call`); exit 4 when
/// `mcp_until` times out.
pub fn run(host: &mut impl Host, step: &Step) -> Result<Flow> {
    match step {
        Step::McpUntil(until) => {
            let deadline = Instant::now() + Duration::from_millis(until.timeout_ms);
            loop {
                let reply = host.call(&until.tool, &until.args)?;
                if !reply.ok {
                    return fail(format!(
                        "mcp_until {} got an error: {:?}",
                        until.tool, reply.text
                    ));
                }
                if until.until.holds(&reply.text) {
                    return Ok(Flow::Next);
                }
                let left = deadline.saturating_duration_since(Instant::now());
                if left.is_zero() {
                    eprintln!(
                        "fake-agent: mcp_until {} timed out; last result {:?}",
                        until.tool, reply.text
                    );
                    return Ok(Flow::Exit(4));
                }
                if let Some(id) = host.wait(UNTIL_POLL.min(left)) {
                    return Ok(Flow::Interrupted(id));
                }
            }
        }
        Step::CaptureJson { name, pointer } => {
            let result = &host.vars().result;
            let value = serde_json::from_str::<Value>(result)
                .ok()
                .and_then(|value| value.pointer(pointer).cloned());
            let Some(value) = value else {
                return fail(format!("capture_json: nothing at {pointer} in {result:?}"));
            };
            let text = match value {
                Value::String(text) => text,
                other => other.to_string(),
            };
            host.vars().captures.insert(name.clone(), text);
            host.save_vars()?;
            Ok(Flow::Next)
        }
        Step::Expect(expected) => {
            let result = &host.vars().result;
            if expected.holds(result) {
                return Ok(Flow::Next);
            }
            let (pointer, equals) = (&expected.pointer, &expected.equals);
            fail(format!("expected {equals} at {pointer}, got {result:?}"))
        }
        Step::ExpectErrorContains(text) => {
            let result = &host.vars().result;
            if result.contains(text.as_str()) {
                return Ok(Flow::Next);
            }
            fail(format!(
                "expected an error containing {text:?}, got {result:?}"
            ))
        }
        other => anyhow::bail!("{other:?} is not an M9.12 step"),
    }
}

/// PTY mode with an MCP server: `None` for any other argv (M3's terminal mode).
pub fn pty(args: &[String]) -> Result<Option<i32>> {
    let Some(server) = runtime::mcp_server(args)? else {
        return Ok(None);
    };
    let hooks = runtime::discover(args)?;
    let (role, key) = roles::key(Some(&server));
    let resume = resumed(args);
    let session = resume.clone().unwrap_or_else(crate::default_session_id);
    let script = roles::find(role.as_deref(), key.as_deref(), &session, resume.is_some())?;
    roles::record_args(&script.name, args)?;
    let steps = match &script.path {
        None => {
            println!("fake-agent: no script (FAKE_AGENT_SCRIPT is unset)");
            Vec::new()
        }
        Some(path) => match File::open(path) {
            Err(error) => {
                println!("fake-agent: no script ({}: {error})", path.display());
                Vec::new()
            }
            Ok(file) => match crate::script::parse_script(BufReader::new(file)) {
                Ok(steps) => steps,
                Err(error) => {
                    eprintln!("fake-agent: {error:#}");
                    return Ok(Some(2));
                }
            },
        },
    };
    raw_terminal();
    let mut pty = Pty {
        pos: script.pos(),
        vars: script.vars(),
        steps,
        script,
        server,
        hooks,
        session,
        message: prompt(args),
        input: BufReader::new(io::stdin()),
    };
    pty.run().map(Some)
}

/// The session a relaunch resumes: Claude's `--resume <id>`, Codex's `resume <id>`.
fn resumed(args: &[String]) -> Option<String> {
    let config = args.split(|arg| arg == "--").next().unwrap_or_default();
    config
        .windows(2)
        .find(|pair| pair[0] == "--resume" || pair[0] == "resume")
        .map(|pair| pair[1].clone())
}

/// The initial prompt, the argument after `--`.
fn prompt(args: &[String]) -> String {
    match args.iter().position(|arg| arg == "--") {
        Some(index) => args.get(index + 1).cloned().unwrap_or_default(),
        None => String::new(),
    }
}

/// A TUI's terminal: no line editing, no echo, and `\r` as typed. Signals stay on, so a
/// `^C` still ends the agent as in M3.
fn raw_terminal() {
    let fd = io::stdin().as_raw_fd();
    // SAFETY: `termios` is plain data; both calls only read or write it for `fd`.
    unsafe {
        if libc::isatty(fd) != 1 {
            return;
        }
        let mut termios: libc::termios = std::mem::zeroed();
        if libc::tcgetattr(fd, &mut termios) != 0 {
            return;
        }
        termios.c_lflag &= !(libc::ICANON | libc::ECHO);
        termios.c_iflag &= !libc::ICRNL;
        termios.c_cc[libc::VMIN] = 1;
        termios.c_cc[libc::VTIME] = 0;
        libc::tcsetattr(fd, libc::TCSANOW, &termios);
    }
}

enum Read {
    Message(String),
    Eof,
    Timeout,
}

struct Pty {
    steps: Vec<Step>,
    pos: usize,
    script: Script,
    vars: Vars,
    server: McpServer,
    hooks: Runtime,
    session: String,
    /// The last message read (`FAKE_AGENT_MESSAGE`), at first the argv's prompt.
    message: String,
    input: BufReader<Stdin>,
}

impl Pty {
    fn run(&mut self) -> Result<i32> {
        let mut output = io::stdout();
        while let Some(step) = self.steps.get(self.pos).cloned() {
            if let Step::ReadMessage { timeout_ms, expect } = step {
                // The position stays at the read until a message is taken, so a resumed
                // session reads again.
                self.script.save_pos(self.pos)?;
                match self.read_message(timeout_ms)? {
                    Read::Message(text) => {
                        if let Some(expect) = expect
                            && !text.contains(expect.as_str())
                        {
                            eprintln!(
                                "fake-agent: expected a message containing {expect:?}, got {text:?}"
                            );
                            return Ok(3);
                        }
                        self.message = text;
                    }
                    Read::Eof => return Ok(0),
                    Read::Timeout => {
                        eprintln!("fake-agent: read_message timed out");
                        return Ok(4);
                    }
                }
                self.advance()?;
                continue;
            }
            self.advance()?;
            match step {
                Step::McpCall {
                    tool,
                    args,
                    expect_error,
                } => {
                    let reply = self.call(&tool, &args)?;
                    if reply.ok == expect_error {
                        let wanted = if expect_error { "an error" } else { "success" };
                        eprintln!(
                            "fake-agent: mcp_call {tool} expected {wanted}, got {:?}",
                            reply.text
                        );
                        return Ok(3);
                    }
                }
                step if owns(&step) => match run(self, &step)? {
                    Flow::Next | Flow::Interrupted(_) => {}
                    Flow::Exit(code) => return Ok(code),
                },
                step => {
                    if let Some(code) =
                        crate::run_step(step, &self.hooks, &mut self.input, &mut output)?
                    {
                        return Ok(code);
                    }
                }
            }
        }
        io::copy(&mut self.input, &mut io::sink()).context("read stdin to EOF")?;
        Ok(0)
    }

    fn advance(&mut self) -> Result<()> {
        self.pos += 1;
        self.script.save_pos(self.pos)
    }

    /// The turn ends (`Stop`, or Codex's `notify`); then a bracketed paste or typed
    /// bytes up to `\r` (or `\n`) are the next message, recorded to
    /// `FAKE_AGENT_STDIN_FILE` with the time, and announced with `UserPromptSubmit`.
    fn read_message(&mut self, timeout_ms: Option<u64>) -> Result<Read> {
        self.turn_ended()?;
        let deadline = timeout_ms.map(|ms| Instant::now() + Duration::from_millis(ms));
        let mut raw = Vec::new();
        let mut in_paste = false;
        loop {
            if self.input.buffer().is_empty() && !readable(deadline) {
                return Ok(Read::Timeout);
            }
            let byte = match self.input.fill_buf() {
                Ok([]) => return Ok(Read::Eof),
                Ok(bytes) => bytes[0],
                Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
                // A PTY whose other side closed reads EIO on Linux.
                Err(_) => return Ok(Read::Eof),
            };
            self.input.consume(1);
            raw.push(byte);
            if raw.ends_with(PASTE_START) {
                in_paste = true;
            } else if raw.ends_with(PASTE_END) {
                in_paste = false;
            } else if !in_paste && (byte == b'\r' || byte == b'\n') {
                break;
            }
        }
        let text = String::from_utf8_lossy(&unframe(&raw[..raw.len() - 1])).into_owned();
        let line = json!({"at": crate::headless::timestamp(),
            "raw": String::from_utf8_lossy(&raw), "text": text});
        roles::record_stdin(&self.script.name, &line.to_string())?;
        if let Some(command) = self.hooks.hook("UserPromptSubmit") {
            let mut payload = json!({"prompt": text, "session_id": self.session});
            crate::fill_hook_payload(&mut payload, "UserPromptSubmit")?;
            crate::run_hook(command, &payload)?;
        }
        Ok(Read::Message(text))
    }

    fn turn_ended(&mut self) -> Result<()> {
        if let Some(command) = self.hooks.hook("Stop") {
            let mut payload = json!({"session_id": self.session});
            crate::fill_hook_payload(&mut payload, "Stop")?;
            crate::run_hook(command, &payload)
        } else if let Some(command) = self.hooks.notify() {
            let mut payload = json!({"thread-id": self.session,
                "last-assistant-message": ""});
            crate::fill_notify_payload(&mut payload)?;
            crate::run_notify(command, &payload)
        } else {
            Ok(())
        }
    }
}

impl Host for Pty {
    fn call(&mut self, tool: &str, args: &Value) -> Result<Reply> {
        let args = mcp::fill(args, &self.vars.captures);
        let reply = mcp::call(&self.server, tool, &args)?;
        mcp::log(&self.script.name, tool, &args, &reply)?;
        self.vars.result = reply.text.clone();
        self.script.save_vars(&self.vars)?;
        Ok(reply)
    }

    fn wait(&mut self, duration: Duration) -> Option<String> {
        thread::sleep(duration);
        None
    }

    fn vars(&mut self) -> &mut Vars {
        &mut self.vars
    }

    fn save_vars(&mut self) -> Result<()> {
        self.script.save_vars(&self.vars)
    }
}

/// `raw` without its paste brackets.
fn unframe(raw: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(raw.len());
    let mut rest = raw;
    while !rest.is_empty() {
        if let Some(after) = rest
            .strip_prefix(PASTE_START)
            .or(rest.strip_prefix(PASTE_END))
        {
            rest = after;
        } else {
            out.push(rest[0]);
            rest = &rest[1..];
        }
    }
    out
}

/// Whether stdin has input (or its end) before `deadline`; `None` waits for ever.
fn readable(deadline: Option<Instant>) -> bool {
    loop {
        let timeout = match deadline {
            None => -1,
            Some(deadline) => {
                let left = deadline.saturating_duration_since(Instant::now());
                i32::try_from(left.as_millis()).unwrap_or(i32::MAX)
            }
        };
        let mut fd = libc::pollfd {
            fd: io::stdin().as_raw_fd(),
            events: libc::POLLIN,
            revents: 0,
        };
        // SAFETY: one valid `pollfd` for the duration of the call.
        let n = unsafe { libc::poll(&mut fd, 1, timeout) };
        if n > 0 {
            return true;
        }
        if n == 0 {
            return false;
        }
        if io::Error::last_os_error().kind() != io::ErrorKind::Interrupted {
            // Let the read report what is wrong.
            return true;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{Match, Until, prompt, resumed, unframe};
    use crate::script::{Step, parse_script};
    use serde_json::json;
    use std::io::Cursor;

    fn strings(values: &[&str]) -> Vec<String> {
        values.iter().map(|v| (*v).to_owned()).collect()
    }

    #[test]
    fn parses_the_m9_steps_and_rejects_unknown_keys() {
        let input = concat!(
            "{\"mcp_until\":{\"tool\":\"run_status\",\"args\":{\"wait_secs\":50},",
            "\"until\":{\"pointer\":\"/gate/state\",\"equals\":\"approved\"},\"timeout_ms\":9}}\n",
            "{\"capture_json\":{\"name\":\"sid\",\"pointer\":\"/scout_id\"}}\n",
            "{\"expect\":{\"pointer\":\"/n\",\"equals\":3}}\n",
            "{\"expect_error_contains\":{\"text\":\"closed\"}}\n",
        );
        let steps = parse_script(Cursor::new(input)).unwrap();
        let until = Match {
            pointer: "/gate/state".into(),
            equals: json!("approved"),
        };
        assert_eq!(
            steps,
            vec![
                Step::McpUntil(Until {
                    tool: "run_status".into(),
                    args: json!({"wait_secs": 50}),
                    until,
                    timeout_ms: 9,
                }),
                Step::CaptureJson {
                    name: "sid".into(),
                    pointer: "/scout_id".into(),
                },
                Step::Expect(Match {
                    pointer: "/n".into(),
                    equals: json!(3),
                }),
                Step::ExpectErrorContains("closed".into()),
            ]
        );
        for line in [
            "{\"expect\":{\"pointer\":\"/n\",\"equals\":3,\"x\":1}}\n",
            "{\"capture_json\":{\"name\":\"a\"}}\n",
            "{\"mcp_until\":{\"tool\":\"t\",\"args\":{},\"until\":{\"pointer\":\"/a\"},\"timeout_ms\":1}}\n",
        ] {
            assert!(parse_script(Cursor::new(line)).is_err(), "{line}");
        }
    }

    #[test]
    fn a_match_needs_json_and_the_exact_value() {
        let m = Match {
            pointer: "/a/b".into(),
            equals: json!(1),
        };
        assert!(m.holds(r#"{"a":{"b":1}}"#));
        assert!(!m.holds(r#"{"a":{"b":"1"}}"#));
        assert!(!m.holds(r#"{"a":{}}"#));
        assert!(!m.holds("not json"));
    }

    #[test]
    fn unframe_drops_only_the_paste_brackets() {
        let raw = b"x \x1b[200~a\nb\x1b[201~ y\x1b[A";
        assert_eq!(unframe(raw), b"x a\nb y\x1b[A");
    }

    #[test]
    fn resume_and_prompt_come_from_either_runtimes_argv() {
        let claude = strings(&["--name", "n", "--model", "opus", "--resume", "s-1"]);
        assert_eq!(resumed(&claude).as_deref(), Some("s-1"));
        assert_eq!(prompt(&claude), "");
        let codex = strings(&["-C", "/r", "-m", "gpt", "resume", "thr-1"]);
        assert_eq!(resumed(&codex).as_deref(), Some("thr-1"));
        let fresh = strings(&["--model", "opus", "--", "resume thr-1"]);
        assert_eq!(
            (resumed(&fresh), prompt(&fresh)),
            (None, "resume thr-1".into())
        );
    }
}
