//! The read-only run snapshot: what `run list` and the TUI's run view are shown.
//!
//! An `Option` field tolerates absence (serde's derive reads a missing one as `None`).
//! A new non-`Option` field (a `Vec`, `bool` or number) needs `#[serde(default)]`, or
//! a protocol bump, or a snapshot from an older sender fails with "missing field"
//! (review E-M3, F4).

use std::collections::BTreeMap;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::adapt::{
    DeciderSource, DiffStats, PhaseSecs, RunPath, RunUsage, SizeCheckInfo, TriageInfo,
};
use crate::orch::{HoldInfo, IntegrationInfo, MessageKind, OrchestratorInfo, TaskNoteInfo};
use crate::planner::PlannerInfo;
use crate::profile::{ProfileSource, ProposalAlertInfo};
use crate::run::{
    AgentRole, BlockReason, Budget, DoneSignal, GateCounts, Route, RouteSpec, RunState, Size,
    TaskKind, TaskState, TestMode, Verdict,
};
pub use crate::run::{Finding, Severity};
use crate::scout::ScoutInfo;
use crate::tiers::{SignalInfo, StageInfo, TaskOrigin, TierInfo};

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

/// Field-wise and saturating (milestone 8b's usage by role): a sum never panics or
/// wraps, whatever a stream or an OTLP export reported.
impl std::ops::AddAssign for TokenUsage {
    fn add_assign(&mut self, other: Self) {
        self.input = self.input.saturating_add(other.input);
        self.output = self.output.saturating_add(other.output);
        self.cache_read = self.cache_read.saturating_add(other.cache_read);
        self.cache_write = self.cache_write.saturating_add(other.cache_write);
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
    /// Milestone 8b: the decider's ≤ 40-line summary; `summary` stays the raw tail.
    #[serde(default)]
    pub decider_summary: Option<String>,
    #[serde(default)]
    pub summary_source: Option<DeciderSource>,
    /// Milestone 9.1: the tier record, when the check was a tier job.
    #[serde(default)]
    pub tier: Option<TierInfo>,
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
    // Milestone 8c: unix seconds, formatted by the client.
    /// When the current rate limit began; `None` while not rate-limited.
    #[serde(default)]
    pub rate_limited_since: Option<u64>,
    /// The model's value: the round counts as rate-limited while it is in the future.
    #[serde(default)]
    pub rate_limited_until: Option<u64>,
    /// When a failed gate sent this worker session back (rung 1), oldest first.
    #[serde(default)]
    pub sent_back_at: Vec<u64>,
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
    /// The last 10 events, newest first (milestone 8c: raw times, was `"<hh:mm> <text>"`).
    #[serde(default)]
    pub history: Vec<TaskEventInfo>,
    // Milestone 8b.
    #[serde(default)]
    pub decider_usage: Option<TokenUsage>,
    #[serde(default)]
    pub size_check: Option<SizeCheckInfo>,
    #[serde(default)]
    pub diff: Option<DiffStats>,
    #[serde(default)]
    pub phases: Option<PhaseSecs>,
    #[serde(default)]
    pub block_source: Option<DeciderSource>,
    // Milestone 8c: the plan's own brief, acceptance and unresolved route (its `None`s
    // mean "policy"), for the plan gate's edit form.
    #[serde(default)]
    pub brief: String,
    #[serde(default)]
    pub acceptance: Vec<String>,
    #[serde(default)]
    pub route_spec: RouteSpec,
    // Milestone 9.
    /// The approval hold (decision 28) the task waits in, by id.
    #[serde(default)]
    pub hold: Option<String>,
    #[serde(default)]
    pub review_target: Option<String>,
    /// A research task's report size.
    #[serde(default)]
    pub research_bytes: Option<u32>,
    /// Decision 42d: every message the task was sent, the latest one's kind, and its
    /// first line (at most 80 characters).
    #[serde(default)]
    pub message_count: u32,
    #[serde(default)]
    pub last_message_kind: Option<MessageKind>,
    #[serde(default)]
    pub last_message_line: Option<String>,
    /// The task's last 10 `task_note`s (decisions 16a, 42d).
    #[serde(default)]
    pub task_notes: Vec<TaskNoteInfo>,
    // Milestone 9.1 decision 55.
    #[serde(default = "first_stage")]
    pub stage: u16,
    #[serde(default)]
    pub origin: TaskOrigin,
    /// What a fix task fixes, for example `bisect of t4`.
    #[serde(default)]
    pub fixes: Option<String>,
    #[serde(default)]
    pub tier: Option<TierInfo>,
    #[serde(default)]
    pub weakening: Vec<SignalInfo>,
    // Milestone 9.0.5.
    /// Decision 3: the live round's latest action, one line of at most
    /// `task_detail::ACTIVITY_MAX` characters; `None` without a live round.
    #[serde(default)]
    pub activity: Option<String>,
    /// Milestone 9.1 decision 54, for the plan review (ruling C-28 (5)): the task is its
    /// stage's one atomic hub, with the plan's reason, and it changes an interface.
    #[serde(default)]
    pub atomic: bool,
    #[serde(default)]
    pub atomic_reason: Option<String>,
    #[serde(default)]
    pub interface_change: bool,
}

fn first_stage() -> u16 {
    1
}

/// One task event (milestone 8c): its unix time and text.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TaskEventInfo {
    pub at: u64,
    pub text: String,
}

/// One plan-edit batch (milestone 8c): its unix time and what it did. Milestone 9
/// (decision 40) adds who sent it, whether it was accepted, and a message's resolved
/// recipients; an entry from milestone 8c was an accepted batch.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlanEditInfo {
    pub at: u64,
    pub text: String,
    /// `user`, `orchestrator` or `planner:<epic>`; `user` when absent (decision 40).
    #[serde(default = "user_by_default")]
    pub source: String,
    #[serde(default = "accepted_by_default")]
    pub accepted: bool,
    #[serde(default)]
    pub error: Option<String>,
    #[serde(default)]
    pub recipients: Vec<String>,
}

fn accepted_by_default() -> bool {
    true
}

fn user_by_default() -> String {
    "user".to_string()
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
    /// `"user"`, `"--yes"` or `"fast path"`.
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
    // Milestone 8b.
    #[serde(default)]
    pub path: Option<RunPath>,
    #[serde(default)]
    pub triage: Option<TriageInfo>,
    #[serde(default)]
    pub promote_requested_at: Option<u64>,
    #[serde(default)]
    pub profile_source: Option<ProfileSource>,
    #[serde(default)]
    pub usage: Option<RunUsage>,
    #[serde(default)]
    pub scouts: Vec<ScoutInfo>,
    // Milestone 8c.
    /// Unix seconds; when the plan was approved (by the user, `--yes` or the fast path).
    #[serde(default)]
    pub approved_at: Option<u64>,
    /// Accepted plan edits, newest first, at most 10.
    #[serde(default)]
    pub plan_edits: Vec<PlanEditInfo>,
    #[serde(default)]
    pub plan_edits_since_approval: u32,
    // Placeholders: milestone 9 fills `planners`, milestone 9.5 the two estimates.
    #[serde(default)]
    pub planners: Vec<PlannerInfo>,
    #[serde(default)]
    pub estimate_left_secs: Option<u64>,
    #[serde(default)]
    pub bound_ratio_permille: Option<u32>,
    // Milestone 9.
    #[serde(default)]
    pub orchestrator: Option<OrchestratorInfo>,
    #[serde(default)]
    pub holds: Vec<HoldInfo>,
    #[serde(default)]
    pub integration: Vec<IntegrationInfo>,
    /// Decision 16: bumped whenever `run_status`'s digest would change.
    #[serde(default)]
    pub digest_revision: u64,
    #[serde(default)]
    pub research_report: Option<PathBuf>,
    // Milestone 9.1 decision 55.
    #[serde(default)]
    pub stages: Vec<StageInfo>,
    #[serde(default)]
    pub test_slots: u32,
}

/// Every run the daemon knows about, at one revision.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RunsSnapshot {
    pub revision: u64,
    pub runs: Vec<RunInfo>,
    /// Milestone 8c: the daemon's unix seconds at publication, the clients' time base.
    #[serde(default)]
    pub now: u64,
    /// Milestone 9.0.5 decision 10: the profile proposals ready to confirm.
    #[serde(default)]
    pub proposals: Vec<ProposalAlertInfo>,
}
