//! Pure per-window sub-agent state derived from agent hook events.
//!
//! Split by responsibility, `subagents.rs` having grown past the repository's
//! 600-line rule (`AGENTS.md`): `spawn` parses a sub-agent spawn request out
//! of a hook's tool input into a `PendingSpawn`; `tracker` holds
//! `SubagentTracker`, the state machine that turns a stream of hook events
//! into `SubagentInfo` rows and matches a `SubagentStart` against the
//! `PendingSpawn` that queued it. This file only wires the two together and
//! re-exports what the rest of the daemon needs.

mod spawn;
mod tracker;

pub use spawn::{
    CLAUDE_SPAWN_TOOL, CODEX_SPAWN_LABEL_KEYS, CODEX_SPAWN_MODEL_KEY, CODEX_SPAWN_TOOL,
    CODEX_SPAWN_TOOL_ALIAS, CODEX_SPAWN_TYPE_KEY, LABEL_MAX_CHARS, PendingSpawn, make_label,
    spawn_request,
};
pub use tracker::{MAX_PENDING_SPAWNS, MAX_SUBAGENTS, SUBAGENT_RETENTION, SubagentTracker};

#[cfg(test)]
mod tests;
