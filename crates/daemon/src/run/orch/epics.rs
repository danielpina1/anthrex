//! Milestone 9's epics and their sub-planner sessions (decisions 31–33, 37), moved out of
//! `run/orch/mod.rs` (milestone 9.16 ruling C8, move-only). Pure.

use proto::{Effort, IntegrationState, Route, Runtime, TokenUsage};
use serde::{Deserialize, Serialize};

use super::super::model::OpId;

/// A sub-planner's phase (decision 32). `Queued` and `Planning` are live.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PlannerPhase {
    #[default]
    Queued,
    Planning,
    Finished,
    Failed {
        reason: String,
    },
}

impl PlannerPhase {
    /// Queued or planning: the epic is its sub-planner's (decision 23.5).
    pub fn is_live(&self) -> bool {
        matches!(self, PlannerPhase::Queued | PlannerPhase::Planning)
    }
}

/// One epic and its sub-planner sessions (decisions 31–33, 37).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EpicRecord {
    pub epic: String,
    pub title: String,
    pub area: Vec<String>,
    pub brief: String,
    pub scout_refs: Vec<String>,
    pub route: Route,
    pub phase: PlannerPhase,
    /// The live or last session's brief.
    pub request: String,
    pub sessions: Vec<PlannerSession>,
    pub started_at: u64,
    pub ended_at: Option<u64>,
    pub edits_accepted: u32,
    pub edits_rejected: u32,
    pub last_rejection: Option<String>,
    pub replans: Vec<String>,
    pub note: Option<String>,
    pub gate_hold: Option<String>,
    pub base: Option<String>,
    /// `(task id, merge commit)`.
    pub merges: Vec<(String, String)>,
    pub integration_state: IntegrationState,
    pub integration_rounds: u32,
    /// How many of `merges` the latest integration review covers: a round is due only
    /// once a merge came after it (decision 37).
    #[serde(default)]
    pub integration_reviewed: u32,
    /// Milestone 9.6 ruling T11-1: in a design run, the requirement ids the epic owns,
    /// as its latest `spawn_subplanner` named them.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub covers: Vec<String>,
    /// Milestone 9.17 decision 18: the latest accepted `submit_epic`'s frame.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub frame: Option<super::plan_frame::PlanFrame>,
}

impl EpicRecord {
    /// A new epic as `spawn_subplanner` asks for it (decision 21), its sub-planner
    /// queued; the engine sets its route and start time.
    pub fn requested(
        epic: &str,
        title: &str,
        area: Vec<String>,
        brief: &str,
        scout_refs: Vec<String>,
    ) -> EpicRecord {
        EpicRecord {
            epic: epic.into(),
            title: title.into(),
            area,
            brief: brief.into(),
            scout_refs,
            route: Route {
                runtime: Runtime::Claude,
                model: String::new(),
                effort: Effort::HIGH,
            },
            phase: PlannerPhase::Queued,
            request: brief.into(),
            sessions: Vec::new(),
            started_at: 0,
            ended_at: None,
            edits_accepted: 0,
            edits_rejected: 0,
            last_rejection: None,
            replans: Vec::new(),
            note: None,
            gate_hold: None,
            base: None,
            merges: Vec::new(),
            integration_state: IntegrationState::default(),
            integration_rounds: 0,
            integration_reviewed: 0,
            covers: Vec::new(),
            frame: None,
        }
    }

    /// Decision 15's planner check (M9.8 review, ruling 3): `window` is the epic's
    /// latest session's, that session has not ended, and the epic is being planned (a
    /// queued re-plan has no such session yet). The engine's writes and the driver's
    /// reads both ask it.
    pub fn is_live_caller(&self, window: u32) -> bool {
        let latest = self.sessions.last();
        latest.and_then(|s| s.window_id) == Some(window)
            && latest.is_some_and(|s| s.ended_at.is_none())
            && self.phase == PlannerPhase::Planning
    }
}

/// One sub-planner session of an epic.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlannerSession {
    pub session: u32,
    pub window_id: Option<u32>,
    pub op: Option<OpId>,
    pub started_at: u64,
    pub ended_at: Option<u64>,
    pub usage: TokenUsage,
    /// The session's rejected `submit_epic` batches (decision 22's `max_rejections`).
    #[serde(default)]
    pub rejections: u32,
}

#[cfg(test)]
impl EpicRecord {
    /// A test epic `epic` in `phase`, with no sessions.
    pub fn new(epic: &str, phase: PlannerPhase) -> EpicRecord {
        EpicRecord {
            epic: epic.into(),
            title: format!("Epic {epic}"),
            area: vec![format!("crates/{epic}/**")],
            brief: String::new(),
            scout_refs: Vec::new(),
            route: Route {
                runtime: Runtime::Claude,
                model: "claude-opus-5".into(),
                effort: Effort::HIGH,
            },
            phase,
            request: String::new(),
            sessions: Vec::new(),
            started_at: 0,
            ended_at: None,
            edits_accepted: 0,
            edits_rejected: 0,
            last_rejection: None,
            replans: Vec::new(),
            note: None,
            gate_hold: None,
            base: None,
            merges: Vec::new(),
            integration_state: IntegrationState::default(),
            integration_rounds: 0,
            integration_reviewed: 0,
            covers: Vec::new(),
            frame: None,
        }
    }
}
