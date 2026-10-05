//! Milestone 9 task M9.12's scripted steps that run in both modes, through [`Host`]:
//! `mcp_until`, `capture_json`, `expect` and `expect_error_contains` (moved out of
//! `orch_steps.rs` unchanged by the final fix wave, W3).

use std::time::{Duration, Instant};

use anyhow::Result;
use serde::Deserialize;
use serde_json::Value;

use crate::mcp::Reply;
use crate::roles::Vars;
use crate::script::Step;

/// How long `mcp_until` waits between two calls.
const UNTIL_POLL: Duration = Duration::from_millis(200);

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
    pub(super) fn holds(&self, result: &str) -> bool {
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
    crate::diag::say!("fake-agent: {text}");
    Ok(Flow::Exit(3))
}

/// Runs a step [`owns`] accepts. Exit 3 for a result that differs from what the step
/// expects (an error reply to `mcp_until` included, as for `mcp_call`); exit 4 when
/// `mcp_until` times out. No `mcp_until` call starts once its deadline has passed, so
/// the step ends at most one call's time after it.
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
                if !left.is_zero()
                    && let Some(id) = host.wait(UNTIL_POLL.min(left))
                {
                    return Ok(Flow::Interrupted(id));
                }
                if Instant::now() >= deadline {
                    let (tool, text) = (&until.tool, &reply.text);
                    crate::diag::say!(
                        "fake-agent: mcp_until {tool} timed out; last result {text:?}"
                    );
                    return Ok(Flow::Exit(4));
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
            let vars = host.vars();
            let result = &vars.result;
            if vars.last_error && result.contains(text.as_str()) {
                return Ok(Flow::Next);
            }
            let got = if vars.last_error { "error" } else { "success" };
            fail(format!(
                "expected an error containing {text:?}, got {got} {result:?}"
            ))
        }
        other => anyhow::bail!("{other:?} is not an M9.12 step"),
    }
}
