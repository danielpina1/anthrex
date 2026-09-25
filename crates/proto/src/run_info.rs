//! The read-only run snapshot: what `run list` and the TUI's run view are shown.
//!
//! Every field that later milestones (M8c, M9) might add to is `#[serde(default)]`
//! here, so a struct that gains a field between milestones still deserializes against
//! an older sender.

use std::collections::BTreeMap;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::run::{
    AgentRole, BlockReason, Budget, DoneSignal, GateCounts, Route, RunState, Size, TaskKind,
    TaskState, TestMode, Verdict,
};
pub use crate::run::{Finding, Severity};

/// How much of a task's or agent round's budget has been used.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Spend {
    pub tool_calls: u32,
    pub secs: u64,
    pub tokens: u64,
}

/// Token counts for one agent round, split the way the runtimes report them.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TokenUsage {
    pub input: u64,
    pub output: u64,
    pub cache_read: u64,
    pub cache_write: u64,
}

impl TokenUsage {
    /// Decision 40: the tokens actually billed. Cache reads are excluded — they are
    /// far cheaper than a fresh input token and would otherwise dominate the total for
    /// a long-lived session. Defined here, in `proto`, because the daemon cannot add
    /// an inherent impl to a foreign type.
    pub fn billable(&self) -> u64 {
        self.input + self.cache_write + self.output
    }
}

/// Why a task is blocked, with the human-readable detail.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BlockInfo {
    pub reason: BlockReason,
    pub text: String,
}

/// The outcome of one `check` run.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CheckInfo {
    pub at: u64,
    pub ok: bool,
    pub code: Option<i32>,
    pub timed_out: bool,
    pub secs: u64,
    pub summary: String,
    pub on_candidate: bool,
}

/// The outcome of one proof (TDD red/green) run.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProofInfo {
    pub at: u64,
    pub test: String,
    pub red: String,
    pub red_failed: bool,
    pub head_passed: bool,
    pub matched: bool,
    pub ok: bool,
}

/// The outcome of one review round.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReviewInfo {
    pub round: u32,
    pub route: Route,
    pub verdict: Option<Verdict>,
    pub summary: String,
    pub findings: Vec<Finding>,
    pub blocking: bool,
}

/// One agent round: one process, one session, on one task (or none, for a reviewer that
/// works across tasks in later milestones).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentRoundInfo {
    pub role: AgentRole,
    pub session: u32,
    pub round: u32,
    pub window_id: Option<u32>,
    pub route: Route,
    /// The runtime's own session id (decision 52), not anthrex's `RunRef`.
    pub session_id: Option<String>,
    pub started_at: u64,
    pub ended_at: Option<u64>,
    pub tool_calls: u32,
    pub last_event: u64,
    pub turn_open: bool,
    pub turns: u32,
    pub rate_limited: bool,
    pub open_subagents: u32,
    pub denials: u32,
    pub usage: TokenUsage,
}

/// One task's full state, as shown to a client.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TaskInfo {
    pub id: String,
    pub title: String,
    pub epic: Option<String>,
    pub kind: TaskKind,
    pub size: Size,
    pub hub: bool,
    pub test_mode: TestMode,
    pub test_mode_reason: Option<String>,
    /// Decision 9/10 raises and forced test modes, as human-readable notes.
    pub notes: Vec<String>,
    pub owns: Vec<String>,
    pub deps: Vec<String>,
    pub implicit_deps: Vec<String>,
    pub priority: i32,
    pub route: Route,
    pub review_route: Option<Route>,
    pub budget: Budget,
    pub spent_session: Spend,
    pub spent_total: Spend,
    pub state: TaskState,
    pub block: Option<BlockInfo>,
    pub rung: u8,
    pub failures: u8,
    pub bounces: GateCounts,
    pub stalls: u8,
    pub budget_exceeded: u8,
    pub conflicts: u8,
    pub branch: String,
    pub worktree: PathBuf,
    pub start_commit: Option<String>,
    pub head: Option<String>,
    pub test: Option<String>,
    pub red: Option<String>,
    pub done_signal: Option<DoneSignal>,
    pub rounds: Vec<AgentRoundInfo>,
    pub reviews: Vec<ReviewInfo>,
    pub last_check: Option<CheckInfo>,
    pub last_proof: Option<ProofInfo>,
    pub merge_commit: Option<String>,
    pub merged_without_approval: Option<String>,
    pub salvage_refs: Vec<String>,
    pub on_critical_path: bool,
    pub wave: u32,
    /// The last 10 events, newest first, each `"<hh:mm> <text>"`.
    pub history: Vec<String>,
}

/// The base branch has moved under a run (decision 21): not a halt on its own, but
/// recorded so the client can show it and, when needed, confirm accepting onto it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BaseMovedInfo {
    pub from: String,
    pub to: String,
    /// `"<sha7> <author>: <subject>"`, newest first, at most 50; empty in a snapshot,
    /// filled in `RunReply::ConfirmNeeded`.
    pub commits: Vec<String>,
    /// Every commit in `from..to`, which can exceed `commits.len()`.
    pub total: u32,
}

/// One run's full state, as shown to a client.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RunInfo {
    pub run_id: String,
    pub goal: String,
    pub project: PathBuf,
    pub root: PathBuf,
    pub state: RunState,
    pub paused_from: Option<RunState>,
    pub halted_reason: Option<String>,
    /// `"user"` or `"--yes"`.
    pub approved_by: Option<String>,
    pub base_branch: String,
    pub base_sha: String,
    pub run_branch: String,
    pub run_head: String,
    pub base_moved: Option<BaseMovedInfo>,
    pub revision: u64,
    pub max_writers: u8,
    pub max_readers: u8,
    pub max_bounces: u8,
    pub writers_busy: u8,
    pub readers_busy: u8,
    pub unverified: bool,
    /// Decision 54; reported `false` when the sandbox could not be enabled.
    pub worker_sandbox: bool,
    /// M8a final fix batch F1c round 2: this run's checks, proofs and `setup` run
    /// unconfined, because the daemon's platform cannot confine them and the user
    /// allowed it (`--unconfined-checks` or `[orchestrator] unconfined_checks`).
    #[serde(default)]
    pub unconfined_checks: bool,
    /// Decision 53: the project settings `--trust-project` accepted.
    pub trusted_project: Vec<String>,
    /// Per runtime label, for M9.5.
    pub rate_limits: BTreeMap<String, u32>,
    pub tasks: Vec<TaskInfo>,
    pub critical_path: Vec<String>,
    pub attention: Vec<String>,
    pub report_path: PathBuf,
    pub outcome: Option<String>,
    pub created_at: u64,
}

/// Every run the daemon knows about, at one revision.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RunsSnapshot {
    pub revision: u64,
    pub runs: Vec<RunInfo>,
}
