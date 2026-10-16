//! Milestone 9.17 decision 18: a submitted plan's frame, what the orchestrator (per
//! round) or a sub-planner (per epic) decided for the user and what the plan leaves
//! out. Pure: types and lookups only. Milestone 9.17's task M9.17.5 writes them; task
//! M9.17.3 adds the types, which nothing writes yet, and the reviewer prompt reads them
//! (`contract_scope::out_of_scope_block`).

use proto::DecidedItem;
use serde::{Deserialize, Serialize};

use crate::run::model::Run;

/// Decision 13: at most this many `decided` items in one submit.
pub const DECIDED_MAX: usize = 20;
/// Decision 13: at most this many `out_of_scope` items in one submit.
pub const OUT_OF_SCOPE_ITEMS_MAX: usize = 20;

/// One plan's frame (decision 18). `RunOrch.plan_frames` holds at most one per round;
/// `EpicRecord.frame` holds its epic's latest. Every text is stored one line, its
/// control characters dropped.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlanFrame {
    pub round: u32,
    pub decided: Vec<DecidedItem>,
    pub out_of_scope: Vec<String>,
    pub at: u64,
}

/// The frame of `round`'s plan, if its submit carried one.
pub fn frame_for(run: &Run, round: u32) -> Option<&PlanFrame> {
    run.orch.plan_frames.iter().find(|f| f.round == round)
}

/// Decision 18: a frame text as stored. A line break or tab becomes a space, any other
/// control character is dropped, and the ends are trimmed. The parser
/// refuses an item this leaves empty (task 5 review M2).
pub fn clean(text: &str) -> String {
    let kept: String = (text.chars())
        .filter(|&c| !(c.is_control() && !c.is_whitespace()))
        .collect();
    crate::run::messages::one_line(&kept).trim().to_string()
}

#[cfg(test)]
#[path = "plan_frame_tests.rs"]
mod tests;
