//! Run history (milestone 8b decision 33): one line of `<repo_dir>/history.jsonl` per
//! finished task, finished run or detected revert, and what `anthrex run stats` reports.
//!
//! No type here uses `deny_unknown_fields`, so milestone 9.5 can add fields and an
//! older reader still reads the line.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::adapt::{DiffStats, PhaseSecs, RunPath, RunUsage, SizeCheckInfo, TriageInfo};
use crate::profile::ProfileSource;
use crate::run::{AgentRole, BlockReason, DoneSignal, GateCounts, Route, Size, TaskKind, TestMode};
use crate::run_info::TokenUsage;

/// The `v` every record written by this milestone carries.
pub const HISTORY_VERSION: u32 = 1;

/// One line of `history.jsonl`, tagged `"type": "task" | "run" | "revert"`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum HistoryLine {
    Task(TaskRecord),
    Run(RunRecord),
    Revert(RevertRecord),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskOutcome {
    Merged,
    MergedWithoutApproval,
    Cancelled,
    Blocked,
    Unfinished,
}

/// How often each gate ran and failed for one task.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct GateTally {
    pub proofs: u32,
    pub proofs_failed: u32,
    pub checks: u32,
    pub checks_failed: u32,
    pub review_rounds: u32,
    pub reviews_rejected: u32,
    pub candidates_red: u32,
    pub generated_bounces: u32,
}

/// Review findings by severity.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SeverityTally {
    pub critical: u32,
    pub important: u32,
    pub minor: u32,
}

/// One route a routing decision could have chosen (spec §15, M8b decision 33a), with
/// why it was not, when it was not.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RoutingCandidate {
    pub route: Route,
    pub skipped_reason: Option<String>,
}

/// What the task looked like when a route was chosen for it (decision 33a): the input a
/// future router may learn from. Never rewritten by a later edit.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RoutingInput {
    pub title: String,
    pub brief: String,
    pub acceptance: Vec<String>,
    pub owns: Vec<String>,
    pub kind: TaskKind,
    pub size: Size,
    pub hub: bool,
    pub interface_change: bool,
    pub test_mode: TestMode,
    pub languages: Vec<String>,
}

/// One route chosen for a task-bound agent session (decision 33a), identified by
/// `(role, session, round, lane)`. `candidates[selected_index].route == chosen`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RoutingDecision {
    pub seq: u32,
    pub at: u64,
    pub role: AgentRole,
    pub session: u32,
    pub round: Option<u32>,
    pub lane: Option<String>,
    /// `initial`, `review` or `escalation` (M9.5 adds `race` and `test_writer`).
    pub trigger: String,
    /// `explicit_task`, `class_default`, `review_policy` or `escalation_policy`.
    pub source: String,
    /// `m8a-worker-v1`, `m8a-review-v1` or `m8a-escalate-v1`.
    pub policy_version: String,
    /// `None` until milestone 9.5's candidate lists apply.
    #[serde(default)]
    pub pick_policy: Option<String>,
    pub input: RoutingInput,
    pub chosen: Route,
    pub selected_index: u32,
    pub candidates: Vec<RoutingCandidate>,
}

/// One finished task.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TaskRecord {
    pub v: u32,
    pub record_id: String,
    pub at: u64,
    pub run_id: String,
    pub task_id: String,
    pub path: Option<RunPath>,
    pub kind: TaskKind,
    pub hub: bool,
    pub test_mode: TestMode,
    pub planned_size: Size,
    pub final_size: Size,
    pub size_check: Option<SizeCheckInfo>,
    pub route: Route,
    pub review_routes: Vec<Route>,
    /// Decision 33a: every route chosen for the task, in order. Absent from a line
    /// written before it: empty.
    #[serde(default)]
    pub routing_decisions: Vec<RoutingDecision>,
    pub outcome: TaskOutcome,
    pub block: Option<BlockReason>,
    pub diff: Option<DiffStats>,
    pub tool_calls: u32,
    pub worker_usage: TokenUsage,
    pub reviewer_usage: TokenUsage,
    pub decider_usage: TokenUsage,
    pub phases: PhaseSecs,
    pub wall_secs: u64,
    pub gates: GateTally,
    pub severities: SeverityTally,
    pub bounces: GateCounts,
    pub failures: u8,
    pub stalls: u8,
    pub budget_exceeded: u8,
    pub conflicts: u8,
    pub max_rung: u8,
    pub sessions: u32,
    pub done_signal: Option<DoneSignal>,
    pub merge_commit: Option<String>,
}

/// One finished run.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RunRecord {
    pub v: u32,
    pub record_id: String,
    pub at: u64,
    pub run_id: String,
    /// The first 200 characters.
    pub goal: String,
    pub path: Option<RunPath>,
    pub triage: Option<TriageInfo>,
    pub profile_source: Option<ProfileSource>,
    /// `"accepted"`, `"discarded"`, `"failed"` or `"complete"`.
    pub outcome: String,
    pub base_branch: String,
    pub accepted_commit: Option<String>,
    pub tasks: u32,
    pub usage: Option<RunUsage>,
}

/// A merged task's commit that was later reverted on the base branch.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RevertRecord {
    pub v: u32,
    pub record_id: String,
    pub at: u64,
    pub run_id: String,
    pub task_id: Option<String>,
    pub reverted: String,
    pub revert_commit: String,
}

/// One row of `anthrex run stats`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StatsRow {
    /// `"S"`, `"M"` or `"hub"`.
    pub class: String,
    pub tasks: u32,
    pub merged: u32,
    pub median_lines: Option<u32>,
    pub median_tool_calls: Option<u32>,
    pub median_tokens: Option<u64>,
    pub median_work_secs: Option<u64>,
    pub bounces: u32,
    pub reverted: u32,
}

/// `anthrex run stats`: recorded aggregates of one repository's history.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HistoryStats {
    pub path: PathBuf,
    pub task_records: u32,
    pub run_records: u32,
    pub rows: Vec<StatsRow>,
    pub decider_calls: u32,
    pub decider_fallbacks: u32,
    pub size_checked: u32,
    pub size_raised: u32,
    pub problems: Vec<String>,
}
