//! Milestone 9 task M9.16: the steps a scripted orchestrator (`orchestrator-run-<n>`)
//! and its workers run, as `fake-agent` reads them (M9.12's `mcp_until`,
//! `capture_json`, `expect`, `expect_error_contains`), and the plan tasks they add.

use std::path::Path;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde_json::{Value, json};

/// The `wait_secs` every [`until`] passes. It passes no `since`, so each call answers at
/// once (decision 16); a real long-poll captures `/revision` and passes it as
/// `"since": "{{#rev}}"` (`fake-agent` fills a whole `{{#name}}` as JSON). The calls'
/// bound, `wait_secs + 5` s, is asserted from the MCP log's `ms`.
pub const STATUS_WAIT_SECS: u64 = 5;

/// A turn begins: the window is `Working`, so no wake-up is pasted while the script
/// calls its tools (decision 39 delivers only to an idle window).
pub fn prompt() -> Value {
    json!({"hook": "UserPromptSubmit", "payload": {"prompt": "plan"}})
}

/// `mcp_call` of `tool` with `args`; any error reply ends the script (exit 3).
pub fn call(tool: &str, args: Value) -> Value {
    json!({"mcp_call": {"tool": tool, "args": args}})
}

/// `mcp_call` of `tool` with `args` that must answer an error.
pub fn call_err(tool: &str, args: Value) -> Value {
    json!({"mcp_call": {"tool": tool, "args": args, "expect_error": true}})
}

/// `edit_plan` with `edits` and the extra keys of `more` (`submit`, `summary`).
pub fn edit_plan(edits: Vec<Value>, more: Value) -> Value {
    let mut args = json!({"edits": edits});
    for (key, value) in more.as_object().cloned().unwrap_or_default() {
        args[key] = value;
    }
    call("edit_plan", args)
}

/// `run_status` until `pointer` holds `equals`, for at most `timeout`.
pub fn until(pointer: &str, equals: Value, timeout: Duration) -> Value {
    json!({"mcp_until": {
        "tool": "run_status",
        "args": {"wait_secs": STATUS_WAIT_SECS},
        "until": {"pointer": pointer, "equals": equals},
        "timeout_ms": u64::try_from(timeout.as_millis()).unwrap(),
    }})
}

pub fn capture_json(name: &str, pointer: &str) -> Value {
    json!({"capture_json": {"name": name, "pointer": pointer}})
}

pub fn expect(pointer: &str, equals: Value) -> Value {
    json!({"expect": {"pointer": pointer, "equals": equals}})
}

pub fn expect_error(text: &str) -> Value {
    json!({"expect_error_contains": {"text": text}})
}

/// A `run_status` with `wait_secs = 0`, which no other step sends: the script got past
/// every expectation before it ([`passed`]).
pub fn marker() -> Value {
    call("run_status", json!({"wait_secs": 0}))
}

/// How many [`marker`]s `script` reached, from the MCP log's lines.
pub fn passed(log: &[Value], script: &str) -> usize {
    log.iter()
        .filter(|l| l["script"] == script && l["tool"] == "run_status")
        .filter(|l| l["args"] == json!({"wait_secs": 0}))
        .count()
}

/// `read_message`, optionally checking the text contains `expect` (exit 3 otherwise).
pub fn read(expect: Option<&str>) -> Value {
    match expect {
        Some(text) => json!({"read_message": {"expect": text}}),
        None => json!({"read_message": {}}),
    }
}

/// A worker's `sh` step that waits, at most one `RUN_WAIT` (1500 × 0.2 s), for `path`,
/// with the turn open.
pub fn wait_file(path: &Path) -> Value {
    json!({"sh": {"cmd": format!(
        "for i in $(seq 1 1500); do [ -e '{}' ] && exit 0; sleep 0.2; done; exit 1",
        path.display()
    )}})
}

/// An S `check`-mode code task owning `owns`, with the extra keys of `more`.
pub fn plan_task(id: &str, owns: &[&str], more: Value) -> Value {
    let mut task = json!({
        "id": id, "title": format!("Task {id}"), "brief": format!("Do {id}."),
        "acceptance": [format!("{id} is done")], "owns": owns, "size": "S",
        "test_mode": "check", "test_mode_reason": "a text file",
    });
    for (key, value) in more.as_object().cloned().unwrap_or_default() {
        task[key] = value;
    }
    task
}

/// `add_task` of `task`.
pub fn add(task: Value) -> Value {
    json!({"op": "add_task", "task": task})
}

/// `fake-agent`'s timestamps (`2026-09-29T12:00:00.123Z`, UTC) as milliseconds since
/// the epoch.
pub fn epoch_ms(stamp: &str) -> u64 {
    let n = |range: std::ops::Range<usize>| -> i64 { stamp[range].parse().unwrap() };
    let (year, month, day) = (n(0..4), n(5..7), n(8..10));
    // Howard Hinnant's days-from-civil.
    let y = if month <= 2 { year - 1 } else { year };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let mp = (month + 9) % 12;
    let doy = (153 * mp + 2) / 5 + day - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146_097 + doe - 719_468;
    let secs = days * 86_400 + n(11..13) * 3_600 + n(14..16) * 60 + n(17..19);
    u64::try_from(secs * 1_000 + n(20..23)).unwrap()
}

/// Now, in milliseconds since the epoch, on the clock `fake-agent` stamps with.
pub fn now_ms() -> u64 {
    u64::try_from(
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_millis(),
    )
    .unwrap()
}
