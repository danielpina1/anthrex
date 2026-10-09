//! M9.8.5: the model-discovery handshakes. Claude's `initialize` control request (the
//! first stdin line of a stream-json session) and Codex's `app-server` JSON-RPC
//! (`initialize`, then paged `model/list`), answered from fixtures so no test reaches
//! a real agent.
//!
//! `FAKE_AGENT_MODELS_DIR` replaces the built-in fixtures with that directory's files of
//! the same names. `FAKE_AGENT_DISCOVERY` is `hang` (answer nothing, never exit until
//! killed), `garbage` (`not json` before each valid reply) or `empty` (no models).

use std::env;
use std::fs;
use std::io::{self, BufRead, Write};
use std::path::PathBuf;

use anyhow::{Context, Result};
use serde_json::{Value, json};

use crate::roles;

const CLAUDE_FIXTURE: &str = "claude-initialize.json";
const CODEX_FIXTURE: &str = "codex-model-list.json";
const BUILTIN_CLAUDE: &str = include_str!("../fixtures/claude-initialize.json");
const BUILTIN_CODEX: &str = include_str!("../fixtures/codex-model-list.json");

#[derive(PartialEq)]
enum Mode {
    Normal,
    Hang,
    Garbage,
    Empty,
}

fn mode() -> Mode {
    match env::var("FAKE_AGENT_DISCOVERY").as_deref() {
        Ok("hang") => Mode::Hang,
        Ok("garbage") => Mode::Garbage,
        Ok("empty") => Mode::Empty,
        _ => Mode::Normal,
    }
}

fn fixture(name: &str, builtin: &str) -> Result<Value> {
    let text = match env::var_os("FAKE_AGENT_MODELS_DIR") {
        Some(dir) => {
            let path = PathBuf::from(dir).join(name);
            fs::read_to_string(&path).with_context(|| format!("read {}", path.display()))?
        }
        None => builtin.to_string(),
    };
    serde_json::from_str(&text).with_context(|| format!("parse fixture {name}"))
}

/// The request id when `first` is Claude's `initialize` control request; the line is
/// recorded as `discovery-claude`.
pub fn claude_initialize(first: Option<&str>) -> Option<Value> {
    let line = first?;
    let value: Value = serde_json::from_str(line).ok()?;
    if value["type"] != "control_request" || value["request"]["subtype"] != "initialize" {
        return None;
    }
    // A failed record is a test-harness fault, not a protocol one: say so and go on.
    if let Err(error) = roles::record_stdin("discovery-claude", line) {
        crate::diag::say!("fake-agent: {error:#}");
    }
    Some(value["request_id"].clone())
}

/// Answers `initialize` (its first line was read and recorded by `claude_initialize`), then reads
/// stdin to EOF. No turn runs.
pub fn answer_claude(id: &Value, _args: &[String]) -> Result<i32> {
    let mode = mode();
    let mut out = io::stdout().lock();
    if mode != Mode::Hang {
        let mut response = fixture(CLAUDE_FIXTURE, BUILTIN_CLAUDE)?;
        if mode == Mode::Empty {
            response["models"] = json!([]);
        }
        if mode == Mode::Garbage {
            writeln!(out, "not json")?;
        }
        let reply = json!({"type": "control_response",
            "response": {"subtype": "success", "request_id": id, "response": response}});
        writeln!(out, "{reply}")?;
        out.flush()?;
    }
    drain_stdin(mode == Mode::Hang)?;
    Ok(0)
}

fn drain_stdin(hang: bool) -> Result<()> {
    io::copy(&mut io::stdin().lock(), &mut io::sink()).context("read stdin to EOF")?;
    if hang {
        loop {
            std::thread::sleep(std::time::Duration::from_secs(60));
        }
    }
    Ok(())
}

/// `codex app-server`: newline-delimited JSON-RPC without a `jsonrpc` member.
pub fn codex_app_server(args: &[String]) -> Result<Option<i32>> {
    if args.first().map(String::as_str) != Some("app-server") {
        return Ok(None);
    }
    let mode = mode();
    let mut pages = fixture(CODEX_FIXTURE, BUILTIN_CODEX)?;
    if mode == Mode::Empty {
        pages = json!([{"data": [], "nextCursor": null}]);
    }
    let mut out = io::stdout().lock();
    for line in io::stdin().lock().lines() {
        let line = line.context("read stdin")?;
        roles::record_stdin("discovery-codex", &line)?;
        if mode == Mode::Hang {
            continue;
        }
        let Ok(request) = serde_json::from_str::<Value>(&line) else {
            continue;
        };
        // A notification (no id) is never answered.
        let Some(id) = request.get("id").cloned() else {
            continue;
        };
        let reply = match request["method"].as_str() {
            Some("initialize") => json!({"id": id, "result": {"userAgent": "fake-agent"}}),
            Some("model/list") => match page(&pages, &request["params"]["cursor"]) {
                Some(page) => json!({"id": id, "result": page}),
                None => json!({"id": id, "error": {"code": -32602, "message": "unknown cursor"}}),
            },
            _ => json!({"id": id, "error": {"code": -32601, "message": "method not found"}}),
        };
        if mode == Mode::Garbage {
            writeln!(out, "not json")?;
        }
        writeln!(out, "{reply}")?;
        out.flush()?;
    }
    drain_stdin(mode == Mode::Hang)?;
    Ok(Some(0))
}

/// Page for `cursor`: `null` is page 0; otherwise the page after the one whose
/// `nextCursor` is `cursor`.
fn page<'a>(pages: &'a Value, cursor: &Value) -> Option<&'a Value> {
    let pages = pages.as_array()?;
    if cursor.is_null() {
        return pages.first();
    }
    let at = pages
        .iter()
        .position(|page| page["nextCursor"] == *cursor)?;
    pages.get(at + 1)
}
