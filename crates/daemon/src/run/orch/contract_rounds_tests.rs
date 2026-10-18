//! Milestone 9.16 decision 25: the handoff prompt's last line and the handoff's texts.

use super::{
    FILL_UNKNOWN_TEXT, fill_attention_text, handed_off_name, handed_off_text, handoff_prompt,
};
use crate::run::model::HandoffRecord;

fn record(window_tokens: Option<u64>) -> HandoffRecord {
    HandoffRecord {
        from_run: "3f9a".into(),
        from_window: 7,
        fill_tokens: 170_000,
        window_tokens,
        at: 1_790_000_000,
    }
}

#[test]
fn handoff_prompt_names_the_fill() {
    let ended = ("o-3f9a", "3f9a", "accepted");
    let plain = handoff_prompt("first", ended, Some("added login"), None, None);
    let known = handoff_prompt(
        "first",
        ended,
        Some("added login"),
        None,
        Some(&record(Some(200_000))),
    );
    assert_eq!(
        known,
        format!(
            "{plain}The previous session ended with context 85% full (170k / 200k); this one starts fresh."
        )
    );
    let unknown = handoff_prompt(
        "first",
        ended,
        Some("added login"),
        None,
        Some(&record(None)),
    );
    assert_eq!(
        unknown,
        format!(
            "{plain}The previous session ended with context 170k (window unknown); this one starts fresh."
        )
    );
    assert!(!plain.contains("starts fresh"), "{plain}");
}

#[test]
fn the_handoffs_texts() {
    assert_eq!(handed_off_name("3f9a"), "3f9a/orchestrator (handed off)");
    assert_eq!(
        handed_off_text(&record(Some(200_000))),
        "handed off from 3f9a: context 85% full (170k / 200k)"
    );
    assert_eq!(
        handed_off_text(&record(None)),
        "handed off from 3f9a: context 170k (window unknown)"
    );
    assert_eq!(
        fill_attention_text(184_000, 200_000),
        "orchestrator context 92% full (184k / 200k)"
    );
    assert_eq!(
        FILL_UNKNOWN_TEXT,
        "context fill unknown; continuing in the same session"
    );
}

/// Milestone 9.19 decision 10: a session past its window reads `over 100%`, never
/// `450%`, in the handoff prompt, the log line and the attention line; a zero window
/// is an unknown one.
#[test]
fn a_million_token_session_is_over_not_450_percent() {
    let over = |window| HandoffRecord {
        fill_tokens: 900_000,
        window_tokens: window,
        ..record(None)
    };
    let ended = ("o-3f9a", "3f9a", "accepted");
    let prompt = handoff_prompt("first", ended, None, None, Some(&over(Some(200_000))));
    assert!(
        prompt.ends_with(
            "The previous session ended with context over 100% (900k / 200k window); this one starts fresh."
        ),
        "{prompt}"
    );
    assert_eq!(
        handed_off_text(&over(Some(200_000))),
        "handed off from 3f9a: context over 100% (900k / 200k window)"
    );
    assert_eq!(
        fill_attention_text(900_000, 200_000),
        "orchestrator context over 100% (900k / 200k window)"
    );
    assert_eq!(
        handed_off_text(&over(Some(0))),
        "handed off from 3f9a: context 900k (window unknown)"
    );
    let prompt = handoff_prompt("first", ended, None, None, Some(&over(Some(0))));
    assert!(
        prompt.ends_with(
            "The previous session ended with context 900k (window unknown); this one starts fresh."
        ),
        "{prompt}"
    );
}
