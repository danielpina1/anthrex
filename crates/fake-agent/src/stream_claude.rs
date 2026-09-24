//! Headless Claude (`claude -p --input-format stream-json --output-format stream-json`):
//! the lines it writes, only in the shapes of M8a.1's recordings
//! (`crates/daemon/tests/fixtures/headless/claude-2.1.278-*`, decision 51), and the
//! stdin envelopes it accepts.

use std::io::{self, BufRead};
use std::sync::mpsc::{self, Receiver};
use std::thread;

use anyhow::Result;
use serde_json::{Map, Value, json};

use crate::headless::{Events, Out, fresh_id, timestamp};
use crate::mcp::Reply;
use crate::roles;
use crate::script::Usage;

/// One accepted stdin line.
#[derive(Debug, Clone, PartialEq)]
pub enum Line {
    Message(String),
    Interrupt(String),
}

/// M8a.1's user-message envelope (`{"type":"user","message":{"role":"user","content":
/// [{"type":"text","text":…}]},"parent_tool_use_id":null,"session_id":…}`, with exactly
/// one text block and the last key optional) or its interrupt control request. Anything else is `None`.
pub fn parse_line(line: &str) -> Option<Line> {
    let Ok(Value::Object(object)) = serde_json::from_str::<Value>(line) else {
        return None;
    };
    match object.get("type")?.as_str()? {
        "user" => {
            let keys_ok = object.keys().all(|k| {
                matches!(
                    k.as_str(),
                    "type" | "message" | "parent_tool_use_id" | "session_id"
                )
            });
            if !keys_ok || !object.get("parent_tool_use_id")?.is_null() {
                return None;
            }
            if object.get("session_id").is_some_and(|id| !id.is_string()) {
                return None;
            }
            let message = object.get("message")?.as_object()?;
            if message.len() != 2 || message.get("role")? != "user" {
                return None;
            }
            // Exactly the recorded one text block.
            let [block] = message.get("content")?.as_array()?.as_slice() else {
                return None;
            };
            let block = block.as_object()?;
            if block.len() != 2 || block.get("type")? != "text" {
                return None;
            }
            Some(Line::Message(block.get("text")?.as_str()?.to_owned()))
        }
        "control_request" => {
            if object.len() != 3 {
                return None;
            }
            let id = object.get("request_id")?.as_str()?;
            let request = object.get("request")?.as_object()?;
            (request.len() == 1 && request.get("subtype")? == "interrupt")
                .then(|| Line::Interrupt(id.to_owned()))
        }
        _ => None,
    }
}

/// Reads stdin on its own thread: records every line, then sends each accepted one. A
/// line in any other shape exits the process with 5 at once, which is how a driver that
/// writes a wrong envelope fails a test. EOF closes the channel.
pub fn read_stdin(name: String) -> Receiver<Line> {
    let (tx, rx) = mpsc::channel();
    thread::spawn(move || {
        for line in io::stdin().lock().lines() {
            let Ok(line) = line else { return };
            if let Err(error) = roles::record_stdin(&name, &line) {
                eprintln!("fake-agent: {error:#}");
            }
            let Some(parsed) = parse_line(&line) else {
                eprintln!("fake-agent: bad input line");
                std::process::exit(5);
            };
            if tx.send(parsed).is_err() {
                return;
            }
        }
    });
    rx
}

/// Claude's events on stdout.
pub struct Claude {
    out: Out,
    session: String,
    cwd: String,
    model: String,
    permission_mode: String,
    mcp: bool,
    /// The turn's last assistant text, the `result`'s `result`.
    last_text: String,
    denials: Vec<Value>,
    /// The `tool_use` id of the step running now, if it is a tool.
    open_tool: Option<String>,
    counter: u64,
    /// The session's `result` count, its `result_index`.
    results: u64,
}

impl Claude {
    pub fn new(session: &str, model: Option<&str>, mode: Option<&str>, mcp: bool) -> Self {
        let cwd = std::env::current_dir()
            .map(|p| p.display().to_string())
            .unwrap_or_default();
        Self {
            out: Out,
            session: session.to_owned(),
            cwd,
            model: model.unwrap_or("claude-fake").to_owned(),
            permission_mode: mode.unwrap_or("default").to_owned(),
            mcp,
            last_text: String::new(),
            denials: Vec::new(),
            open_tool: None,
            counter: 0,
            results: 0,
        }
    }

    fn next(&mut self) -> u64 {
        self.counter += 1;
        self.counter
    }

    /// The keys every recorded top-level line of these types carries.
    fn line(&mut self, kind: &str, fields: Value) -> Result<()> {
        let mut object = Map::new();
        object.insert("type".into(), kind.into());
        if let Value::Object(fields) = fields {
            object.extend(fields);
        }
        object.insert("session_id".into(), self.session.clone().into());
        object.insert("uuid".into(), fresh_id().into());
        self.out.write(&Value::Object(object))
    }

    fn assistant(&mut self, block: Value, extra: Value) -> Result<()> {
        let id = format!("msg_fake{:04}", self.next());
        let synthetic = self.model == "<synthetic>";
        let mut fields = json!({
            "message": {
                "model": self.model,
                "id": id,
                "type": "message",
                "role": "assistant",
                "content": [block],
                "container": null,
                "stop_reason": if synthetic { json!("stop_sequence") } else { Value::Null },
                "stop_sequence": if synthetic { json!("") } else { Value::Null },
                "stop_details": null,
                "usage": {
                    "input_tokens": 1,
                    "cache_creation_input_tokens": 0,
                    "cache_read_input_tokens": 0,
                    "cache_creation": {"ephemeral_5m_input_tokens": 0, "ephemeral_1h_input_tokens": 0},
                    "output_tokens": 1,
                    "service_tier": "standard",
                    "inference_geo": "not_available",
                },
                "diagnostics": null,
                "context_management": null,
            },
            "parent_tool_use_id": null,
            "timestamp": timestamp(),
        });
        if let (Value::Object(fields), Value::Object(extra)) = (&mut fields, extra) {
            fields.extend(extra);
        }
        self.line("assistant", fields)
    }

    fn tool_use(&mut self, name: &str, input: Value) -> Result<String> {
        let id = format!("toolu_fake{:04}", self.next());
        let block = json!({
            "type": "tool_use",
            "id": id,
            "name": name,
            "input": input,
            "caller": {"type": "direct"},
        });
        self.assistant(block, json!({}))?;
        Ok(id)
    }

    fn tool_result(&mut self, id: &str, text: &str, is_error: bool) -> Result<()> {
        let block = json!({
            "tool_use_id": id,
            "type": "tool_result",
            "content": text,
            "is_error": is_error,
        });
        self.user(block)
    }

    fn user(&mut self, block: Value) -> Result<()> {
        self.line(
            "user",
            json!({
                "message": {"role": "user", "content": [block]},
                "parent_tool_use_id": null,
                "timestamp": timestamp(),
            }),
        )
    }

    fn result(&mut self, fields: Value, usage: Usage) -> Result<()> {
        let index = self.results;
        self.results += 1;
        let mut object = json!({
            "duration_ms": 1,
            "duration_api_ms": 1,
            "num_turns": 1,
            "stop_reason": "end_turn",
            "total_cost_usd": 0,
            "usage": {
                "input_tokens": usage.input,
                "cache_creation_input_tokens": usage.cache_write,
                "cache_read_input_tokens": usage.cache_read,
                "output_tokens": usage.output,
                "output_tokens_details": {"thinking_tokens": 0},
                "server_tool_use": {"web_search_requests": 0, "web_fetch_requests": 0},
                "service_tier": "standard",
                "cache_creation": {
                    "ephemeral_1h_input_tokens": usage.cache_write,
                    "ephemeral_5m_input_tokens": 0,
                },
                "inference_geo": "not_available",
                "iterations": [],
                "speed": "standard",
            },
            "modelUsage": {},
            "permission_denials": std::mem::take(&mut self.denials),
            "fast_mode_state": "off",
            "fast_mode_disabled_reason": "sdk_opt_in_required",
            "subagent_stats": {
                "spawned": 0,
                "requested": {"background": 0, "foreground": 0, "unset": 0},
                "started_in_background": 0,
                "max_depth": 0,
                "spawned_by_subagents": 0,
                "completed": 0,
                "failed": 0,
                "killed": {"parent": 0, "user": 0, "system": 0},
                "refused": {"depth_limit": 0, "concurrency_limit": 0, "budget": 0},
                "by_type": {},
            },
            "queued_turn_count": 0,
            "result_index": index,
        });
        if let (Value::Object(object), Value::Object(fields)) = (&mut object, fields) {
            object.extend(fields);
        }
        self.last_text.clear();
        self.open_tool = None;
        self.line("result", object)
    }
}

impl Events for Claude {
    fn turn_started(&mut self) -> Result<()> {
        let servers = if self.mcp {
            json!([{"name": "anthrex", "status": "connected", "source": "dynamic"}])
        } else {
            json!([])
        };
        let fields = json!({
            "subtype": "init",
            "cwd": self.cwd,
            "tools": ["Bash", "Edit", "Read", "Write"],
            "mcp_servers": servers,
            "model": self.model,
            "permissionMode": self.permission_mode,
            "slash_commands": [],
            "terminal_slash_commands": [],
            "apiKeySource": "none",
            "claude_code_version": "2.1.278",
            "output_style": "default",
            "agents": [],
            "skills": [],
            "plugins": [],
            "capabilities": ["interrupt_receipt_v1", "interrupt_cancel_queued_v1", "msg_lifecycle_v1"],
            "analytics_disabled": false,
            "product_feedback_disabled": false,
            "memory_paths": {"auto": ""},
            "messaging_socket_path": "",
            "fast_mode_state": "off",
            "fast_mode_disabled_reason": "sdk_opt_in_required",
        });
        self.line("system", fields)
    }

    fn text(&mut self, text: &str) -> Result<()> {
        self.last_text = text.to_owned();
        self.assistant(json!({"type": "text", "text": text}), json!({}))
    }

    fn command_started(&mut self, command: &str) -> Result<()> {
        let id = self.tool_use("Bash", json!({"command": command}))?;
        self.open_tool = Some(id);
        Ok(())
    }

    fn command_finished(&mut self, _command: &str, output: &str, code: i32) -> Result<()> {
        let id = self.open_tool.take().unwrap_or_default();
        let text = if code == 0 {
            output.to_owned()
        } else {
            format!("Exit code {code}\n{output}")
        };
        self.tool_result(&id, &text, code != 0)
    }

    fn mcp_started(&mut self, tool: &str, args: &Value) -> Result<()> {
        let id = self.tool_use(&format!("mcp__anthrex__{tool}"), args.clone())?;
        self.open_tool = Some(id);
        Ok(())
    }

    fn mcp_finished(&mut self, _tool: &str, _args: &Value, reply: &Reply) -> Result<()> {
        let id = self.open_tool.take().unwrap_or_default();
        self.tool_result(&id, &reply.text, !reply.ok)
    }

    fn deny(&mut self, tool: &str, reason: &str) -> Result<()> {
        let id = self.tool_use(tool, json!({}))?;
        let fields = json!({
            "subtype": "permission_denied",
            "tool_name": tool,
            "tool_use_id": id,
            "decision_reason_type": "asyncAgent",
            "decision_reason": reason,
            "message": reason,
        });
        self.line("system", fields)?;
        self.denials
            .push(json!({"tool_name": tool, "tool_use_id": id, "tool_input": {}}));
        self.tool_result(&id, reason, true)
    }

    fn api_retry(&mut self, error: &str, attempt: u32, delay_ms: u64) -> Result<()> {
        let fields = json!({
            "subtype": "api_retry",
            "attempt": attempt,
            "max_retries": 10,
            "retry_delay_ms": delay_ms,
            "error_status": error_status(error),
            "error": error,
        });
        self.line("system", fields)
    }

    fn turn_completed(&mut self, usage: Usage) -> Result<()> {
        let text = self.last_text.clone();
        let fields = json!({
            "subtype": "success",
            "is_error": false,
            "result": text,
            "terminal_reason": "completed",
            "api_error_status": null,
        });
        self.result(fields, usage)
    }

    /// M8a.1 items 3 and 5: the category is on a synthetic `assistant` line, and the
    /// `result` is `success` with `is_error` and `terminal_reason: "api_error"`.
    fn turn_failed(&mut self, error: &str, usage: Usage) -> Result<()> {
        let text = failure_text(error);
        let block = json!({"type": "text", "text": text});
        let saved = std::mem::replace(&mut self.model, "<synthetic>".into());
        let extra = json!({"error": error, "is_api_error_message": true});
        let written = self.assistant(block, extra);
        self.model = saved;
        written?;
        let fields = json!({
            "subtype": "success",
            "is_error": true,
            "result": text,
            "stop_reason": "stop_sequence",
            "terminal_reason": "api_error",
            "api_error_status": error_status(error),
        });
        self.result(fields, usage)
    }

    /// M8a.1 item 2's interrupt: inside a tool step, the recorded rejection
    /// `tool_result`, the `[Request interrupted by user for tool use]` text and
    /// `aborted_tools` / `stop_reason: "tool_use"`; outside one, `aborted_streaming`.
    fn interrupted(&mut self, request_id: &str, usage: Usage) -> Result<()> {
        self.control_response(request_id)?;
        let (terminal, stop) = match self.open_tool.take() {
            Some(id) => {
                self.tool_result(&id, TOOL_REJECTED, true)?;
                let text = "[Request interrupted by user for tool use]";
                self.user(json!({"type": "text", "text": text}))?;
                ("aborted_tools", "tool_use")
            }
            // Unrecorded: `stop_reason` is a string in the only recording.
            None => ("aborted_streaming", "end_turn"),
        };
        let fields = json!({
            "subtype": "error_during_execution",
            "is_error": true,
            "stop_reason": stop,
            "terminal_reason": terminal,
            "errors": [],
        });
        self.result(fields, usage)
    }

    fn control_response(&mut self, request_id: &str) -> Result<()> {
        let line = json!({"type": "control_response", "response": {
            "subtype": "success",
            "request_id": request_id,
            "response": {"still_queued": []},
        }});
        self.out.write(&line)
    }
}

/// The recorded text of a tool the user interrupted.
const TOOL_REJECTED: &str = "The user doesn't want to proceed with this tool use. The tool use was rejected (eg. if it was a file edit, the new_string was NOT written to the file). STOP what you are doing and wait for the user to tell you how to proceed.";

/// The status the documented `api_retry` lines pair with each category.
fn error_status(error: &str) -> u16 {
    match error {
        "rate_limit" => 429,
        "overloaded" => 529,
        "authentication_failed" | "oauth_org_not_allowed" => 401,
        "billing_error" | "invalid_request" => 400,
        "model_not_found" => 404,
        _ => 500,
    }
}

/// The recorded or documented text of a failed turn (M8a.1 items 3 and 5).
fn failure_text(error: &str) -> String {
    match error {
        "rate_limit" => "You've hit your session limit · resets 3:45pm".into(),
        "billing_error" => "Credit balance is too low".into(),
        "authentication_failed" => "Not logged in · Please run /login".into(),
        other => format!("API Error: {other}"),
    }
}

#[cfg(test)]
mod tests {
    use super::{Line, parse_line};

    #[test]
    fn accepts_the_recorded_envelope_and_interrupt() {
        let recorded =
            include_str!("../../daemon/tests/fixtures/headless/claude-2.1.278-input.jsonl");
        let parsed: Vec<Option<Line>> = recorded.lines().map(parse_line).collect();
        assert!(matches!(&parsed[0], Some(Line::Message(t)) if t.starts_with("Run `ls`")));
        assert_eq!(parsed[3], Some(Line::Interrupt("1".into())));
        assert!(parsed.iter().all(Option::is_some));
    }

    #[test]
    fn refuses_every_other_shape() {
        for line in [
            "hello",
            "[]",
            r#"{"type":"user","message":"hi"}"#,
            r#"{"type":"user","message":{"role":"user","content":"hi"},"parent_tool_use_id":null}"#,
            r#"{"type":"user","message":{"role":"assistant","content":[{"type":"text","text":"x"}]},"parent_tool_use_id":null}"#,
            r#"{"type":"user","message":{"role":"user","content":[{"type":"text","text":"x"}]},"parent_tool_use_id":"t"}"#,
            r#"{"type":"user","message":{"role":"user","content":[{"type":"text","text":"x"}]},"parent_tool_use_id":null,"extra":1}"#,
            r#"{"type":"user","message":{"role":"user","content":[]},"parent_tool_use_id":null}"#,
            r#"{"type":"user","message":{"role":"user","content":[{"type":"text","text":"x"},{"type":"text","text":"y"}]},"parent_tool_use_id":null}"#,
            r#"{"type":"control_request","request_id":"1","request":{"subtype":"can_use_tool"}}"#,
            r#"{"type":"control_request","request_id":1,"request":{"subtype":"interrupt"}}"#,
        ] {
            assert_eq!(parse_line(line), None, "{line}");
        }
        let without_session = r#"{"type":"user","message":{"role":"user","content":[{"type":"text","text":"x"}]},"parent_tool_use_id":null}"#;
        assert_eq!(parse_line(without_session), Some(Line::Message("x".into())));
    }
}
