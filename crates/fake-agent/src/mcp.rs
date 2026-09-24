//! The `mcp_call` step (M8a.20): spawn the session's configured MCP server, then
//! `initialize` (protocol `2025-06-18`), `notifications/initialized` and one
//! `tools/call`, over newline-delimited JSON-RPC on the server's stdio.

use std::collections::BTreeMap;
use std::io::{BufRead, BufReader, Write};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError};
use std::thread;
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail};
use serde_json::{Value, json};

use crate::runtime::McpServer;

/// A bound on one whole call: Codex's own `tool_timeout_sec` for the anthrex server
/// (`daemon::headless::argv::codex_args`), which covers `anthrex mcp`'s
/// `TOOL_REPLY_TIMEOUT` (100 s) to the daemon.
pub const MCP_CALL_TIMEOUT: Duration = Duration::from_secs(120);
/// How long the server may take to exit once its stdin is closed.
const EXIT_GRACE: Duration = Duration::from_secs(2);

pub const PROTOCOL_VERSION: &str = "2025-06-18";

/// A tool's answer: `ok` is false for an `isError` result or a JSON-RPC error.
#[derive(Debug, Clone, PartialEq)]
pub struct Reply {
    pub ok: bool,
    pub text: String,
}

/// Replaces `{{name}}` with each `capture`'s value in every string of `args`.
pub fn fill(args: &Value, captures: &BTreeMap<String, String>) -> Value {
    match args {
        Value::String(text) => {
            let mut text = text.clone();
            for (name, value) in captures {
                text = text.replace(&format!("{{{{{name}}}}}"), value);
            }
            Value::String(text)
        }
        Value::Array(items) => Value::Array(items.iter().map(|v| fill(v, captures)).collect()),
        Value::Object(object) => Value::Object(
            object
                .iter()
                .map(|(k, v)| (k.clone(), fill(v, captures)))
                .collect(),
        ),
        other => other.clone(),
    }
}

/// Calls `tool` with `args` on a fresh `server` process.
pub fn call(server: &McpServer, tool: &str, args: &Value) -> Result<Reply> {
    let mut command = Command::new(&server.command);
    command
        .args(&server.args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit());
    crate::isolate_process_group(&mut command);
    let mut child = command
        .spawn()
        .with_context(|| format!("spawn the MCP server {}", server.command))?;
    let group = child.id() as libc::pid_t;
    let result = converse(&mut child, tool, args);
    finish(&mut child, group);
    result
}

fn converse(child: &mut Child, tool: &str, args: &Value) -> Result<Reply> {
    let deadline = Instant::now() + MCP_CALL_TIMEOUT;
    let mut stdin = child.stdin.take().context("MCP stdin was not piped")?;
    let lines = read_lines(child.stdout.take().context("MCP stdout was not piped")?);

    let init = json!({
        "protocolVersion": PROTOCOL_VERSION,
        "capabilities": {},
        "clientInfo": {"name": "fake-agent", "version": "0"},
    });
    send(&mut stdin, &request(1, "initialize", init))?;
    let reply = response(&lines, 1, deadline)?;
    if let Some(error) = reply.get("error") {
        bail!("the MCP server refused initialize: {error}");
    }
    send(
        &mut stdin,
        &json!({"jsonrpc": "2.0", "method": "notifications/initialized"}),
    )?;
    let params = json!({"name": tool, "arguments": args});
    send(&mut stdin, &request(2, "tools/call", params))?;
    let reply = response(&lines, 2, deadline)?;
    drop(stdin);

    if let Some(error) = reply.get("error") {
        let text = error["message"].as_str().unwrap_or("JSON-RPC error");
        return Ok(Reply {
            ok: false,
            text: text.to_owned(),
        });
    }
    let result = &reply["result"];
    let text = result["content"]
        .as_array()
        .map(|parts| {
            parts
                .iter()
                .filter_map(|part| part["text"].as_str())
                .collect::<Vec<_>>()
                .join("\n")
        })
        .unwrap_or_default();
    Ok(Reply {
        ok: result["isError"] != json!(true),
        text,
    })
}

fn request(id: u64, method: &str, params: Value) -> Value {
    json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params})
}

fn send(stdin: &mut ChildStdin, message: &Value) -> Result<()> {
    let mut line = serde_json::to_vec(message).context("encode JSON-RPC")?;
    line.push(b'\n');
    stdin.write_all(&line).context("write to the MCP server")?;
    stdin.flush().context("flush to the MCP server")
}

fn read_lines(stdout: impl std::io::Read + Send + 'static) -> Receiver<String> {
    let (tx, rx) = mpsc::channel();
    thread::spawn(move || {
        for line in BufReader::new(stdout).lines() {
            let Ok(line) = line else { return };
            if tx.send(line).is_err() {
                return;
            }
        }
    });
    rx
}

/// The response with `id`, skipping notifications and anything else.
fn response(lines: &Receiver<String>, id: u64, deadline: Instant) -> Result<Value> {
    loop {
        let left = deadline.saturating_duration_since(Instant::now());
        match lines.recv_timeout(left) {
            Ok(line) => {
                let Ok(value) = serde_json::from_str::<Value>(&line) else {
                    continue;
                };
                if value["id"] == json!(id) {
                    return Ok(value);
                }
            }
            Err(RecvTimeoutError::Timeout) => {
                bail!("the MCP server did not answer within {MCP_CALL_TIMEOUT:?}")
            }
            Err(RecvTimeoutError::Disconnected) => {
                bail!("the MCP server closed its output before answering")
            }
        }
    }
}

/// Lets the server exit at its stdin's EOF, then kills its group.
fn finish(child: &mut Child, group: libc::pid_t) {
    let deadline = Instant::now() + EXIT_GRACE;
    while Instant::now() < deadline {
        if let Ok(Some(_)) = child.try_wait() {
            return;
        }
        thread::sleep(Duration::from_millis(10));
    }
    crate::kill_process_group(group);
    let _ = child.wait();
}

#[cfg(test)]
mod tests {
    use super::fill;
    use serde_json::json;
    use std::collections::BTreeMap;

    #[test]
    fn fill_replaces_every_capture_in_nested_strings() {
        let captures = BTreeMap::from([("red".to_string(), "abc1234".to_string())]);
        let args = json!({"red": "{{red}}", "list": ["x {{red}} {{red}}", 3], "n": null,
            "other": "{{green}}"});

        assert_eq!(
            fill(&args, &captures),
            json!({"red": "abc1234", "list": ["x abc1234 abc1234", 3], "n": null,
                "other": "{{green}}"})
        );
    }
}
