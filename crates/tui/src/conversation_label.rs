//! Milestone 9 decision 42i: who a user turn in the conversation view came from. A
//! message delivered to a worker is a user turn whose text starts with the daemon's
//! own prefix, `[anthrex] Message from the orchestrator (` or `[anthrex] Message from
//! the user (` (`run/engine/worker_messages.rs`), so the view labels it `orchestrator`
//! or `user`. Every other user turn keeps `you`. No protocol field says so (the
//! controller's ruling): the prefix is the whole signal. Pure.

use proto::{Block, Turn};

const FROM_ORCHESTRATOR: &str = "[anthrex] Message from the orchestrator (";
const FROM_USER: &str = "[anthrex] Message from the user (";

/// The header label of a user turn: `orchestrator`, `user`, or `you`.
pub fn user_turn_label(turn: Option<&Turn>) -> &'static str {
    let first = turn.and_then(|turn| {
        turn.blocks.iter().find_map(|block| match block {
            Block::Text { text } => Some(text.as_str()),
            _ => None,
        })
    });
    match first {
        Some(text) if text.starts_with(FROM_ORCHESTRATOR) => "orchestrator",
        Some(text) if text.starts_with(FROM_USER) => "user",
        _ => "you",
    }
}
