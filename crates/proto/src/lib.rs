//! Wire types shared by the daemon, the TUI client, and the CLI.

/// Bumped whenever a message shape changes incompatibly — or whenever an existing
/// field's *meaning* changes, which is why this is 6 and not 5.
///
/// Milestone 6.5 adds the agent conversation model (`proto::conversation`) and five
/// messages built on it: `ClientMsg::SubscribeConversation` and
/// `ClientMsg::UnsubscribeConversation`, and `DaemonMsg::ConversationSnapshot`,
/// `DaemonMsg::ConversationDelta` and `DaemonMsg::ConversationGone`. None of these replace
/// or change the meaning of an existing field, but a milestone-5 daemon paired with a
/// milestone-6.5 client (or the reverse) has no way to decode a message it has never seen
/// — `read_frame` would hand back a decode error indistinguishable from a corrupt frame,
/// on a connection that otherwise looks healthy. The version bump turns that into a clean
/// handshake refusal instead, which is what every earlier bump on this line has done for
/// a genuinely new shape, not only for a changed one.
///
/// Task M6.5.10 added `session_id` to `ConversationDelta` without a further bump: version
/// 6 has not shipped, so no client or daemon speaking a 6 without it exists.
///
/// Milestone 8a task 2 bumps this to 7: it adds `ClientMsg::Run(RunRequest)` and
/// `DaemonMsg::Run(RunReply)`, two message shapes a milestone-6.5 daemon or client has
/// never seen and cannot decode, plus `HookSource::Stream`, a new variant of an existing
/// enum a milestone-6.5 peer would also fail to decode. Derivation: `PROTO_VERSION` was 6
/// at `crates/proto/src/lib.rs:19` before this change (set by M6.5); 6 + 1 = 7.
///
/// Milestone 8b task 2 bumps this to 8: it adds `AgentRole::Scout`, the `RunRequest`
/// variants `StartGoal`, `Promote`, `Stats` and `Profile`, the `RunReply` variants
/// `Triaged`, `Profile` and `Stats`, and `ToolCall.scout_id` — new variants a
/// milestone-8a peer cannot decode. The new snapshot fields are `#[serde(default)]`, so a
/// milestone-8a `run.json` still loads. Derivation: `PROTO_VERSION` was 7 at
/// `crates/proto/src/lib.rs:25` before this change (set by M8a task 2); 7 + 1 = 8.
///
/// Milestone 8c task 1 bumps this to 9: it adds the run view's snapshot fields
/// (`RunsSnapshot.now`, the plan's approval time and edit log, task briefs and route
/// specs, rate-limit times, rung 1 bounces, the planner placeholders) and changes
/// `TaskInfo.history`'s element type to `TaskEventInfo`. The new fields are all
/// `#[serde(default)]`; the bump keeps an 8 client from showing a 9 daemon's view half
/// filled. Derivation: `PROTO_VERSION` was 8 at `crates/proto/src/lib.rs:32` before
/// this change (set by M8b task 2); 8 + 1 = 9.
///
/// Milestone 9 task 2 bumps this to 10: it adds `ClientMsg::RunTagged` and the
/// `request_id` its replies echo, the `RunRequest` variants `ApproveHold` and
/// `RejectHold`, `AgentRole::Planner`, `RunState::Planning`, `TaskState::Reported`,
/// `BlockReason::MessagePause`, the `PlanEdit` variants `Message` and `Refresh`, and the
/// orchestrator, hold, integration and task-note snapshot types (`proto::orch`) —
/// variants a milestone-8c peer cannot decode. Every new field is `#[serde(default)]`,
/// so a milestone-8c `run.json` and snapshot still load. Derivation: `PROTO_VERSION`
/// was 9 at `crates/proto/src/lib.rs:40` before this change (set by M8c task 1);
/// 9 + 1 = 10.
pub const PROTO_VERSION: u32 = 10;

/// How long the daemon waits for a freshly connected client's `Hello`, and how long a
/// client waits for the daemon's `Welcome`, before giving up on the handshake. Design
/// decision 29: a client that connects and then says nothing must not hold a daemon
/// resource forever, and a daemon that accepted but never answers must not hang a client
/// indefinitely either.
pub const HANDSHAKE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5);

pub mod adapt;
pub mod codec;
pub mod conversation;
pub mod history;
pub mod messages;
pub mod orch;
pub mod paths;
pub mod planner;
pub mod profile;
pub mod run;
pub mod run_info;
pub mod run_wire;
pub mod scout;
pub mod types;

pub use adapt::{
    DeciderMode, DeciderSource, DiffStats, PhaseSecs, RunPath, RunUsage, Scale, SizeCheckInfo,
    TriageInfo,
};
pub use codec::{CodecError, MAX_FRAME, decode, encode, read_frame, write_frame};
pub use conversation::{
    Block, Conversation, DegradeReason, DropCause, NoticeKind, Role, ToolResult, ToolState, Turn,
    TurnPatch, TurnState,
};
pub use history::{
    GateTally, HISTORY_VERSION, HistoryLine, HistoryStats, RevertRecord, RoleOutcome,
    RoleRoutingDecision, RoleRoutingInput, RoutingCandidate, RoutingDecision, RoutingInput,
    RunRecord, SeverityTally, StatsRow, TaskOutcome, TaskRecord,
};
pub use messages::{ClientMsg, DaemonMsg, HookSource};
pub use orch::{
    HoldInfo, HoldKind, HoldState, IntegrationInfo, IntegrationState, MessageKind, MessageTarget,
    OrchestratorChoice, OrchestratorInfo, TaskNoteInfo, TaskNoteKind,
};
// Re-exported by name, never by glob (C20): a glob re-export of `run` or `run_wire`
// could silently shadow an existing root name (for instance `run_wire::request` beside
// `messages::request`) the next time either module gains a public item, with no
// compile error to catch it.
pub use planner::{PlannerInfo, PlannerState};
pub use profile::{
    CommandCheck, DroppedCommand, OutputFilter, ProfileMeta, ProfileSource, ProfileStatus,
    ProfileVerification, ProposalOrigin, ProposalRecord, ProposalState, RepoProfile,
};
pub use run::{
    AgentRole, BlockReason, Budget, DoneSignal, EditFile, Effort, Finding, FinishAction,
    GateCounts, GateKind, ModelEntry, Plan, PlanEdit, PlanTask, ProfileSpec, Route, RouteSpec,
    RunRef, RunState, Severity, Size, Strength, TaskKind, TaskState, TestMode, Verdict,
};
pub use run_info::{
    AgentRoundInfo, BaseMovedInfo, BlockInfo, CheckInfo, PlanEditInfo, ProofInfo, ReviewInfo,
    RunInfo, RunsSnapshot, Spend, TaskEventInfo, TaskInfo, TokenUsage,
};
pub use run_wire::{ProfileReply, ProfileRequest, RunReply, RunRequest, ToolCall};
pub use scout::{ScoutFile, ScoutInfo, ScoutKind, ScoutReport, ScoutState};
pub use types::{
    ClientKind, ExitInfo, GitOperation, GitState, Head, Runtime, Status, SubagentInfo,
    SubagentState, WindowInfo, WindowKind, WindowSpec,
};

#[cfg(test)]
#[path = "adapt_tests.rs"]
mod adapt_tests;

#[cfg(test)]
#[path = "orch_tests.rs"]
mod orch_tests;

#[cfg(test)]
mod tests {
    #[test]
    fn proto_version_is_ten() {
        assert_eq!(super::PROTO_VERSION, 10);
    }

    #[test]
    fn handshake_timeout_is_five_seconds() {
        assert_eq!(super::HANDSHAKE_TIMEOUT, std::time::Duration::from_secs(5));
    }
}
