//! Ruling F-1 (amending M8a decision 32): which failed turns are deterministic client
//! errors. Each runtime's `classify` asks here after its own rate-limit, authentication
//! and billing rules, so the engine never matches error text itself.

use serde_json::Value;

/// The HTTP statuses of a client error a continue cannot fix. 429 is not one: it is a
/// rate limit.
pub const CLIENT_ERROR_STATUSES: [u64; 5] = [400, 401, 403, 404, 422];

/// The API error types of a client error a continue cannot fix.
pub const CLIENT_ERROR_TYPES: [&str; 3] = [
    "invalid_request_error",
    "not_found_error",
    "permission_error",
];

/// Whether a failure with `status` (when the runtime reports one) and `text` is a
/// deterministic client error: a client status, given or found in the text (a JSON
/// error's `status`, or `API Error: <status>`), or a client error type, as a JSON
/// error's `error.type` or a word of its own in the text.
pub fn is_client_error(status: Option<u64>, text: &str) -> bool {
    let json = text
        .find('{')
        .and_then(|at| serde_json::from_str::<Value>(&text[at..]).ok());
    let json_status = json
        .as_ref()
        .and_then(|v| v.get("status"))
        .and_then(Value::as_u64);
    let api_status = text
        .trim_start()
        .strip_prefix("API Error: ")
        .and_then(|rest| {
            rest.get(..3)
                .filter(|_| !rest[3..].starts_with(|c: char| c.is_ascii_alphanumeric()))
        })
        .and_then(|digits| digits.parse::<u64>().ok());
    let client_status = [status, json_status, api_status]
        .into_iter()
        .flatten()
        .any(|s| CLIENT_ERROR_STATUSES.contains(&s));
    let json_type = json
        .as_ref()
        .and_then(|v| v.get("error"))
        .and_then(|e| e.get("type"))
        .and_then(Value::as_str);
    client_status
        || json_type.is_some_and(|t| CLIENT_ERROR_TYPES.contains(&t))
        || CLIENT_ERROR_TYPES.iter().any(|t| has_word(text, t))
}

/// Whether `word` occurs in `text` with no ASCII letter, digit or `_` on either side.
fn has_word(text: &str, word: &str) -> bool {
    let bytes = text.as_bytes();
    let part = |b: u8| b.is_ascii_alphanumeric() || b == b'_';
    text.match_indices(word).any(|(at, _)| {
        let before = at.checked_sub(1).map(|i| bytes[i]);
        let after = bytes.get(at + word.len()).copied();
        !before.is_some_and(part) && !after.is_some_and(part)
    })
}

#[cfg(test)]
#[path = "failure_tests.rs"]
mod tests;
