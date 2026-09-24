//! Headless Codex (`codex exec --json`, one process per turn): the lines it writes, only
//! in the shapes of M8a.1's recordings (`crates/daemon/tests/fixtures/headless/codex-*`,
//! decision 51).

use anyhow::Result;
use serde_json::{Value, json};

use crate::headless::{Events, Out};
use crate::mcp::Reply;
use crate::script::Usage;

/// The recorded usage-limit text (`codex-0.156.1-usage-limit.jsonl`).
pub const USAGE_LIMIT: &str = "You’ve hit your usage limit. Visit https://chatgpt.com/codex/settings/usage to purchase more credits or try again at Sep 25th, 2026 11:33 AM.";

/// Codex's events on stdout. Item ids restart in every process, as recorded.
pub struct Codex {
    out: Out,
    thread: String,
    items: u64,
    open_item: Option<String>,
}

impl Codex {
    pub fn new(thread: &str) -> Self {
        Self {
            out: Out,
            thread: thread.to_owned(),
            items: 0,
            open_item: None,
        }
    }

    pub fn thread_started(&mut self) -> Result<()> {
        let line = json!({"type": "thread.started", "thread_id": self.thread});
        self.out.write(&line)
    }

    fn item_id(&mut self) -> String {
        let id = format!("item_{}", self.items);
        self.items += 1;
        id
    }

    fn item(&mut self, phase: &str, item: Value) -> Result<()> {
        self.out.write(&json!({"type": phase, "item": item}))
    }
}

impl Events for Codex {
    fn turn_started(&mut self) -> Result<()> {
        self.out.write(&json!({"type": "turn.started"}))
    }

    fn text(&mut self, text: &str) -> Result<()> {
        let id = self.item_id();
        let item = json!({"id": id, "type": "agent_message", "text": text});
        self.item("item.completed", item)
    }

    /// Written once the command has exited: see `command_finished`.
    fn command_started(&mut self, _command: &str) -> Result<()> {
        Ok(())
    }

    /// M8a.1 item 7: a command that exits non-zero is not reported at all.
    fn command_finished(&mut self, command: &str, output: &str, code: i32) -> Result<()> {
        if code != 0 {
            return Ok(());
        }
        let id = self.item_id();
        let started = json!({
            "id": id,
            "type": "command_execution",
            "command": command,
            "aggregated_output": "",
            "exit_code": null,
            "status": "in_progress",
        });
        self.item("item.started", started)?;
        let completed = json!({
            "id": id,
            "type": "command_execution",
            "command": command,
            "aggregated_output": output,
            "exit_code": 0,
            "status": "completed",
        });
        self.item("item.completed", completed)
    }

    fn mcp_started(&mut self, tool: &str, args: &Value) -> Result<()> {
        let id = self.item_id();
        self.open_item = Some(id.clone());
        let item = json!({
            "id": id,
            "type": "mcp_tool_call",
            "server": "anthrex",
            "tool": tool,
            "arguments": args,
            "result": null,
            "error": null,
            "status": "in_progress",
        });
        self.item("item.started", item)
    }

    /// The recorded completed call (`-mcp-approval-approve`) or failed one (`-auto`).
    fn mcp_finished(&mut self, tool: &str, args: &Value, reply: &Reply) -> Result<()> {
        let id = self.open_item.take().unwrap_or_else(|| self.item_id());
        let (result, error, status) = if reply.ok {
            let content = json!({
                "content": [{"type": "text", "text": reply.text}],
                "structured_content": null,
            });
            (content, Value::Null, "completed")
        } else {
            (Value::Null, json!({"message": reply.text}), "failed")
        };
        let item = json!({
            "id": id,
            "type": "mcp_tool_call",
            "server": "anthrex",
            "tool": tool,
            "arguments": args,
            "result": result,
            "error": error,
            "status": status,
        });
        self.item("item.completed", item)
    }

    /// Codex has no permission-denied event (M8a.1 item 7).
    fn deny(&mut self, _tool: &str, _reason: &str) -> Result<()> {
        Ok(())
    }

    /// Codex has no retry event.
    fn api_retry(&mut self, _error: &str, _attempt: u32, _delay_ms: u64) -> Result<()> {
        Ok(())
    }

    /// `input_tokens` includes the cached part, as recorded.
    fn turn_completed(&mut self, usage: Usage) -> Result<()> {
        let line = json!({"type": "turn.completed", "usage": {
            "input_tokens": usage.input + usage.cache_read,
            "cached_input_tokens": usage.cache_read,
            "cache_write_input_tokens": usage.cache_write,
            "output_tokens": usage.output,
            "reasoning_output_tokens": 0,
        }});
        self.out.write(&line)
    }

    /// The recorded usage-limit pair (`error`, then `turn.failed`) for `rate_limit`.
    fn turn_failed(&mut self, error: &str, _usage: Usage) -> Result<()> {
        let message = if error == "rate_limit" {
            USAGE_LIMIT.to_owned()
        } else {
            error.to_owned()
        };
        self.out
            .write(&json!({"type": "error", "message": message}))?;
        self.out
            .write(&json!({"type": "turn.failed", "error": {"message": message}}))
    }

    /// Never called: a Codex turn is interrupted by a signal, and ends with no line.
    fn interrupted(&mut self, _request_id: &str, _usage: Usage) -> Result<()> {
        Ok(())
    }

    fn control_response(&mut self, _request_id: &str) -> Result<()> {
        Ok(())
    }
}
