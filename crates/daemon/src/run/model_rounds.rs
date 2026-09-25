//! A task's session rounds and gate records: [`AgentRound`] and its states, the check,
//! proof and review records, done claims and the task's history events. Pure, like
//! `model.rs`, which re-exports them (split out of it in F4).

use super::*;

/// Decision 32's turn-end fallback.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum FallbackState {
    #[default]
    None,
    Counting,
    Nudged {
        had_commits: bool,
    },
}

/// Decision 32's stall watchdog.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum StallState {
    #[default]
    Watching,
    Interrupted {
        deadline: u64,
    },
    Nudged,
}

/// A turn that ended on an API error, waiting for its continue message.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum FailedTurn {
    #[default]
    None,
    WaitingContinue {
        at: u64,
        rate_limit: bool,
    },
    ContinueSent {
        rate_limit: bool,
    },
}

/// One agent session (worker or reviewer) on a task.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentRound {
    pub role: AgentRole,
    pub session: u32,
    pub round: u32,
    pub window_id: Option<u32>,
    pub route: Route,
    pub launch_op: OpId,
    pub session_id: Option<String>,
    pub pid: Option<u32>,
    pub ended: bool,
    pub started_at: u64,
    pub ended_at: Option<u64>,
    pub turn_open: bool,
    pub turns: u32,
    pub turn_had_task_done: bool,
    pub last_event: u64,
    pub tool_calls: u32,
    pub rate_limited_until: Option<u64>,
    pub in_retry_streak: bool,
    pub open_subagents: BTreeSet<String>,
    pub denials: u32,
    pub usage: TokenUsage,
    pub deaths: u8,
    pub fallback: FallbackState,
    pub stall: StallState,
    pub failed_turn: FailedTurn,
    pub review_nudged: bool,
    pub wrap_up_sent: bool,
    pub retiring: bool,
    pub delivery_failures: u8,
    /// A failed delivery is retried no earlier than this (decision 29's
    /// `DELIVERY_RETRY_SECS`; M8a.11 fix round 1).
    #[serde(default)]
    pub delivery_retry_at: Option<u64>,
    /// The tools of the `PermissionDenied` events seen in the open turn, so its
    /// `TurnEnded`'s `permission_denials` adds only the rest (decision 27; M8a.12 fix
    /// round 1, review m-4).
    #[serde(default)]
    pub turn_denied: Vec<String>,
    /// Seconds not charged: the task was not working or the run not running (T15-I3).
    #[serde(default)]
    pub excused_secs: u64,
    /// The latest denial, `<tool>: <reason>`, for `denied_text` (decision 32; M8a.12).
    #[serde(default)]
    pub last_denial: Option<String>,
    /// A completed turn's fallback waits for open sub-agents (decision 32; M8a.12).
    #[serde(default)]
    pub fallback_waiting: bool,
    /// The outbox messages an in-flight `ResumeSession` carries (decision 29; M8a.12).
    #[serde(default)]
    pub carried: Vec<u64>,
    /// The failed turn's error, for its `rate_limit_continue` (decision 32; M8a.12).
    #[serde(default)]
    pub failed_error: Option<String>,
    /// The `ResumeSession` this round awaits; any other resume's result is dropped
    /// (M8a.12 fix round 2, ruling T12-N).
    #[serde(default)]
    pub resume_op: Option<OpId>,
    /// The turn-end fallback's `CountCommits` this round awaits (ruling T12-N).
    #[serde(default)]
    pub count_op: Option<OpId>,
    /// Failed `CountCommits` in a row, and when the next one is sent (M8a.12 fix round
    /// 3, ruling T12-A2).
    #[serde(default)]
    pub count_failures: u8,
    #[serde(default)]
    pub count_retry_at: Option<u64>,
    /// The `turns` the fallback's count (and its retry) was issued for: its result is
    /// dropped once a later turn has started (M8a.12 fix round 4, ruling T12-R4).
    #[serde(default)]
    pub count_turn: u32,
    /// The open turn was interrupted by the stall watchdog: its end sends the queued
    /// `stall_nudge` only, never the fallback, even after activity inside the grace
    /// ended the grace (M8a.12 fix round 4, ruling T12-R4 on N3-1).
    #[serde(default)]
    pub interrupted: bool,
    /// The `CreateWindow` lost at a daemon restart (M8a.15): re-issued, with a new op
    /// id, by the first running pass (decision 44's "the engine re-issues").
    #[serde(default)]
    pub relaunch: Option<Box<OpKind>>,
    /// The process a turn last ended in (ruling T13-P1): its later exit is normal.
    #[serde(default)]
    pub closed_pid: Option<u32>,
    /// The process whose exit the round last took (final review B-10): its exit again
    /// (the engine's synthetic copy and the real one) is dropped until the next
    /// process starts.
    #[serde(default)]
    pub exited_pid: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CheckRecord {
    pub at: u64,
    pub ok: bool,
    pub code: Option<i32>,
    pub timed_out: bool,
    pub tail: String,
    pub secs: u64,
    pub on_candidate: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProofRecord {
    pub at: u64,
    pub test: String,
    pub red: String,
    /// The task head the proof ran at (M8a.13: `proof_failed_message` names it).
    #[serde(default)]
    pub head: String,
    pub red_failed: bool,
    pub head_passed: bool,
    pub matched: bool,
    pub red_tail: String,
    pub head_tail: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReviewRecord {
    pub round: u32,
    pub route: Route,
    pub base: String,
    pub head: String,
    pub verdict: Option<Verdict>,
    pub summary: String,
    pub findings: Vec<Finding>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DoneClaim {
    pub summary: String,
    pub test: Option<String>,
    pub red: Option<String>,
    pub signal: DoneSignal,
}

/// A `task_done` claim (or the turn-end fallback's) whose `VerifyDone` is in flight;
/// `reply` is the tool call's, `None` for the fallback (decision 32; M8a.12).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PendingClaim {
    pub reply: Option<u64>,
    pub claim: DoneClaim,
    /// The claiming round's window: a result for a round that is no longer the task's
    /// live worker is dropped (M8a.12 fix round 1, ruling T12-I1).
    #[serde(default)]
    pub window_id: Option<u32>,
    /// The claim's `VerifyDone`: only that op's result settles it (M8a.12 fix round 2,
    /// ruling T12-N).
    #[serde(default)]
    pub op: Option<OpId>,
    /// The claiming round's `turns` when the claim was made: a verdict that finds a
    /// later turn open waits for its end (M8a.12 fix round 3, ruling T12-O2).
    #[serde(default)]
    pub turn: u32,
}

/// A fresh worker session the ladder (rung 2) or a failed resume has decided on,
/// started once the old session is gone and `DiffSoFar` has come back (decisions 28,
/// 30, 38; M8a.12). `append` ends the prompt: the messages a failed resume carried.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FreshSession {
    pub reason: String,
    pub append: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TaskEvent {
    pub at: u64,
    pub text: String,
}
