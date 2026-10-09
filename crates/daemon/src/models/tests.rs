//! The probes against `/bin/sh` stand-ins that answer with fixed lines (M9.8.6).

use super::*;
use std::time::{Duration, Instant};
use tokio_util::sync::CancellationToken;

const WALL_CLOCK_SLACK: Duration = Duration::from_secs(2);

fn script(dir: &std::path::Path, name: &str, body: &str) -> String {
    let path = dir.join(name);
    testexec::write_executable(&path, format!("#!/bin/sh\n{body}\n"));
    path.to_str().unwrap().to_string()
}

/// Real-CLI manual check fix: the real Claude 2.1.280 `initialize` reply's models
/// (`fake-agent/fixtures/claude-initialize.json`), on one line.
fn claude_reply() -> String {
    let response: serde_json::Value = serde_json::from_str(include_str!(
        "../../../fake-agent/fixtures/claude-initialize.json"
    ))
    .unwrap();
    serde_json::json!({"type": "control_response", "response": {"subtype": "success",
        "request_id": "anthrex-models-1", "response": response}})
    .to_string()
}

#[test]
fn claude_probe_reads_the_initialize_reply_and_sends_no_prompt() {
    let dir = tempfile::tempdir().unwrap();
    let seen = dir.path().join("stdin");
    let program = script(
        dir.path(),
        "claude",
        &format!(
            "IFS= read -r line; printf '%s\\n' \"$line\" > '{}'\necho 'Warning: a banner that is not JSON'\necho '{}'\ncat >> '{}'",
            seen.display(),
            claude_reply(),
            seen.display()
        ),
    );
    let deadline = Instant::now() + DISCOVERY_TIMEOUT;
    let (models, raw) =
        claude_probe::probe(&program, deadline, &CancellationToken::new(), &[]).unwrap();
    assert_eq!(
        models.iter().map(|m| m.id.as_str()).collect::<Vec<_>>(),
        [
            "default",
            "opus[1m]",
            "claude-fable-5-1[1m]",
            "sonnet",
            "haiku"
        ]
    );
    assert_eq!(
        (models.iter())
            .map(|m| m.resolved.as_deref().unwrap_or("?"))
            .collect::<Vec<_>>(),
        [
            "claude-opus-5-5[1m]",
            "claude-opus-5-5[1m]",
            "claude-fable-5-1",
            "claude-sonnet-5",
            "claude-haiku-4-5-20251001"
        ]
    );
    assert!(models[0].is_default && !models[1].is_default);
    assert_eq!(models[1].efforts, ["low", "medium", "high", "xhigh", "max"]);
    assert_eq!(
        models[1].default_effort, None,
        "Claude reports no default effort"
    );
    assert!(
        models[4].efforts.is_empty(),
        "haiku reports no supportsEffort"
    );
    assert_eq!(raw.len(), 2, "{raw:?}");
    let lines: Vec<String> = std::fs::read_to_string(&seen)
        .unwrap()
        .lines()
        .map(str::to_string)
        .collect();
    assert_eq!(
        lines,
        [
            r#"{"type":"control_request","request_id":"anthrex-models-1","request":{"subtype":"initialize"}}"#
        ]
    );
}

#[test]
fn codex_probe_follows_the_cursor() {
    let dir = tempfile::tempdir().unwrap();
    let page = |id: u32, model: &str, next: &str| {
        format!(
            r#"{{"id":{id},"result":{{"data":[{{"id":"{model}","displayName":"{model}","description":"d","supportedReasoningEfforts":[{{"reasoningEffort":"low"}},"high"],"defaultReasoningEffort":"low","isDefault":false}}],"nextCursor":{next}}}}}"#
        )
    };
    let program = script(
        dir.path(),
        "codex",
        &format!(
            "read -r a; echo '{{\"id\":1,\"result\":{{}}}}'\nread -r b\nread -r c; echo '{}'\nread -r d; echo '{}'\ncat > /dev/null",
            page(2, "gpt-6-sol", "\"p2\""),
            page(3, "gpt-6.1-sol", "null")
        ),
    );
    let deadline = Instant::now() + DISCOVERY_TIMEOUT;
    let (models, _) =
        codex_probe::probe(&program, deadline, &CancellationToken::new(), &[]).unwrap();
    assert_eq!(
        models.iter().map(|m| m.id.as_str()).collect::<Vec<_>>(),
        ["gpt-6-sol", "gpt-6.1-sol"]
    );
    assert_eq!(models[0].efforts, ["low", "high"]);
    assert_eq!(models[0].default_effort.as_deref(), Some("low"));
}

#[test]
fn a_probe_past_its_deadline_is_killed_reaped_and_reported() {
    let dir = tempfile::tempdir().unwrap();
    let pid_file = dir.path().join("pid");
    // `warm` exits at once: run first, it pays macOS's first exec of a fresh file
    // (`docs/timing-budgets.md`, "Measured primitive costs") outside the 500 ms below,
    // so the probe's own run always reaches the pid line before its deadline.
    let program = script(
        dir.path(),
        "claude",
        &format!(
            "[ \"$1\" = warm ] && exit 0\necho $$ > '{}'\nexec sleep 30",
            pid_file.display()
        ),
    );
    let warm = std::process::Command::new(&program)
        .arg("warm")
        .status()
        .unwrap();
    assert!(warm.success());
    let started = Instant::now();
    let deadline = started + Duration::from_millis(500);
    let result = claude_probe::probe(&program, deadline, &CancellationToken::new(), &[]);
    assert_eq!(result.unwrap_err(), ProbeError::Timeout);
    assert!(
        started.elapsed() < Duration::from_millis(500) + REAP_RESERVE + WALL_CLOCK_SLACK,
        "{:?}",
        started.elapsed()
    );
    let pid: i32 = std::fs::read_to_string(&pid_file)
        .unwrap()
        .trim()
        .parse()
        .unwrap();
    // Our own child only, and only asked whether it exists (signal 0).
    assert_eq!(
        unsafe { libc::kill(pid, 0) },
        -1,
        "the probe's child {pid} is still alive"
    );
}

#[test]
fn a_missing_cli_is_missing() {
    let deadline = Instant::now() + DISCOVERY_TIMEOUT;
    let result = version::cli_version(
        "/nonexistent/anthrex-test/claude",
        deadline,
        &CancellationToken::new(),
    );
    assert_eq!(result.unwrap_err(), ProbeError::Missing);
}

#[test]
fn a_model_id_that_is_not_a_model_ref_is_dropped() {
    // Review focus 3: a CLI's id with a space or a control character never reaches argv.
    let dir = tempfile::tempdir().unwrap();
    let reply = r#"{"type":"control_response","response":{"subtype":"success","request_id":"anthrex-models-1","response":{"models":[{"value":"bad id","displayName":"x","description":"","supportsEffort":false},{"value":"claude-ok","displayName":"\u001b[31mOK\u202e","description":"","supportsEffort":false}]}}}"#;
    let program = script(
        dir.path(),
        "claude",
        &format!("read -r line; echo '{reply}'; cat > /dev/null"),
    );
    let (models, _) = claude_probe::probe(
        &program,
        Instant::now() + DISCOVERY_TIMEOUT,
        &CancellationToken::new(),
        &[],
    )
    .unwrap();
    assert_eq!(
        models.iter().map(|m| m.id.as_str()).collect::<Vec<_>>(),
        ["claude-ok"]
    );
    // Labels are kept as given; the TUI cleans them when drawing (decision 38).
    assert_eq!(models[0].label, "\u{1b}[31mOK\u{202e}");
}

#[test]
fn an_endless_reply_line_ends_the_probe() {
    let dir = tempfile::tempdir().unwrap();
    let program = script(dir.path(), "claude", "read -r line; yes x | tr -d '\\n'");
    let result = claude_probe::probe(
        &program,
        Instant::now() + DISCOVERY_TIMEOUT,
        &CancellationToken::new(),
        &[],
    );
    assert!(
        matches!(result, Err(ProbeError::Failed(ref m)) if m.contains("too long")),
        "{result:?}"
    );
}
