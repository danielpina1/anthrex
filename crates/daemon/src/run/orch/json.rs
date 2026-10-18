//! Helpers the three read tools share (decisions 16–18): cutting untrusted text,
//! measuring a JSON answer, and the last-resort shrink that keeps an answer under its
//! cap whatever the run holds. Pure.
//!
//! Every text an agent or the repository wrote (worker notes, block reasons, scout
//! summaries, review text) reaches a model only inside a JSON string, which
//! `serde_json` escapes: a newline is `\n`, never a line break, so no such text can
//! open an anthrex line or section. [`cut`] also folds the three line breaks JSON
//! leaves raw (U+0085, U+2028, U+2029) into spaces.

use serde_json::Value;

/// `text` cut to `max` characters, `…` appended when it was longer, with every raw
/// line break JSON does not escape turned into a space.
pub(crate) fn cut(text: &str, max: usize) -> String {
    let mut out: String = text.chars().take(max).map(fold).collect();
    if text.chars().nth(max).is_some() {
        out.push('…');
    }
    out
}

/// [`cut`] of an optional text.
pub(crate) fn cut_opt(text: Option<&str>, max: usize) -> Option<String> {
    text.map(|t| cut(t, max))
}

fn fold(c: char) -> char {
    match c {
        '\u{85}' | '\u{2028}' | '\u{2029}' => ' ',
        c => c,
    }
}

/// Every string in `value` with the raw line breaks JSON leaves unescaped folded into
/// spaces: applied to a whole answer, so no field is missed.
pub(crate) fn fold_all(value: &mut Value) {
    match value {
        Value::String(s) => {
            if s.contains(['\u{85}', '\u{2028}', '\u{2029}']) {
                *s = s.chars().map(fold).collect();
            }
        }
        Value::Array(items) => items.iter_mut().for_each(fold_all),
        Value::Object(map) => map.values_mut().for_each(fold_all),
        _ => {}
    }
}

/// The compact JSON size of `value`, in bytes: what the tool result's text holds.
pub(crate) fn size(value: &Value) -> usize {
    serde_json::to_vec(value).map_or(usize::MAX, |bytes| bytes.len())
}

/// Every string in `value` (object keys excepted) cut to `max` characters: the last
/// step of a trim, once the documented ones have not brought an answer under its cap.
pub(crate) fn shrink_strings(value: &mut Value, max: usize) {
    match value {
        Value::String(s) => {
            if s.chars().nth(max).is_some() {
                *s = cut(s, max);
            }
        }
        Value::Array(items) => items.iter_mut().for_each(|v| shrink_strings(v, max)),
        Value::Object(map) => map.values_mut().for_each(|v| shrink_strings(v, max)),
        _ => {}
    }
}

/// A stored time in the orchestrator's JSON (milestone 9.19 decision 3, ruling R7):
/// `2026-10-12 14:30 UTC`, the long form, labelled, the same whatever `now` is, so a
/// digest's fingerprint never moves with the clock.
pub(crate) fn at_text(at: u64) -> String {
    common::numbers::clock_full(at, common::numbers::Zone::Utc)
}

/// `line` without the [`at_text`] it starts with (its three words); empty when it has
/// none.
pub(crate) fn after_at(line: &str) -> &str {
    line.splitn(4, ' ').nth(3).unwrap_or("")
}

/// FNV-1a, 64 bits (decision 16's fingerprint).
pub(crate) fn fnv1a(bytes: &[u8]) -> u64 {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in bytes {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x0100_0000_01b3);
    }
    hash
}

/// `<runtime> <model or (default)> <effort>`, as `run status` writes a route.
pub(crate) fn route_text(route: &proto::Route) -> String {
    let model = if route.model.is_empty() {
        "(default)"
    } else {
        route.model.as_str()
    };
    let effort = route.effort.to_string();
    format!("{} {model} {effort}", route.runtime.label())
}

/// A serde enum's JSON label (`blocked`, `message_pause`, …); empty if it has none.
pub(crate) fn label<T: serde::Serialize>(value: &T) -> String {
    match serde_json::to_value(value) {
        Ok(Value::String(s)) => s,
        _ => String::new(),
    }
}

#[cfg(test)]
mod tests {
    /// Milestone 9.19 decision 3: a stored time is the long UTC form, whatever `now`,
    /// and a line that starts with one gives back its text.
    #[test]
    fn a_stored_time_is_long_and_labelled() {
        // 2026-10-12 14:07:59 UTC.
        assert_eq!(super::at_text(1_791_814_079), "2026-10-12 14:07 UTC");
        let line = format!("{} blocked (question): which one?", super::at_text(0));
        assert_eq!(super::after_at(&line), "blocked (question): which one?");
        assert_eq!(super::after_at("14:07 too short"), "");
    }
}
