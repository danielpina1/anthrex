//! Milestone 9's engine events (`OrchEvent`), moved out of `orch.rs` so that file stays
//! under 600 lines. Pure (design decision 2).

use proto::{RoleOutcome, RoleRoutingDecision, Runtime, TokenUsage, ToolCall};

use super::super::ReplyId;

/// The events of milestone 9's agents. Later tasks add the scouts', sub-planners' and
/// wake-ups' own.
#[derive(Debug, Clone, PartialEq)]
pub enum OrchEvent {
    /// Decision 15: an orchestrator's or sub-planner's write tool, or a worker's
    /// `task_note`, with the runtime refusals the driver computed (ruling T22-I1b).
    Tool {
        reply: ReplyId,
        call: ToolCall,
        refusals: Vec<(Runtime, String)>,
    },
    /// Decision 28's verdicts, from the user's `run approve|reject --hold` only.
    ApproveHold {
        reply: ReplyId,
        run_id: String,
        hold: String,
    },
    RejectHold {
        reply: ReplyId,
        run_id: String,
        hold: String,
    },
    /// Decision 20: a run scout's session ended (the driver awaits its handle).
    ScoutEnded {
        run_id: String,
        scout_id: String,
        outcome: ScoutEnd,
        usage: TokenUsage,
    },
    /// Decision 32: a sub-planner's session ended.
    PlannerEnded {
        run_id: String,
        epic: String,
        session: u32,
        outcome: ScoutEnd,
        usage: TokenUsage,
    },
    /// Decision 39: the driver pasted the wake-up of `digest_revision` into the
    /// orchestrator's window; `notes_seq` is its `Effect::WakeOrchestrator`'s.
    OrchestratorWoken {
        run_id: String,
        digest_revision: u64,
        notes_seq: u64,
        /// Milestone 9.3 decision 11 (D13): the wake carried the run's `request_wake`,
        /// which only this clears.
        request: bool,
    },
    /// Decisions 16 and 39: the orchestrator read the digest at `digest_revision`,
    /// whose answer included the wake notes up to `notes_seq` (`wake::notes_seq` of
    /// the run the answer was built from; M9.9 review fixes, M6).
    DigestRead {
        run_id: String,
        digest_revision: u64,
        notes_seq: u64,
    },
    /// Decision 13: the driver saw the orchestrator's window exit (`live: false`), or
    /// come back after an exit (`live: true`, the user's `anthrex restart`).
    OrchestratorWindow {
        run_id: String,
        window_id: u32,
        live: bool,
        /// The record's `launches` when the driver looked (M9.13 review).
        launch: u64,
    },
    /// Decision 14a: the run's OTLP token, drawn by the driver from the OS random
    /// source when it launches an orchestrator whose record has none.
    OtlpToken { run_id: String, token: String },
    /// Whole-branch fix round 2, item 2: the driver holds `run_id`'s wake-up only
    /// because its orchestrator's window was at a prompt (`held`), or no longer does.
    WakeHeld { run_id: String, held: bool },
    /// M9.17 fix round 2: `run promote`'s installed check found `installed` (decision
    /// 17's map); a fast-path run with no orchestrator yet records it, so its promoted
    /// sub-planners' route agrees with the check.
    Installed {
        run_id: String,
        installed: std::collections::BTreeMap<String, bool>,
    },
    /// Decision 43: the record of a session the driver dispatches (a run-bound decider,
    /// a run scout), sent before the session starts.
    /// Answered once the record is kept (review M-2): the driver starts the session
    /// only after the step that keeps it was saved.
    RoleRoute {
        reply: ReplyId,
        run_id: String,
        decision: Box<RoleRoutingDecision>,
    },
    /// Decision 43: that session ended.
    RoleRouteEnded {
        run_id: String,
        record_id: String,
        outcome: RoleOutcome,
        result: Option<String>,
    },
}

/// How a run scout's or sub-planner's session ended: its report or epic accepted, or
/// the machine's failure.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ScoutEnd {
    Reported,
    Failed { reason: String },
}

impl OrchEvent {
    /// The request the event answers, if any.
    pub fn reply(&self) -> Option<ReplyId> {
        match self {
            OrchEvent::Tool { reply, .. }
            | OrchEvent::ApproveHold { reply, .. }
            | OrchEvent::RejectHold { reply, .. }
            | OrchEvent::RoleRoute { reply, .. } => Some(*reply),
            OrchEvent::ScoutEnded { .. }
            | OrchEvent::PlannerEnded { .. }
            | OrchEvent::OrchestratorWoken { .. }
            | OrchEvent::DigestRead { .. }
            | OrchEvent::OrchestratorWindow { .. }
            | OrchEvent::OtlpToken { .. }
            | OrchEvent::WakeHeld { .. }
            | OrchEvent::Installed { .. }
            | OrchEvent::RoleRouteEnded { .. } => None,
        }
    }
}
