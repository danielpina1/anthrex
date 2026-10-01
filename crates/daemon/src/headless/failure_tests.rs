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
