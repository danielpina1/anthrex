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
    /// Absolute Unix seconds (decision A2), not a wall-clock timestamp type — those
    /// are neither `Serialize` nor portable across the wire in a fixed shape.
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
pub const TURN_OVERHEAD: usize = 64;
pub const BLOCK_OVERHEAD: usize = 32;

/// `ConversationGone.reason` values (decision-pinned wire strings; see `messages.rs`).
pub const GONE_WINDOW_REMOVED: &str = "window removed";
pub const GONE_SUBAGENT_UNKNOWN: &str = "no such sub-agent in this window";
pub const GONE_WINDOW_UNKNOWN: &str = "no such window";

/// The compact-JSON length of a `serde_json::Value`, used to measure `input` and
/// `result` fields the same way regardless of how they are stored in memory.
fn json_byte_len(value: &serde_json::Value) -> usize {
    serde_json::to_string(value).map(|s| s.len()).unwrap_or(0)
}

impl Block {
    fn byte_size(&self) -> usize {
        let content = match self {
            Block::Text { text } => text.len(),
            Block::ToolCall {
                id: _,
                name,
                summary,
                input,
                result,
                state: _,
                duration_ms: _,
            } => {
                // `id` is the runtime's own correlation id, not content shown to the
                // user, so it does not count toward the cap (task brief's
                // `byte_size_counts_structure_and_content`).
                name.len()
                    + summary.len()
                    + input.as_ref().map(json_byte_len).unwrap_or(0)
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
