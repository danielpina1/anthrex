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

#[cfg(test)]
#[path = "conversation_tests.rs"]
mod tests;
