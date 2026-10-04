//! The reducer's input events: [`EventKind`] and a window's [`AgentSignal`]s.

use proto::{FinishAction, PlanEdit, TokenUsage, ToolCall};

use super::delivery;
use super::{OpResult, OrchEvent, Rebaseline, ReplyId, TurnOutcome};
use crate::run::model::{OpId, Run};
use crate::run::validate::EditScope;

#[derive(Debug, Clone, PartialEq)]
pub enum EventKind {
    Start {
        reply: ReplyId,
        /// Boxed: a whole run dwarfs every other event.
        run: Box<Run>,
    },
    Approve {
        reply: ReplyId,
        run_id: String,
    },
    Reject {
        reply: ReplyId,
        run_id: String,
    },
    Edit {
        reply: ReplyId,
        run_id: String,
        edits: Vec<PlanEdit>,
        scope: EditScope,
        /// Ruling T22-I1b: decision 50's or 53's refusal for each runtime the run could
        /// not reach when the request came in and whose checks fail; an edit that makes
        /// one of them reachable is refused with its text.
        refusals: Vec<(proto::Runtime, String)>,
        /// Milestone 9 decision 13: the user submits a planning run's plan after the
        /// batch (M9.7 review fixes, ruling 5).
        submit: bool,
    },
    Retry {
        reply: ReplyId,
        run_id: String,
        task_id: String,
    },
    Override {
        reply: ReplyId,
        run_id: String,
        task_id: String,
        reason: String,
    },
    Cancel {
        reply: ReplyId,
        run_id: String,
    },
    /// `rebaseline`: the refs the driver read (milestone 9.1: every stage head too).
    Resume {
        reply: ReplyId,
        run_id: String,
        rebaseline: Option<Rebaseline>,
    },
    /// Decision 21: a guard saw the base branch advance.
    BaseAdvanced {
        run_id: String,
        to: String,
        commits: u32,
    },
    Finish {
        reply: ReplyId,
        run_id: String,
        action: FinishAction,
    },
    Tool {
        reply: ReplyId,
        call: ToolCall,
    },
    /// M8b decision 25: `run promote`, which milestone 9 performs (decision 29) with the
    /// orchestrator the user chose, if any.
    Promote {
        reply: ReplyId,
        run_id: String,
        orchestrator: Option<proto::OrchestratorChoice>,
    },
    /// M8b decision 30: the OTLP ledger's new total for `(run, "orchestrator")`. It
    /// replaces the one before, on top of the usage restored at the daemon's start.
    OrchestratorUsage {
        run_id: String,
        usage: TokenUsage,
    },
    OpDone {
        run_id: String,
        op: OpId,
        result: OpResult,
    },
    Signal {
        window_id: u32,
        signal: AgentSignal,
    },
    Delivered {
        run_id: String,
        message_ids: Vec<u64>,
        ok: bool,
        error: Option<String>,
    },
    Restore {
        runs: Vec<Run>,
        replay: Vec<(String, OpId, OpResult)>,
        /// Ops kept pending with no answer yet: the driver answers them later with an
        /// ordinary `OpDone` (ruling T22-N3: a replayed accept's clean-up, run once the
        /// socket is bound).
        held: Vec<(String, OpId)>,
    },
    Stop,
    Tick,
    /// Milestone 9: the orchestrator's and sub-planners' events (`orch.rs`).
    Orch(OrchEvent),
    /// Milestone 9.2 decision 25: `run deliver` and `run watch` (`delivery/`).
    Delivery(delivery::DeliveryRequest),
    /// Milestone 9.3 decision 10: `run iterate` (`goal_rounds.rs`).
    Iterate {
        reply: ReplyId,
        run_id: String,
        goal: String,
    },
}

/// The driver's translation of a window's session events (decision 27).
///
/// **Ordering contract** (ruling T13-P1), which `signals::apply` relies on and the
/// session driver guarantees (`headless::session`'s module doc, carried on the manager's
/// feed as `WindowSignal.pid`):
///
/// - every signal of a process carries, or is sent for, that process's pid;
/// - `ProcessStarted { pid }` for a new process is delivered before any other signal of
///   that process;
/// - a process's `ProcessExited` is delivered after its last stream signal, including a
///   `TurnEnded { Failed { SandboxUnavailable } }` the driver synthesises from stderr
///   before `Init` (M8a.12's carry).
///
/// Signals of two different processes of one window (a Codex turn's process and the
/// next, a killed Claude process and its `--resume`) may interleave; the pid tells them
/// apart. M8a.22's driver forwards the feed in the order it receives it.
#[derive(Debug, Clone, PartialEq)]
pub enum AgentSignal {
    Init {
        session_id: String,
    },
    TurnStarted,
    ToolUse {
        name: String,
        target: Option<String>, // 9.0.5 decision 5: its summary; `None` for a sub-agent's
    },
    /// `denials`: the tool names of the result's `permission_denials` (M8a.12 fix round
    /// 1, review m-4: the Interfaces' `u32` became the names, which `denied_text` needs).
    TurnEnded {
        outcome: TurnOutcome,
        usage: Option<TokenUsage>,
        denials: Vec<String>,
    },
    ApiRetry {
        error: String,
        delay_ms: u64,
    },
    PermissionDenied {
        tool: String,
        reason: String,
    },
    SubagentStart {
        agent_id: String,
    },
    SubagentStop {
        agent_id: String,
    },
    /// The usage of a turn Claude Code started by itself while a delivered turn waits
    /// (`WindowSignalKind::Unprompted`, ruling T7-N1): spend, never a turn end.
    Spend {
        usage: TokenUsage,
    },
    /// Any other event: a sub-agent's text, a tool result, compaction, an unknown line.
    Activity,
    ProcessExited {
        code: Option<i32>,
        killed_by_engine: bool,
        pid: u32,
    },
    ProcessStarted {
        pid: u32,
    },
    /// Top-level assistant text, cut at `proto::WORKER_SUMMARY_MAX` (9.0.5 decision 5).
    Said {
        text: String,
    },
}
