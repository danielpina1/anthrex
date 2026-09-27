//! Helpers for the M8b.6 tests (`adapt_modes.rs`, `adapt_edges.rs`): a scout's argv,
//! and headless Claude sessions that run `bash` steps under `--settings` hooks.

use std::path::Path;

use serde_json::{Value, json};

use super::{Agent, RUN, is_result, lines, user_message, write_steps};

/// A headless Claude argv with an anthrex MCP server of role `scout`, as M8b.9's
/// `mcp_args` writes it.
pub fn scout_argv(scout: &str) -> Vec<String> {
    let config = json!({"mcpServers": {"anthrex": {
        "type": "stdio",
        "command": "/nonexistent/anthrex",
        "args": ["mcp", "--role", "scout", "--scout", scout, "--window", "7",
                 "--socket", "/tmp/nonexistent.sock"],
    }}});
    let mut args: Vec<String> = ["-p", "--input-format", "stream-json"]
        .map(String::from)
        .to_vec();
    args.extend(["--output-format".into(), "stream-json".into()]);
    args.extend(["--mcp-config".into(), config.to_string()]);
    args
}

/// A headless Claude argv with `settings` as `--settings`, and no MCP server.
pub fn claude_with_settings(settings: Option<&Value>) -> Vec<String> {
    let mut args: Vec<String> = ["-p", "--input-format", "stream-json"]
        .map(String::from)
        .to_vec();
    args.extend(["--output-format".into(), "stream-json".into()]);
    if let Some(settings) = settings {
        args.extend(["--settings".into(), settings.to_string()]);
    }
    args
}

pub fn group(matcher: &str, commands: &[String]) -> Value {
    let hooks: Vec<Value> = commands
        .iter()
        .map(|c| json!({"type": "command", "command": c}))
        .collect();
    json!({"matcher": matcher, "hooks": hooks})
}

/// Runs `steps` in a headless Claude session with `settings`; returns the agent and
/// the bash log's entries.
pub fn run_bash(dir: &Path, settings: Option<&Value>, steps: &[Value]) -> (Agent, Vec<Value>) {
    let script = write_steps(&dir.join("script.jsonl"), steps);
    let log = dir.join("bash.jsonl");
    let mut agent = Agent::spawn(
        &claude_with_settings(settings),
        dir,
        &[
            ("FAKE_AGENT_SCRIPT", &script),
            ("FAKE_AGENT_BASH_LOG", &log),
        ],
    );
    agent.send(&user_message("go", "s"));
    agent.until(RUN, is_result);
    agent.close_stdin();
    assert!(agent.wait(RUN).success(), "stderr {}", agent.stderr());
    let entries = lines(&log)
        .iter()
        .map(|l| serde_json::from_str(l).unwrap())
        .collect();
    (agent, entries)
}

/// The Bash `tool_use` inputs and `tool_result` contents, in order.
pub fn bash_pairs(agent: &Agent) -> Vec<(Value, Value)> {
    let blocks: Vec<Value> = agent
        .seen
        .iter()
        .filter(|v| v["type"] == "assistant" || v["type"] == "user")
        .map(|v| v["message"]["content"][0].clone())
        .collect();
    let uses = blocks.iter().filter(|b| b["type"] == "tool_use");
    let results = blocks.iter().filter(|b| b["type"] == "tool_result");
    uses.zip(results)
        .map(|(u, r)| (u["input"].clone(), r["content"].clone()))
        .collect()
}
