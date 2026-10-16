//! Milestone 9.16: what a Claude `result` line's `modelUsage` adds to the stream (decision
//! 23). The `rate_limit_event` side is `quota_tests.rs`.

use super::*;
use crate::headless::SessionEvent;
use crate::headless::test_support::{CLAUDE_STREAM, line_type, lines};
use serde_json::json;

/// Milestone 9.16 decision 23: a `result` line's `modelUsage` gives one `ContextWindow` per
/// model, keyed as written, after the line's `TurnEnded` (and after its structured output).
#[test]
fn result_model_usage_yields_context_windows_after_turn_end() {
    const DECIDER: &str =
        include_str!("../../tests/fixtures/deciders/claude-2.1.280-decider.jsonl");
    const CAPTURE_280: &str =
        include_str!("../../tests/fixtures/headless/claude-2.1.280-rate-limit.jsonl");
    let result = |fixture: &'static str| {
        lines(fixture)
            .into_iter()
            .find(|line| line_type(line) == "result")
            .unwrap()
    };
    // The decider's key carries no date (2.1.280); the 2.1.278 stream's does.
    let events = ClaudeStream::default().parse_line(result(DECIDER));
    let [
        SessionEvent::StructuredOutput { .. },
        SessionEvent::TurnEnded { .. },
        SessionEvent::ContextWindow { model, tokens },
    ] = events.as_slice()
    else {
        panic!("{events:#?}");
    };
    assert_eq!((model.as_str(), *tokens), ("claude-haiku-4-5", 200_000));
    let events = ClaudeStream::default().parse_line(result(CLAUDE_STREAM));
    assert!(
        matches!(events.as_slice(), [
            SessionEvent::TurnEnded { .. },
            SessionEvent::ContextWindow { model, tokens: 200_000 },
        ] if model == "claude-haiku-4-5-20251001"),
        "{events:#?}"
    );
    // The 1M capture: the key keeps its `[1m]` tag, as written.
    let events = ClaudeStream::default().parse_line(result(CAPTURE_280));
    assert_eq!(
        events.last(),
        Some(&SessionEvent::ContextWindow {
            model: "claude-opus-5-5[1m]".into(),
            tokens: 1_000_000,
        })
    );

    // One per model with a readable window, in key order; a model without one, or with a
    // zero, a string or a negative, gives none.
    let line = json!({
        "type": "result", "subtype": "success", "is_error": false,
        "modelUsage": {
            "claude-opus-5-5": {"contextWindow": 200_000},
            "claude-haiku-4-5": {"contextWindow": 200_000},
            "a": {"contextWindow": 0},
            "b": {"contextWindow": "200000"},
            "c": {"contextWindow": -1},
            "d": {},
        }
    })
    .to_string();
    let windows: Vec<_> = ClaudeStream::default()
        .parse_line(&line)
        .into_iter()
        .skip(1)
        .collect();
    assert_eq!(
        windows,
        [
            SessionEvent::ContextWindow {
                model: "claude-haiku-4-5".into(),
                tokens: 200_000
            },
            SessionEvent::ContextWindow {
                model: "claude-opus-5-5".into(),
                tokens: 200_000
            },
        ]
    );
    for usage in [json!({}), json!(null), json!("x")] {
        let line = json!({"type": "result", "subtype": "success", "modelUsage": usage}).to_string();
        assert!(
            matches!(
                ClaudeStream::default().parse_line(&line).as_slice(),
                [SessionEvent::TurnEnded { .. }]
            ),
            "{line}"
        );
    }
}
