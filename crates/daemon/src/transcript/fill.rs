//! Milestone 9.16 decisions 5, 22 and 24: how full a session's context is, and Codex's quota,
//! read from the tail of the session's own transcript or rollout. Pure: the caller reads the
//! tail (`TRANSCRIPT_TAIL_BYTES`) off every lock and passes the text.
//!
//! A tail usually starts inside a line; that first line, and any other line that is not a
//! JSON object, is skipped. A line in a shape these readers do not know gives no reading
//! (fail open, decision 28): never a wrong number.

use serde_json::{Map, Value};

use crate::headless::quota::{QuotaSample, codex_rate_limits};

/// A session's context fill.
#[derive(Debug, Clone, PartialEq)]
pub struct Fill {
    pub tokens: u64,
    /// Codex: the rollout's `model_context_window`. Claude: `None`; decision 23 resolves it.
    pub window: Option<u64>,
    /// Claude: the line's `message.model`, as written (it carries no `[1m]` tag even in a 1M
    /// session; M9.16.1 finding 2). Codex: `None`.
    pub model: Option<String>,
}

/// Claude: the last main-chain assistant line's `input_tokens + cache_read_input_tokens +
/// cache_creation_input_tokens + output_tokens` (decision 22). A sub-agent's line
/// (`isSidechain: true`) is skipped; a last main-chain line without all four is no reading.
pub fn claude_fill(tail: &str) -> Option<Fill> {
    let line = objects(tail).rev().find(|line| {
        line.get("type").and_then(Value::as_str) == Some("assistant")
            && line.get("isSidechain").and_then(Value::as_bool) != Some(true)
    })?;
    let message = line.get("message")?;
    let usage = message.get("usage")?;
    let mut tokens = 0u64;
    for field in [
        "input_tokens",
        "cache_read_input_tokens",
        "cache_creation_input_tokens",
        "output_tokens",
    ] {
        tokens = tokens.checked_add(usage.get(field)?.as_u64()?)?;
    }
    Some(Fill {
        tokens,
        window: None,
        model: message
            .get("model")
            .and_then(Value::as_str)
            .map(str::to_owned),
    })
}

/// Codex: the last `token_count` that carries usage; `info.last_token_usage.input_tokens +
/// output_tokens` (cached tokens are inside the input), and `info.model_context_window` as
/// the window, read from the line, since it differs by CLI version (decision 22).
pub fn codex_fill(tail: &str) -> Option<Fill> {
    let info = token_counts(tail).find_map(|mut payload| match payload.get_mut("info") {
        Some(Value::Object(info)) => Some(std::mem::take(info)),
        _ => None,
    })?;
    let last = info.get("last_token_usage")?;
    let tokens = last
        .get("input_tokens")?
        .as_u64()?
        .checked_add(last.get("output_tokens")?.as_u64()?)?;
    Some(Fill {
        tokens,
        window: info
            .get("model_context_window")
            .and_then(Value::as_u64)
            .filter(|w| *w > 0),
        model: None,
    })
}

/// Codex: the last `token_count` that carries `rate_limits`, as a quota reading (decision 5).
pub fn codex_limits(tail: &str) -> Option<QuotaSample> {
    let limits = token_counts(tail).find_map(|mut payload| {
        payload
            .get_mut("rate_limits")
            .map(Value::take)
            .filter(|l| !l.is_null())
    })?;
    codex_rate_limits(&limits)
}

/// Decision 24: whichever criterion comes first; each `0` disables its own, and a known
/// token count with no known window still hands off at the cap.
pub fn handoff_due(tokens: u64, window: Option<u64>, percent: u8, cap: u64) -> bool {
    (cap > 0 && tokens >= cap)
        || (percent > 0
            && window
                .is_some_and(|w| u128::from(tokens) * 100 >= u128::from(w) * u128::from(percent)))
}

/// `tokens * 100 / window`, floored; `0` for a zero window.
pub fn fill_percent(tokens: u64, window: u64) -> u64 {
    (u128::from(tokens) * 100)
        .checked_div(u128::from(window))
        .map_or(0, |p| u64::try_from(p).unwrap_or(u64::MAX))
}

/// The tail's lines that are JSON objects, in order.
fn objects(tail: &str) -> impl DoubleEndedIterator<Item = Map<String, Value>> + '_ {
    tail.lines()
        .filter_map(|line| match serde_json::from_str(line) {
            Ok(Value::Object(map)) => Some(map),
            _ => None,
        })
}

/// Codex `event_msg` payloads of type `token_count`, newest first.
fn token_counts(tail: &str) -> impl Iterator<Item = Value> + '_ {
    objects(tail).rev().filter_map(|mut line| {
        let payload = line.remove("payload")?;
        (line.get("type").and_then(Value::as_str) == Some("event_msg")
            && payload.get("type").and_then(Value::as_str) == Some("token_count"))
        .then_some(payload)
    })
}

#[cfg(test)]
#[path = "fill_tests.rs"]
mod tests;
