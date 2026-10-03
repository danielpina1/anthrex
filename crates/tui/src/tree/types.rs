//! The tree's plain value types, split out of `tree.rs` per the 600-line rule.

use proto::{AgentRoundInfo, WindowInfo};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct RuntimeCounts {
    pub claude: usize,
    pub codex: usize,
    pub shell: usize,
}

/// One agent-round node of the run view (milestone 8c decision 14).
#[derive(Debug, Clone, PartialEq)]
pub struct DisplayRound<'a> {
    pub info: &'a AgentRoundInfo,
    pub number: u32,
    pub started_at: u64,
    pub ended_at: Option<u64>,
    /// The session's last display round: its counters and sub-agents hang here.
    pub last: bool,
    pub window: Option<&'a WindowInfo>,
}
