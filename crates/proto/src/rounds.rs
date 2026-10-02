//! Milestone 9.3 (keep going): rounds of one run, chains of runs on one orchestrator
//! session, and the `round` history line, as the wire carries them (KG §5).
//!
//! Every field that a protocol-14 peer never wrote is `#[serde(default)]`, so an older
//! snapshot, `run.json` and `history.jsonl` still decode.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::run::RunState;
use crate::types::Runtime;

/// The longest goal, round request or next goal, in characters (KG §1.3, §2.1, §8).
pub const GOAL_MAX_CHARS: usize = 16_384;
/// The most rounds one run can have (KG §2.1).
pub const ROUNDS_MAX: u32 = 20;
/// Characters kept in a round's `goal_head` and `summary_head` (KG §5).
pub const ROUND_HEAD_CHARS: usize = 200;

/// The serde default of every `round`: a run, task or stage from before milestone 9.3
/// belongs to round 1.
pub fn first_round() -> u32 {
    1
}

/// Who started a round: round 1 is always the user's goal.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RoundOrigin {
    User,
    Orchestrator,
}

/// How a round ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RoundOutcome {
    Completed,
    Rejected,
    Cancelled,
}

/// One round of a run, as shown to a client.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RoundInfo {
    pub n: u32,
    /// The whole request on one line (`safe_text::one_line`), cut to
    /// [`ROUND_HEAD_CHARS`] characters.
    pub goal_head: String,
    pub origin: RoundOrigin,
    /// `None` while the round runs.
    #[serde(default)]
    pub outcome: Option<RoundOutcome>,
    #[serde(default)]
    pub summary_head: Option<String>,
}

/// A project's idle orchestrator: the head of a chain whose last run was accepted or
/// discarded (KG §3.1).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IdleOrchestrator {
    /// `"o-<h4>"`, after the chain's first run.
    pub chain: String,
    pub project: PathBuf,
    /// The chain's last run id.
    pub after_run: String,
    /// `Accepted` or `Discarded`.
    pub outcome: RunState,
    pub runtime: Runtime,
    pub model: String,
    /// `None` once `fresh`.
    #[serde(default)]
    pub window_id: Option<u32>,
    /// The session ended: continuing starts a fresh one (KG §3.6).
    pub fresh: bool,
    /// The chain's runs (KG §11; defect D8).
    #[serde(default)]
    pub runs: u32,
}

/// One ended round (`"type": "round"`), `record_id` `"<run>/round/<n>"`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RoundLine {
    pub v: u32,
    pub record_id: String,
    pub at: u64,
    pub run_id: String,
    pub round: u32,
    pub origin: RoundOrigin,
    pub outcome: RoundOutcome,
    pub tasks: u32,
    pub merged: u32,
    pub calls: u32,
    pub minutes: u64,
}

#[cfg(test)]
#[path = "rounds_tests.rs"]
mod tests;
