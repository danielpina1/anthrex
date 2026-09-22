//! Per-window conversation state (spec decision 5 onward, milestone 6.5). Only
//! `summary` is wired up by task M6.5.4; later tasks in this milestone add `store`,
//! `build`, `enrich` and `watch` beside it.

mod summary;
pub use summary::{SUMMARY_MAX_GRAPHEMES, for_tool};

/// The three caps from `[conversation]`, resolved to `usize` (spec decision 7).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Caps {
    pub max_turns: usize,
    pub max_bytes: usize,
    pub max_result_bytes: usize,
}

impl Caps {
    pub fn from_config(c: &config::Conversation) -> Self {
        Caps {
            max_turns: c.max_turns as usize,
            max_bytes: c.max_bytes as usize,
            max_result_bytes: c.max_result_bytes as usize,
        }
    }
}

impl Default for Caps {
    fn default() -> Self {
        Caps::from_config(&config::Conversation::default())
    }
}
