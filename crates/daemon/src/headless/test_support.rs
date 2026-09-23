//! M8a.1's recorded fixtures (decision 51), read through `include_str!` so a new fixture
//! version is a new test, not an edit.

pub const CLAUDE_STREAM: &str =
    include_str!("../../tests/fixtures/headless/claude-2.1.278-stream.jsonl");
pub const CLAUDE_STREAM_META: &str =
    include_str!("../../tests/fixtures/headless/claude-2.1.278-stream.meta.json");
pub const CLAUDE_INPUT: &str =
    include_str!("../../tests/fixtures/headless/claude-2.1.278-input.jsonl");
pub const CLAUDE_HOOKS: &str =
    include_str!("../../tests/fixtures/headless/claude-2.1.278-hooks.jsonl");
pub const CLAUDE_DOCUMENTED: &str =
    include_str!("../../tests/fixtures/headless/claude-2.1.278-documented.jsonl");
pub const CLAUDE_SANDBOX: &str =
    include_str!("../../tests/fixtures/headless/claude-2.1.278-sandbox.jsonl");
pub const CLAUDE_PROJECT_SETTINGS: &str =
    include_str!("../../tests/fixtures/headless/claude-2.1.278-project-settings.jsonl");
pub const CODEX_EXEC: &str = include_str!("../../tests/fixtures/headless/codex-0.155.0-exec.jsonl");
pub const CODEX_RESUME: &str =
    include_str!("../../tests/fixtures/headless/codex-0.155.0-resume.jsonl");
pub const CODEX_PROJECT_CONFIG: &str =
    include_str!("../../tests/fixtures/headless/codex-0.155.0-project-config.jsonl");

/// The non-empty lines of a fixture, in order.
pub fn lines(fixture: &str) -> Vec<&str> {
    fixture.lines().filter(|l| !l.trim().is_empty()).collect()
}

/// A line's type as the meta files' `unmodelled` list spells it: `type`, or
/// `type/subtype` for `system` lines.
pub fn line_type(line: &str) -> String {
    let value: serde_json::Value = serde_json::from_str(line).expect("fixture lines are JSON");
    let kind = value["type"].as_str().unwrap_or_default();
    match value["subtype"].as_str() {
        Some(subtype) if kind == "system" => format!("{kind}/{subtype}"),
        _ => kind.to_string(),
    }
}

/// The stream meta's `unmodelled` list.
pub fn unmodelled() -> Vec<String> {
    let meta: serde_json::Value = serde_json::from_str(CLAUDE_STREAM_META).unwrap();
    meta["unmodelled"]
        .as_array()
        .expect("the stream meta lists its unmodelled types")
        .iter()
        .map(|v| v.as_str().unwrap().to_string())
        .collect()
}
