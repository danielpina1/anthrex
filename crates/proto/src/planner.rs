//! Sub-planners (milestone 9), as a run snapshot shows them. Milestone 8c adds the types
//! as `#[serde(default)]` placeholders on `RunInfo.planners`, so the run view can render
//! their absence; nothing fills them until milestone 9.

use serde::{Deserialize, Serialize};

use crate::run::Route;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PlannerState {
    #[default]
    Planning,
    Finished,
    Failed,
}

/// One sub-planner: the epic it plans, its area, and how its edits fared.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlannerInfo {
    pub epic: String,
    pub title: String,
    pub area: Vec<String>,
    pub route: Route,
    pub window_id: Option<u32>,
    pub state: PlannerState,
    pub started_at: u64,
    pub ended_at: Option<u64>,
    pub edits_accepted: u32,
    pub edits_rejected: u32,
    pub last_rejection: Option<String>,
    pub replans: Vec<String>,
    /// Milestone 9: why the planner ended as it did, when it says.
    #[serde(default)]
    pub note: Option<String>,
}
