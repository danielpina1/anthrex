//! The headless modes (M8a.20): Claude's `-p` stream-json and Codex's `exec --json`.
//!
//! A script's steps run inside the current turn. A turn opens when a message arrives
//! (Claude: a stdin line; Codex: the last argument), and ends at `read_message`,
//! `end_turn`, `fail_turn`, an interrupt, or the script's end. A `read_message` at the
//! script position takes the message that opens the next turn, so a Codex session
//! continues its script in the next `exec resume` process, and a Claude session across
//! `--resume`, through the claimed script's `.pos` file. After `end_turn`, a Claude
//! script whose next step is not `read_message` starts an unprompted turn at once, as
//! M8a.1 recorded Claude Code doing.

use std::io::{self, Read, Write};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{Receiver, RecvTimeoutError};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result};
use serde_json::Value;

use crate::mcp::Reply;
use crate::roles::{self, Script, Vars};
use crate::runtime::{self, McpServer, Runtime};
use crate::script::{Step, Usage};
use crate::stream_claude::{self, Claude, Line};
use crate::stream_codex::Codex;

/// A bound on one `sh` or `capture` command: the spec's `RUN_WAIT` (300 s), the
/// longest a scripted deadline loop may run in M8a.24, plus slack.
const SH_TIMEOUT: Duration = Duration::from_secs(330);
const POLL: Duration = Duration::from_millis(10);

/// What each runtime writes for the events a script produces.
pub trait Events {
    fn turn_started(&mut self) -> Result<()>;
    fn text(&mut self, text: &str) -> Result<()>;
    fn command_started(&mut self, command: &str) -> Result<()>;
    fn command_finished(&mut self, command: &str, output: &str, code: i32) -> Result<()>;
    fn mcp_started(&mut self, tool: &str, args: &Value) -> Result<()>;
    fn mcp_finished(&mut self, tool: &str, args: &Value, reply: &Reply) -> Result<()>;
    fn deny(&mut self, tool: &str, reason: &str) -> Result<()>;
    fn api_retry(&mut self, error: &str, attempt: u32, delay_ms: u64) -> Result<()>;
    fn turn_completed(&mut self, usage: Usage) -> Result<()>;
    fn turn_failed(&mut self, error: &str, usage: Usage) -> Result<()>;
    fn interrupted(&mut self, request_id: &str, usage: Usage) -> Result<()>;
    fn control_response(&mut self, request_id: &str) -> Result<()>;
}

/// One JSON line per event on stdout, flushed at once.
#[derive(Default)]
pub struct Out;

impl Out {
    pub fn write(&mut self, value: &Value) -> Result<()> {
        let mut stdout = io::stdout().lock();
        let mut line = serde_json::to_vec(value).context("encode event")?;
        line.push(b'\n');
        stdout.write_all(&line).context("write event")?;
        stdout.flush().context("flush event")
    }
}

/// A UUID-shaped id unique to this process and call.
pub fn fresh_id() -> String {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    format!(
        "{:08x}-{:04x}-4{:03x}-8{:03x}-{:012x}",
        std::process::id(),
        (nanos >> 32) as u16,
        (nanos >> 20) as u16 & 0xfff,
        n as u16 & 0xfff,
        nanos as u64 & 0xffff_ffff_ffff,
    )
}

/// The current time as `YYYY-MM-DDTHH:MM:SS.mmmZ`.
pub fn timestamp() -> String {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    let (days, secs) = (now.as_secs() / 86_400, now.as_secs() % 86_400);
    // Howard Hinnant's days-to-civil.
    let z = days as i64 + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}.{:03}Z",
        secs / 3_600,
        secs / 60 % 60,
        secs % 60,
        now.subsec_millis()
    )
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Claude,
    Codex,
}

/// A headless argv: Claude's has `-p` and `--input-format stream-json`; Codex's starts
/// with `exec`, and `exec resume <id>` resumes.
#[derive(Debug, Clone, PartialEq)]
pub struct Invocation {
    pub kind: Kind,
    pub resume: Option<String>,
    session_id: Option<String>,
    model: Option<String>,
    permission_mode: Option<String>,
}

fn value_of(args: &[String], flag: &str) -> Option<String> {
    let index = args.iter().position(|arg| arg == flag)?;
    args.get(index + 1).cloned()
}

pub fn detect(args: &[String]) -> Option<Invocation> {
    if args.first().map(String::as_str) == Some("exec") {
        let resume = (args.get(1).map(String::as_str) == Some("resume"))
            .then(|| args.get(2).cloned())
            .flatten();
        return Some(Invocation {
            kind: Kind::Codex,
            resume,
            session_id: None,
            model: None,
            permission_mode: None,
        });
    }
    let stream_input = args
        .windows(2)
        .any(|pair| pair[0] == "--input-format" && pair[1] == "stream-json");
    (args.iter().any(|arg| arg == "-p") && stream_input).then(|| Invocation {
        kind: Kind::Claude,
        resume: value_of(args, "--resume"),
        session_id: value_of(args, "--session-id"),
        model: value_of(args, "--model"),
        permission_mode: value_of(args, "--permission-mode"),
    })
}

pub fn run(args: &[String], invocation: Invocation) -> Result<i32> {
    steps::install_signal_handlers();
    let hooks = runtime::discover(args)?;
    let server = runtime::mcp_server(args)?;
    let flag = |name| server.as_ref().and_then(|s| s.flag(name)).map(String::from);
    let (role, task) = (flag("--role"), flag("--task"));
    let session = invocation
        .resume
        .clone()
        .or(invocation.session_id.clone())
        .unwrap_or_else(fresh_id);
    let script = roles::find(
        role.as_deref(),
        task.as_deref(),
        &session,
        invocation.resume.is_some(),
    )?;
    roles::record_args(&script.name, args)?;
    let steps = match &script.path {
        None => Vec::new(),
        Some(path) => {
            let file = std::fs::File::open(path)
                .with_context(|| format!("open script {}", path.display()))?;
            match crate::script::parse_script(io::BufReader::new(file)) {
                Ok(steps) => steps,
                Err(error) => {
                    eprintln!("fake-agent: {error:#}");
                    return Ok(2);
                }
            }
        }
    };

    let (events, input, first): (Box<dyn Events>, _, _) = match invocation.kind {
        Kind::Claude => {
            let claude = Claude::new(
                &session,
                invocation.model.as_deref(),
                invocation.permission_mode.as_deref(),
                server.is_some(),
            );
            let input = Input::new(stream_claude::read_stdin(script.name.clone()));
            (Box::new(claude), Some(input), None)
        }
        Kind::Codex => {
            // Carry T17-C1: the real `codex exec` reads piped stdin to EOF first.
            let mut stdin = String::new();
            io::stdin()
                .read_to_string(&mut stdin)
                .context("read stdin to EOF")?;
            for line in stdin.lines() {
                roles::record_stdin(&script.name, line)?;
            }
            let mut codex = Codex::new(&session);
            codex.thread_started()?;
            (Box::new(codex), None, args.last().cloned())
        }
    };
    let mut runner = Runner {
        kind: invocation.kind,
        pos: script.pos(),
        vars: script.vars(),
        steps,
        script,
        message: String::new(),
        usage: Usage::default(),
        turn_open: false,
        events,
        input,
        hooks,
        server,
        session,
    };
    runner.run(first)
}

/// Claude's stdin: accepted lines, with messages that arrive during a turn queued.
struct Input {
    lines: Receiver<Line>,
    queued: std::collections::VecDeque<String>,
    eof: bool,
}

enum Got {
    Message(String),
    Interrupt(String),
    Eof,
    Timeout,
}

impl Input {
    fn new(lines: Receiver<Line>) -> Self {
        Self {
            lines,
            queued: Default::default(),
            eof: false,
        }
    }

    fn next(&mut self, timeout: Option<Duration>) -> Got {
        if let Some(message) = self.queued.pop_front() {
            return Got::Message(message);
        }
        if self.eof {
            return Got::Eof;
        }
        let line = match timeout {
            Some(timeout) => self.lines.recv_timeout(timeout),
            None => self
                .lines
                .recv()
                .map_err(|_| RecvTimeoutError::Disconnected),
        };
        match line {
            Ok(Line::Message(text)) => Got::Message(text),
            Ok(Line::Interrupt(id)) => Got::Interrupt(id),
            Err(RecvTimeoutError::Timeout) => Got::Timeout,
            Err(RecvTimeoutError::Disconnected) => {
                self.eof = true;
                Got::Eof
            }
        }
    }
}

/// How a step ended.
enum Outcome {
    Done,
    Interrupted(String),
    Exit(i32),
}

struct Runner {
    kind: Kind,
    steps: Vec<Step>,
    pos: usize,
    script: Script,
    vars: Vars,
    message: String,
    usage: Usage,
    turn_open: bool,
    events: Box<dyn Events>,
    input: Option<Input>,
    hooks: Runtime,
    server: Option<McpServer>,
    session: String,
}

impl Runner {
    fn run(&mut self, first: Option<String>) -> Result<i32> {
        let first = match first {
            Some(message) => message,
            None => match self.wait_message(None)? {
                Some(message) => message,
                None => return Ok(0),
            },
        };
        if let Some(code) = self.take_message(first)? {
            return Ok(code);
        }
        loop {
            let Some(step) = self.steps.get(self.pos).cloned() else {
                return self.idle();
            };
            let index = self.pos;
            match step {
                Step::ReadMessage { timeout_ms, .. } => {
                    self.complete_turn()?;
                    self.script.save_pos(index)?;
                    if self.kind == Kind::Codex {
                        return Ok(0);
                    }
                    let timeout = timeout_ms.map(Duration::from_millis);
                    match self.wait_message(timeout)? {
                        Some(message) => {
                            if let Some(code) = self.take_message(message)? {
                                return Ok(code);
                            }
                        }
                        None if self.input.as_ref().is_some_and(|i| i.eof) => return Ok(0),
                        None => {
                            eprintln!("fake-agent: read_message timed out");
                            return Ok(4);
                        }
                    }
                }
                Step::EndTurn => {
                    self.advance()?;
                    self.complete_turn()?;
                    if self.kind == Kind::Codex {
                        return Ok(0);
                    }
                }
                Step::FailTurn(error) => {
                    self.advance()?;
                    self.open_turn()?;
                    let usage = self.take_usage();
                    self.events.turn_failed(&error, usage)?;
                    self.turn_open = false;
                    if self.kind == Kind::Codex {
                        return Ok(1);
                    }
                    if let Some(code) = self.after_closed_turn()? {
                        return Ok(code);
                    }
                }
                Step::Exit(code) => {
                    self.advance()?;
                    return Ok(code);
                }
                step => {
                    self.advance()?;
                    self.open_turn()?;
                    match self.run_step(step)? {
                        Outcome::Done => {}
                        Outcome::Exit(code) => return Ok(code),
                        Outcome::Interrupted(id) => {
                            let usage = self.take_usage();
                            self.events.interrupted(&id, usage)?;
                            self.turn_open = false;
                            if let Some(code) = self.after_closed_turn()? {
                                return Ok(code);
                            }
                        }
                    }
                }
            }
        }
    }

    fn advance(&mut self) -> Result<()> {
        self.pos += 1;
        self.script.save_pos(self.pos)
    }

    fn take_usage(&mut self) -> Usage {
        std::mem::take(&mut self.usage)
    }

    fn open_turn(&mut self) -> Result<()> {
        if !self.turn_open {
            self.turn_open = true;
            self.events.turn_started()?;
        }
        Ok(())
    }

    fn complete_turn(&mut self) -> Result<()> {
        if self.turn_open {
            self.turn_open = false;
            let usage = self.take_usage();
            self.events.turn_completed(usage)?;
        }
        Ok(())
    }

    /// A message opens a turn; a `read_message` at the position takes it.
    fn take_message(&mut self, message: String) -> Result<Option<i32>> {
        self.message = message;
        self.open_turn()?;
        if let Some(Step::ReadMessage { expect, .. }) = self.steps.get(self.pos) {
            if let Some(expect) = expect
                && !self.message.contains(expect.as_str())
            {
                eprintln!(
                    "fake-agent: expected a message containing {expect:?}, got {:?}",
                    self.message
                );
                return Ok(Some(3));
            }
            self.advance()?;
        }
        Ok(None)
    }

    /// After a failed or interrupted Claude turn, the next turn waits for a message.
    fn after_closed_turn(&mut self) -> Result<Option<i32>> {
        let next_reads = matches!(self.steps.get(self.pos), Some(Step::ReadMessage { .. }));
        if self.pos >= self.steps.len() || next_reads {
            return Ok(None);
        }
        match self.wait_message(None)? {
            Some(message) => self.take_message(message),
            None => Ok(Some(0)),
        }
    }

    /// The script has ended: Codex exits; Claude answers every message with an empty
    /// turn until stdin's EOF.
    fn idle(&mut self) -> Result<i32> {
        self.complete_turn()?;
        if self.kind == Kind::Codex {
            return Ok(0);
        }
        while let Some(message) = self.wait_message(None)? {
            self.message = message;
            self.open_turn()?;
            self.complete_turn()?;
        }
        Ok(0)
    }

    /// The next message; an interrupt between turns is answered and ignored. `None`
    /// at EOF or when `timeout` passes.
    fn wait_message(&mut self, timeout: Option<Duration>) -> Result<Option<String>> {
        let deadline = timeout.map(|t| Instant::now() + t);
        loop {
            let left = deadline.map(|d| d.saturating_duration_since(Instant::now()));
            let input = self.input.as_mut().context("Codex reads no messages")?;
            match input.next(left) {
                Got::Message(message) => return Ok(Some(message)),
                Got::Interrupt(id) => self.events.control_response(&id)?,
                Got::Eof | Got::Timeout => return Ok(None),
            }
        }
    }

    /// Sleeps for `duration`, queueing messages; returns an interrupt's request id.
    fn pause(&mut self, duration: Duration) -> Option<String> {
        let deadline = Instant::now() + duration;
        let Some(input) = self.input.as_mut() else {
            thread::sleep(duration);
            return None;
        };
        loop {
            let left = deadline.saturating_duration_since(Instant::now());
            if left.is_zero() {
                return None;
            }
            if input.eof {
                thread::sleep(left);
                return None;
            }
            match input.lines.recv_timeout(left) {
                Ok(Line::Message(text)) => input.queued.push_back(text),
                Ok(Line::Interrupt(id)) => return Some(id),
                Err(RecvTimeoutError::Timeout) => return None,
                Err(RecvTimeoutError::Disconnected) => input.eof = true,
            }
        }
    }
}

#[path = "headless_steps.rs"]
mod steps;

#[cfg(test)]
mod tests {
    use super::{Kind, detect, timestamp};

    fn strings(values: &[&str]) -> Vec<String> {
        values.iter().map(|v| (*v).to_owned()).collect()
    }

    #[test]
    fn detects_each_headless_argv_and_nothing_else() {
        let claude = detect(&strings(&[
            "-p",
            "--input-format",
            "stream-json",
            "--resume",
            "s1",
            "--model",
            "m",
        ]))
        .unwrap();
        assert_eq!(
            (claude.kind, claude.resume.as_deref()),
            (Kind::Claude, Some("s1"))
        );
        let codex = detect(&strings(&["exec", "resume", "th-1", "--json", "--", "hi"])).unwrap();
        assert_eq!(
            (codex.kind, codex.resume.as_deref()),
            (Kind::Codex, Some("th-1"))
        );
        let first = detect(&strings(&["exec", "--json", "--", "hi"])).unwrap();
        assert_eq!((first.kind, first.resume), (Kind::Codex, None));

        for terminal in [
            strings(&["--name", "x", "--settings", "{}"]),
            strings(&["-p", "hello"]),
            strings(&["-C", "/tmp", "exec"]),
        ] {
            assert_eq!(detect(&terminal), None, "{terminal:?}");
        }
    }

    /// Ruling T20-m6: an `sh` deadline loop in M8a.24 may run for one `RUN_WAIT`.
    #[test]
    fn sh_timeout_outlasts_the_specs_run_wait() {
        assert!(super::SH_TIMEOUT > std::time::Duration::from_secs(300));
    }

    #[test]
    fn timestamp_is_iso_8601_utc() {
        let stamp = timestamp();
        assert_eq!(stamp.len(), 24, "{stamp}");
        assert!(stamp.starts_with("20") && stamp.ends_with('Z'), "{stamp}");
        assert_eq!(&stamp[10..11], "T");
    }
}
