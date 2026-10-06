use super::*;
use crate::headless::claude_stream::ClaudeStream;
use crate::headless::{FailureKind, SessionEvent, TurnOutcome, codex_stream};
use serde_json::json;

/// The trial's line (2026-10-01): Codex's `turn.failed` message is the API's JSON error.
const UNSUPPORTED: &str = r#"{"type":"error","status":400,"error":{"type":"invalid_request_error","message":"The 'gpt-6.1-sol' model is not supported when using Codex with a ChatGPT account."}}"#;

fn codex_failed(message: &str) -> (FailureKind, String) {
    let line = json!({"type": "turn.failed", "error": {"message": message}}).to_string();
    match codex_stream::parse_line(&line).as_slice() {
        [
            SessionEvent::ApiErrorText { .. },
            SessionEvent::TurnEnded {
                outcome: TurnOutcome::Failed { kind, error },
                ..
            },
        ] => (*kind, error.clone()),
        other => panic!("{other:?}"),
    }
}

/// Claude's failed API turn as the recordings shape it: the synthetic `assistant` line
/// with its `error` category, then the `result` with `api_error_status`.
fn claude_failed(category: &str, status: Option<u64>, text: &str) -> (FailureKind, String) {
    let mut stream = ClaudeStream::default();
    let assistant = json!({"type": "assistant", "parent_tool_use_id": null, "error": category,
        "message": {"model": "<synthetic>", "content": [{"type": "text", "text": text}]}});
    stream.parse_line(&assistant.to_string());
    let result = json!({"type": "result", "subtype": "success", "is_error": true,
        "result": text, "terminal_reason": "api_error", "api_error_status": status});
    match stream.parse_line(&result.to_string()).as_slice() {
        [
            SessionEvent::TurnEnded {
                outcome: TurnOutcome::Failed { kind, error },
                ..
            },
        ] => (*kind, error.clone()),
        other => panic!("{other:?}"),
    }
}

#[test]
fn the_statuses_and_types_of_a_client_error() {
    for status in CLIENT_ERROR_STATUSES {
        assert!(is_client_error(Some(status), "x"), "{status}");
    }
    for status in [429, 500, 529, 408, 200] {
        assert!(!is_client_error(Some(status), "x"), "{status}");
    }
    for kind in CLIENT_ERROR_TYPES {
        let text = json!({"type": "error", "error": {"type": kind, "message": "m"}}).to_string();
        assert!(is_client_error(None, &text), "{kind}");
        assert!(
            is_client_error(None, &format!("API Error: {text}")),
            "{kind}"
        );
    }
    // A JSON `status` in the text counts; so does `API Error: <status>`.
    assert!(is_client_error(
        None,
        r#"{"status":404,"error":{"type":"x"}}"#
    ));
    assert!(is_client_error(None, "API Error: 422 unprocessable"));
    for text in [
        "overloaded",
        "stream disconnected",
        r#"{"status":529,"error":{"type":"overloaded_error"}}"#,
        "API Error: 500 internal",
        "the request id ends in 400",
        "my_invalid_request_error_count",
    ] {
        assert!(!is_client_error(None, text), "{text}");
    }
}

#[test]
fn codex_classifies_the_trials_unsupported_model_as_a_client_error() {
    assert_eq!(
        codex_failed(UNSUPPORTED),
        (
            FailureKind::ClientError,
            "The 'gpt-6.1-sol' model is not supported when using Codex with a ChatGPT account."
                .into()
        )
    );
    let not_found = r#"{"type":"error","status":404,"error":{"type":"not_found_error","message":"no such model"}}"#;
    assert_eq!(codex_failed(not_found).0, FailureKind::ClientError);
    // A rate limit and a server error keep their rules.
    let limited = r#"{"type":"error","status":429,"error":{"type":"invalid_request_error","message":"slow"}}"#;
    assert_eq!(codex_failed(limited).0, FailureKind::RateLimit);
    let server = r#"{"type":"error","status":500,"error":{"type":"api_error","message":"boom"}}"#;
    assert_eq!(codex_failed(server).0, FailureKind::Other);
    assert_eq!(codex_failed("stream disconnected").0, FailureKind::Other);
}

#[test]
fn claude_classifies_a_client_error_by_status_or_type() {
    let text = "API Error: model_not_found";
    assert_eq!(
        claude_failed("model_not_found", Some(404), text),
        (FailureKind::ClientError, text.into())
    );
    assert_eq!(
        claude_failed("invalid_request", Some(400), "API Error: invalid_request").0,
        FailureKind::ClientError
    );
    let typed = r#"API Error: {"type":"error","error":{"type":"permission_error","message":"no"}}"#;
    assert_eq!(
        claude_failed("unknown", None, typed).0,
        FailureKind::ClientError
    );
    // The categories decision 32 names keep their kinds, though billing is a 400 and
    // authentication a 401.
    assert_eq!(
        claude_failed("billing_error", Some(400), "Credit balance is too low").0,
        FailureKind::Billing
    );
    assert_eq!(
        claude_failed("authentication_failed", Some(401), "Not logged in").0,
        FailureKind::Authentication
    );
    assert_eq!(
        claude_failed("rate_limit", Some(429), "session limit").0,
        FailureKind::RateLimit
    );
    // A server error, an overload and an unknown failure stay `Other`.
    assert_eq!(
        claude_failed("overloaded", Some(529), "API Error: overloaded").0,
        FailureKind::Other
    );
    assert_eq!(
        claude_failed("server_error", Some(500), "API Error: 500").0,
        FailureKind::Other
    );
    assert_eq!(
        claude_failed("unknown", None, "API Error: unknown").0,
        FailureKind::Other
    );
}

/// 2026-10-06, v0.1.0 on Ubuntu 26.04: Claude refused to start without bubblewrap and
/// socat, and the user saw only "the scout ended two turns without a report".
const NO_BWRAP: &str = "Error: sandbox required but unavailable: sandbox is enabled but dependencies are missing: bubblewrap (bwrap) not installed, socat not installed. sandbox.failIfUnavailable is set — refusing to start without a working sandbox.";

#[test]
fn a_startup_failure_names_the_runtime_the_exit_and_the_last_stderr_lines() {
    let lines = ["loading".to_string(), "Error: no config".to_string()];
    assert_eq!(
        startup_failure(proto::Runtime::Codex, Some(2), None, &lines),
        "codex exited at startup (code 2): loading | no config"
    );
    assert_eq!(
        startup_failure(proto::Runtime::Claude, None, Some(9), &[]),
        "claude exited at startup (signal 9)"
    );
}

#[test]
fn a_sandbox_startup_failure_says_what_to_install() {
    let text = startup_failure(proto::Runtime::Claude, Some(1), None, &[NO_BWRAP.into()]);
    assert!(
        text.starts_with(
            "claude exited at startup (code 1): sandbox required but unavailable: sandbox is enabled but dependencies are missing: bubblewrap (bwrap) not installed, socat not installed"
        ),
        "{text}"
    );
    assert!(text.ends_with(SANDBOX_HINT), "{text}");
    // The hint is added once, however often the text passes through.
    assert_eq!(with_sandbox_hint(&text), text);
    assert_eq!(with_sandbox_hint("no sandbox here"), "no sandbox here");
}

#[test]
fn a_startup_failure_is_bounded() {
    let long = vec!["é".repeat(5000); 3];
    let text = startup_failure(proto::Runtime::Claude, Some(1), None, &long);
    assert!(text.len() <= STARTUP_FAILURE_MAX, "{}", text.len());
    assert!(text.ends_with('…'), "{text}");
}

/// 2026-10-06, Ubuntu 26.04 with bubblewrap and socat installed: AppArmor's
/// `bwrap-userns-restrict` confines bwrap, so every sandboxed command fails like this.
const NESTED_USERNS: &str = "apply-seccomp: write /proc/self/setgroups (nested userns is capability-restricted; caller must provide CAP_SYS_ADMIN): Permission denied";

#[test]
fn a_command_the_sandbox_could_not_start_is_named_with_the_docs() {
    let output = format!("bash: line 1\n{NESTED_USERNS}\n");
    let note = sandbox_command_failure(&output).expect("a sandbox failure");
    assert_eq!(
        note,
        format!("Claude's sandbox could not run a command: {NESTED_USERNS}{USERNS_HINT}")
    );
    assert!(USERNS_HINT.contains("https://code.claude.com/docs/en/sandboxing"));
    assert!(sandbox_command_failure("cargo: command not found").is_none());
    let long = format!("apply-seccomp: {}", "x".repeat(5000));
    assert!(sandbox_command_failure(&long).unwrap().len() <= STARTUP_FAILURE_MAX);
}
