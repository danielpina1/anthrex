//! The agent conversation model: what a window's (or sub-agent's) transcript looks like
//! once the daemon has turned hooks and, where available, the runtime's own transcript
//! file into a structured timeline. See `docs/milestones/M6.5-conversation-view.md`.

use crate::types::Runtime;
use serde::{Deserialize, Serialize};

/// One agent's conversation: either a window's own agent, or one of its sub-agents.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Conversation {
    pub window_id: u32,
    /// `None` is the window's own agent; `Some(id)` is one of its sub-agents
    /// (decision A1). The id is `SubagentInfo.id` from milestone 3.
    pub agent_id: Option<String>,
    pub session_id: Option<String>,
    pub runtime: Runtime,
    pub rev: u64,
    pub degraded: Option<DegradeReason>,
    pub dropped_turns: u32,
    pub dropped_by: Option<DropCause>,
    pub turns: Vec<Turn>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Turn {
    pub id: u64,
    pub role: Role,
    /// Absolute Unix seconds (decision A2), not `SystemTime`. `SystemTime` *is*
    /// `Serialize`, in a fixed shape — but that shape is a two-field struct
    /// (`{"secs_since_epoch": .., "nanos_since_epoch": ..}`), not the single `u64` the
    /// protocol wants, and it carries nanosecond precision this model has no use for.
    pub at_unix_secs: u64,
    pub state: TurnState,
    pub blocks: Vec<Block>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    User,
    Assistant,
    System,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TurnState {
    Running,
    Complete,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ToolState {
    Pending,
    Ok,
    Failed,
    Denied,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NoticeKind {
    PermissionRequest,
    Error,
    Compaction,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DropCause {
    Turns,
    Bytes,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DegradeReason {
    /// The runtime's hooks never reported a `transcript_path`.
    NoTranscriptPath,
    /// The path is absent, is a directory, or could not be opened or read.
    Unreadable,
    /// `detect` did not recognise the first line.
    UnknownFormat,
    /// The file is larger than `TRANSCRIPT_MAX_BYTES`.
    TooLarge,
    /// At least one line was valid JSON of a shape the parser does not model, or
    /// exceeded `TRANSCRIPT_LINE_MAX`.
    BadRecord,
    /// A transcript prompt did not match the hook-built prompt its ordinal maps to, so
    /// positional enrichment stopped there (task M6.5.8). Prose before that prompt is
    /// kept; tool detail still joins by tool-use id.
    Misaligned,
}

impl DegradeReason {
    /// The footer text, verbatim. Every one ends in "— timeline only" so the user is
    /// told what still works, not only what does not.
    pub fn message(self) -> &'static str {
        match self {
            DegradeReason::NoTranscriptPath => {
                "no transcript path from this runtime — timeline only"
            }
            DegradeReason::Unreadable => "transcript unreadable — timeline only",
            DegradeReason::UnknownFormat => "transcript format not recognised — timeline only",
            DegradeReason::TooLarge => "transcript too large to read — timeline only",
            DegradeReason::BadRecord => "transcript partly unreadable — timeline only",
            DegradeReason::Misaligned => {
                "the transcript's prompts do not line up with the hook timeline — timeline only"
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Block {
    Text {
        text: String,
    },
    ToolCall {
        /// The runtime's tool-use id when it gave one; `None` otherwise.
        id: Option<String>,
        name: String,
        /// Always present, always derived from the hook (spec decision 5).
        summary: String,
        input: Option<serde_json::Value>,
        result: Option<ToolResult>,
        state: ToolState,
        /// Decision A2: wall-clock `PreToolUse` → `PostToolUse`, `None` while Pending.
        duration_ms: Option<u32>,
    },
    SubagentSpawn {
        agent_id: String,
        kind: String,
        label: String,
        model: Option<String>,
    },
    Notice {
        kind: NoticeKind,
        text: String,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ToolResult {
    pub ok: bool,
    pub summary: String,
    pub detail: Option<String>,
    pub truncated: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TurnPatch {
    Upsert(Turn),
    Drop { id: u64 },
}

/// Fixed per-turn and per-block accounting overhead for `max_bytes` (decision 7), so the
/// cap counts structure as well as content and a conversation of ten thousand empty turns
/// is still bounded.
///
/// Amendment to task M6.5.1 (review finding F3): `byte_size` feeds a cap, so it must be a
/// conservative *over*-estimate of the encoded size, never an under-estimate — an
/// over-estimate only makes the cap bite a little early, while an under-estimate makes it
/// not bite at all, which is unbounded. The original 64/32 were typed independently of
/// what they stand for and were measured to be 23% under: an empty `ToolCall` turn
/// (`rmp_serde::to_vec_named`) encodes to 125 bytes against a reported `byte_size()` of
/// 96. These values are derived from the worst case they have to cover — the `ToolCall`
/// variant with a populated `ToolResult`, every optional field present, and every string
/// field long enough to need the largest MessagePack string-length header (5 bytes) —
/// measured at 73 bytes of pure turn-level structure (`id`/`role`/`at_unix_secs`/`state`/
/// the blocks array, each field at its own worst case) and 125 bytes of pure per-block
/// structure on top, with roughly 10% headroom added to each. `byte_size_never_
/// underestimates_its_encoded_size` in `conversation_tests.rs` pins the *direction* of
/// the error — the actual guarantee — across every `Block` variant, not these two numbers.
pub const TURN_OVERHEAD: usize = 80;
pub const BLOCK_OVERHEAD: usize = 144;

/// `ConversationGone.reason` values (decision-pinned wire strings; see `messages.rs`).
pub const GONE_WINDOW_REMOVED: &str = "window removed";
pub const GONE_SUBAGENT_UNKNOWN: &str = "no such sub-agent in this window";
pub const GONE_WINDOW_UNKNOWN: &str = "no such window";
/// The daemon could not fit this conversation, or the change to it, in one frame
/// (`codec::MAX_FRAME`), and has ended the subscription rather than the connection.
pub const GONE_TOO_LARGE: &str = "conversation too large to send";

/// An upper bound on a `serde_json::Value`'s MessagePack encoding, which is what a
/// frame carries: every number as 9 bytes (a marker and an 8-byte float or integer),
/// `null` and booleans as 1, and every string, array and map as its content plus a 5-byte
/// header, the largest MessagePack uses. Task M6.5.10's review (F1) measured the compact
/// JSON length this replaced at 2.25 times *under* the encoding for a float array (`0.5,`
/// is 4 JSON bytes and 9 MessagePack bytes), and every cap built on `byte_size` assumes
/// it over-estimates.
pub fn value_byte_size(value: &serde_json::Value) -> usize {
    const HEADER: usize = 5;
    match value {
        serde_json::Value::Null | serde_json::Value::Bool(_) => 1,
        serde_json::Value::Number(_) => 9,
        serde_json::Value::String(s) => HEADER + s.len(),
        serde_json::Value::Array(items) => {
            HEADER + items.iter().map(value_byte_size).sum::<usize>()
        }
        serde_json::Value::Object(map) => {
            HEADER
                + map
                    .iter()
                    .map(|(key, item)| HEADER + key.len() + value_byte_size(item))
                    .sum::<usize>()
        }
    }
}

impl Block {
    /// One block's contribution to its turn's `byte_size`, including `BLOCK_OVERHEAD`.
    /// `pub` because a later task needs a per-block measure — for `max_result_bytes`
    /// enforcement and single-block trimming — and duplicating this formula there would
    /// be worse than exposing it (review finding F7). `Turn::byte_size` is still the one
    /// definition every *test* asserts through.
    pub fn byte_size(&self) -> usize {
        let content = match self {
            Block::Text { text } => text.len(),
            Block::ToolCall {
                id,
                name,
                summary,
                input,
                result,
                state: _,
                duration_ms: _,
            } => {
                // `id` is `tool_use_id` from the runtime's own hook payload (amendment
                // to task M6.5.1, review finding F2): its length is set by the runtime,
                // not by us, so a cap that ignored it would be an under-estimate — and
                // `byte_size` feeds a cap, where an under-estimate is unbounded while an
                // over-estimate merely bites a little early. It counts like every other
                // field here.
                id.as_ref().map(|s| s.len()).unwrap_or(0)
                    + name.len()
                    + summary.len()
                    + input.as_ref().map(value_byte_size).unwrap_or(0)
                    + result
                        .as_ref()
                        .map(|r| r.summary.len() + r.detail.as_ref().map(|d| d.len()).unwrap_or(0))
                        .unwrap_or(0)
            }
            Block::SubagentSpawn {
                agent_id,
                kind,
                label,
                model,
            } => {
                agent_id.len()
                    + kind.len()
                    + label.len()
                    + model.as_ref().map(|m| m.len()).unwrap_or(0)
            }
            Block::Notice { kind: _, text } => text.len(),
        };
        BLOCK_OVERHEAD + content
    }
}

impl Turn {
    /// The one definition of a turn's size, used by the daemon's cap and by its tests.
    /// `input` is measured as its compact JSON encoding.
    pub fn byte_size(&self) -> usize {
        TURN_OVERHEAD + self.blocks.iter().map(Block::byte_size).sum::<usize>()
    }
}

impl Conversation {
    /// Sum of `Turn::byte_size`.
    pub fn byte_size(&self) -> usize {
        self.turns.iter().map(Turn::byte_size).sum()
    }
}

// ---------------------------------------------------------------------------------------
// Tool-result bounding, moved unchanged from `crates/cli/src/hook.rs` by M8a.7 (ruling Q5)
// so the daemon can bound a headless session's synthesised `tool_response` exactly as
// `anthrex hook` bounds a real one. `anthrex hook` calls these from here.
// ---------------------------------------------------------------------------------------

/// Spec decision 5a: the bound on what one hook may carry back about a tool's result.
/// Well under `HOOK_PAYLOAD_MAX` (8 MiB, in `crates/cli/src/hook.rs`), which still runs first on the raw stdin
/// bytes and is untouched by this — an over-8-MiB payload is still dropped whole.
///
/// Also `<= config::CONVERSATION_MAX_RESULT_BYTES_MIN`, the smallest legally configured
/// `conversation.max_result_bytes` (amendment, review finding F3: the original comment
/// claimed this against only the *default* `max_result_bytes` (16 KiB), which is false at
/// the low end of the range that existed at the time — `max_result_bytes = 2048` was legal
/// and a 4096-byte hook result would trip it). Task M6.5.3 raised `[conversation]`'s range
/// floor to match this constant and `crates/cli/src/hook.rs` asserts the relationship at compile time so the
/// two can never drift apart silently again.
pub const TOOL_RESULT_SUMMARY_MAX: usize = 4 * 1024;

/// Spec decision 5a: bound the top-level `tool_response`, in place, rather than strip it.
/// Amended by wave-1 review finding F2, then amended again by wave 2: an over-limit
/// *object* never collapses into a plain string at all, in any circumstance. M6.5.5's `ok`
/// predicate reads an object's `error`/`success` keys to tell a failed tool from a succeeded
/// one, and `ToolResult.ok` is a plain `bool` with no honest third "unknown" state — so a
/// choice between "sometimes read a failure as a success" and "sometimes read a success as a
/// failure" is a defect either way, not a tradeoff to make. The wave-2 fix instead
/// guarantees the object always keeps its shape: `bound_object` builds the bounded object up
/// from nothing (`error` first, then `success`, then whatever else fits), rather than
/// shrinking the original down and falling back to text when shrinking runs out of leaves to
/// cut. `tool_result_stringified` still exists, but only ever becomes `true` for a type with
/// no `ok` predicate to protect (array, number, bool, null) — see `bound_tool_response_value`.
pub fn bound_tool_response(object: &mut serde_json::Map<String, serde_json::Value>) {
    let Some(value) = object.get("tool_response").cloned() else {
        return;
    };
    let (tool_response, truncated, stringified) = bound_tool_response_value(value);
    object.insert("tool_response".to_string(), tool_response);
    object.insert(
        "tool_result_truncated".to_string(),
        serde_json::Value::Bool(truncated),
    );
    object.insert(
        "tool_result_stringified".to_string(),
        serde_json::Value::Bool(stringified),
    );
}

/// Truncates one already-extracted `tool_response` value to `TOOL_RESULT_SUMMARY_MAX`,
/// returning the (possibly rewritten) value, whether it was truncated, and whether an
/// object was collapsed into a string in the process (see `bound_tool_response`).
///
/// A `Value::String` is truncated on its own raw UTF-8 content, not on
/// `serde_json::to_string(&value)`'s JSON-quoted form. The reason is semantic: the field
/// carries the result's *text*, so the text is what gets truncated — matching M6.5.5's
/// `summary` rule, which truncates the same field the same way. (An earlier version of
/// this comment argued from byte parity instead — a leading JSON quote shifts every
/// following multi-byte character off an even offset — but that argument is fixture-
/// specific: it happens to distinguish the two readings for a run of 2-byte characters
/// like `é`, and inverts for a run of 3-byte characters like `世`. The semantic argument
/// holds regardless of character width; the parity argument does not, and is not the
/// reason for this choice.)
pub fn bound_tool_response_value(value: serde_json::Value) -> (serde_json::Value, bool, bool) {
    match value {
        serde_json::Value::String(s) => {
            if s.len() <= TOOL_RESULT_SUMMARY_MAX {
                (serde_json::Value::String(s), false, false)
            } else {
                (
                    serde_json::Value::String(truncate_to_char_boundary(
                        &s,
                        TOOL_RESULT_SUMMARY_MAX,
                    )),
                    true,
                    false,
                )
            }
        }
        serde_json::Value::Object(map) => bound_object(map),
        // Array, Number, Bool, Null: none of these carry an `ok` predicate downstream (only
        // a `Value::Object`'s `error`/`success` keys do), so there is no structure to
        // protect and converting one to text loses nothing M6.5.5 reads. `stringified` is
        // `true` exactly when that conversion actually happened (wave-2 review ruling): it
        // is honest documentation of what occurred, not a signal anything needs to branch
        // on, since M6.5.5's documented default `ok == true` for a non-object
        // `tool_response` is already correct for these types whether or not they were
        // truncated.
        other => {
            let encoded = serde_json::to_string(&other).unwrap_or_default();
            if encoded.len() <= TOOL_RESULT_SUMMARY_MAX {
                (other, false, false)
            } else {
                (
                    serde_json::Value::String(truncate_to_char_boundary(
                        &encoded,
                        TOOL_RESULT_SUMMARY_MAX,
                    )),
                    true,
                    true,
                )
            }
        }
    }
}

/// The first `max` bytes of `s`, backed off to the nearest char boundary so the result is
/// always valid UTF-8 (slicing a `String` at a byte index inside a multi-byte character
/// panics).
pub fn truncate_to_char_boundary(s: &str, max: usize) -> String {
    let mut end = max.min(s.len());
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }
    s[..end].to_string()
}

/// An over-limit JSON object is rebuilt from nothing rather than shrunk down (wave-2 review
/// ruling, replacing wave 1's shrink-then-fall-back-to-text approach): `error` goes in
/// first, then `success`, each with its own value truncated if it has to be, so the two keys
/// M6.5.5's `ok` predicate reads always survive, non-null, in an object it can still call
/// `.is_object()` on. Every other key is then added, in whatever order `serde_json::Map`
/// iterates them (this workspace does not enable serde_json's `preserve_order` feature, so
/// that is sorted-by-key order, not the original payload's JSON-text order — "original
/// order" in the sense the brief means it no longer survives parsing by the time this
/// function sees the map), while the whole object's compact encoding still fits, truncating
/// a key's own string content the same way `error`/`success` are. The moment one key cannot
/// be made to fit at all, it and every key after it are dropped: the budget only shrinks as
/// keys are added, so no later key could have fit either.
///
/// The result is always a `Value::Object` — `tool_result_stringified` is always `false` for
/// this function's output; see `bound_tool_response_value` for the (unrelated) case where
/// that flag is `true`.
fn bound_object(
    map: serde_json::Map<String, serde_json::Value>,
) -> (serde_json::Value, bool, bool) {
    let encoded_len = |v: &serde_json::Value| {
        serde_json::to_string(v)
            .map(|s| s.len())
            .unwrap_or(usize::MAX)
    };
    if encoded_len(&serde_json::Value::Object(map.clone())) <= TOOL_RESULT_SUMMARY_MAX {
        return (serde_json::Value::Object(map), false, false);
    }

    let mut result = serde_json::Map::new();
    let mut truncated = false;

    // `error`/`success` are the two keys M6.5.5's `ok` predicate reads; they must survive
    // no matter what else has to give way.
    for key in ["error", "success"] {
        if let Some(value) = map.get(key) {
            insert_shrinking(&mut result, key, value.clone(), false, &mut truncated);
        }
    }

    for (key, value) in map.iter() {
        if key == "error" || key == "success" {
            continue;
        }
        if !insert_shrinking(&mut result, key, value.clone(), true, &mut truncated) {
            break;
        }
    }

    (serde_json::Value::Object(result), truncated, false)
}

/// Inserts `key: value` into `result`, shrinking `value`'s own content (its longest string
/// leaf, recursing into nested objects/arrays, same as wave 1's leaf search) as many times
/// as needed to bring the whole `result` back under `TOOL_RESULT_SUMMARY_MAX`. Returns
/// whether the key ended up present.
///
/// `allow_drop = false` (for `error`/`success`) never removes the key. If leaf-shrinking
/// alone cannot make it fit — no string leaf, or every leaf already empty — the value is
/// replaced *once* by a truncated JSON-text rendering of itself (still present, still
/// non-null), which the next pass through the loop then shrinks as an ordinary string leaf;
/// this cannot recurse a second time, because once the value is a `Value::String` the
/// `matches!` check below returns instead of re-rendering, which is what stops this from
/// oscillating forever between an emptied string and its own 2-byte re-quoted rendering (an
/// empty string is still non-null, so giving up at that point does not lose the signal
/// `error`/`success` exists to carry). `allow_drop = true` (every other key) removes the key
/// outright once it cannot be made to fit; the caller stops walking further keys at that
/// point (see `bound_object`).
fn insert_shrinking(
    result: &mut serde_json::Map<String, serde_json::Value>,
    key: &str,
    value: serde_json::Value,
    allow_drop: bool,
    truncated: &mut bool,
) -> bool {
    let encoded_len = |v: &serde_json::Value| {
        serde_json::to_string(v)
            .map(|s| s.len())
            .unwrap_or(usize::MAX)
    };
    result.insert(key.to_string(), value);
    loop {
        let current = encoded_len(&serde_json::Value::Object(result.clone()));
        if current <= TOOL_RESULT_SUMMARY_MAX {
            return true;
        }
        let overage = current - TOOL_RESULT_SUMMARY_MAX;
        let entry = result.get_mut(key).expect("just inserted");
        match largest_string_leaf(entry) {
            Some(leaf) if !leaf.is_empty() => {
                let target = leaf.len().saturating_sub(overage);
                let mut end = target.min(leaf.len());
                while end > 0 && !leaf.is_char_boundary(end) {
                    end -= 1;
                }
                leaf.truncate(end);
                *truncated = true;
            }
            _ if allow_drop => {
                result.remove(key);
                *truncated = true;
                return false;
            }
            _ => {
                if matches!(entry, serde_json::Value::String(_)) {
                    // Already a string, and already fully shrunk: nothing more can be
                    // done. Leave it (possibly empty, still non-null) and stop.
                    *truncated = true;
                    return true;
                }
                let rendered = serde_json::to_string(entry).unwrap_or_default();
                *entry = serde_json::Value::String(rendered);
                *truncated = true;
            }
        }
    }
}

/// Depth-first search for the longest non-empty string value anywhere inside `value`
/// (recursing through objects and arrays), returning a mutable handle to it so the caller
/// can shrink it in place. `None` once every string leaf is empty (or there are none).
fn largest_string_leaf(value: &mut serde_json::Value) -> Option<&mut String> {
    match value {
        serde_json::Value::String(s) if !s.is_empty() => Some(s),
        serde_json::Value::Array(items) => items
            .iter_mut()
            .filter_map(largest_string_leaf)
            .max_by_key(|s| s.len()),
        serde_json::Value::Object(map) => map
            .values_mut()
            .filter_map(largest_string_leaf)
            .max_by_key(|s| s.len()),
        _ => None,
    }
}

#[cfg(test)]
#[path = "conversation_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "conversation_bound_tests.rs"]
mod bound_tests;
