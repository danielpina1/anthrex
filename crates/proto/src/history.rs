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
use crate::tiers::TaskOrigin;

/// The `v` every record written now carries. Milestone 9 (decision 43) raised it from
/// 1 with the `role_route` line, milestone 9.1 (decision 57) to 3 with the `tier`,
/// `flaky` and `bisect` lines, milestone 9.2 (decision 44) to 4 with the `stage` line,
/// milestone 9.3 (KG §2.6) to 5 with the `round` line; older lines still decode
/// unchanged.
pub const HISTORY_VERSION: u32 = 5;

/// One line of `history.jsonl`, tagged `"type": "task" | "run" | "revert" |
/// "role_route" | "tier" | "flaky" | "bisect" | "stage" | "round"`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum HistoryLine {
    Task(TaskRecord),
    Run(RunRecord),
    Revert(RevertRecord),
    /// Milestone 9 decision 43.
    RoleRoute(RoleRoutingDecision),
    /// Milestone 9.1 decision 57.
    Tier(TierRunRecord),
    Flaky(FlakyRecord),
    Bisect(BisectLine),
    /// Milestone 9.2 decision 44.
    Stage(StageLine),
    /// Milestone 9.3 (KG §2.6).
    Round(crate::rounds::RoundLine),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskOutcome {
    Merged,
    MergedWithoutApproval,
    Cancelled,
    Blocked,
    Unfinished,
    /// Milestone 9 decisions 35 and 36: a research or review task delivered its report
    /// or verdict; it finished and merged nothing.
    Reported,
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

/// Decision 43: the route chosen for an agent session that belongs to no task (the
/// orchestrator, a sub-planner, a run scout, a decider), with the full ordered candidate
/// snapshot. `candidates[selected_index].route == chosen`. An unchosen candidate is
/// never a failure; `outcome` is the session's own, set when it ends.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RoleRoutingDecision {
    pub v: u32,
    /// `<run id>/<role>/<session id>`, or `triage/<unix nanos>/<n>` for pre-run triage
    /// (milestone 9 task M9.13b).
    pub record_id: String,
    pub at: u64,
    /// `None` for pre-run triage.
    #[serde(default)]
    pub run_id: Option<String>,
    #[serde(default)]
    pub task_id: Option<String>,
    pub role: AgentRole,
    pub session_id: String,
    /// `start`, `restart`, `replan`, `retry`, `triage`, `size_check`, …
    pub trigger: String,
    /// `explicit_choice`, `agent_config`, `planner_config`, `roster_default`,
    /// `scout_config` or `decider_config`.
    pub source: String,
    /// `m9-orchestrator-v1`, `m9-planner-v1`, `m9-scout-v1` or `m9-decider-v1`.
    pub policy_version: String,
    /// `None` until milestone 9.5's role lists apply.
    #[serde(default)]
    pub pick_policy: Option<String>,
    pub input: RoleRoutingInput,
    pub chosen: Route,
    pub selected_index: u32,
    pub candidates: Vec<RoutingCandidate>,
    /// `None` until the session ends.
    #[serde(default)]
    pub outcome: Option<RoleOutcome>,
    /// The role's accepted/rejected, report or submission status.
    #[serde(default)]
    pub result: Option<String>,
}

/// What a role session was dispatched for (decision 43). Never transcripts,
/// credentials or raw tool output.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RoleRoutingInput {
    #[serde(default)]
    pub run_path: Option<RunPath>,
    /// Capped as `RunRecord.goal` is.
    #[serde(default)]
    pub goal: Option<String>,
    #[serde(default)]
    pub languages: Vec<String>,
    #[serde(default)]
    pub epic: Option<String>,
    #[serde(default)]
    pub area: Vec<String>,
    #[serde(default)]
    pub question_kind: Option<String>,
}

/// A role session's factual outcome (decision 43).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RoleOutcome {
    Completed,
    Failed,
    Interrupted,
    Fallback,
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
    /// Milestone 9.1; `0` on a line written before stages.
    #[serde(default)]
    pub stage: u16,
    #[serde(default)]
    pub origin: TaskOrigin,
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
    /// Milestone 9.1 decision 34: tests proposed for quarantine, and the window read.
    #[serde(default)]
    pub flaky_proposals: Vec<FlakyProposal>,
    #[serde(default)]
    pub window_days: u32,
    #[serde(default)]
    pub quarantine_after: u32,
    /// Milestone 9.3 (KG §2.6): the `round` lines read, and the runs they belong to.
    #[serde(default)]
    pub rounds: u32,
    #[serde(default)]
    pub iterated_runs: u32,
}

/// One tier job (decision 57), `record_id` `<run>/tier/<op>`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TierRunRecord {
    pub v: u32,
    pub record_id: String,
    pub at: u64,
    pub run_id: String,
    pub task_id: Option<String>,
    pub stage: u16,
    pub tier: u8,
    pub secs: u64,
    /// Modules in the affected set; 0 for build only and for the full suite.
    pub affected: u32,
    pub full_reason: Option<String>,
    pub cache_hit: bool,
    pub cached_steps: u8,
    pub steps: u8,
    pub ok: bool,
    pub flaky: Vec<String>,
}

/// A test that failed and passed on its retry (TT §3.6, with M8b's `type` tag),
/// `record_id` `<run>/flaky/<op>/<test>`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FlakyRecord {
    pub v: u32,
    pub record_id: String,
    pub at: u64,
    pub run_id: String,
    pub task_id: Option<String>,
    pub tier: u8,
    pub test: String,
}

/// One bisect of a red full suite, `record_id` `<run>/bisect/<stage>/<n>`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BisectLine {
    pub v: u32,
    pub record_id: String,
    pub at: u64,
    pub run_id: String,
    pub stage: u16,
    pub head: String,
    pub tests: Vec<String>,
    pub range: u32,
    pub probes: u32,
    pub culprit: Option<String>,
    pub reason: Option<String>,
    pub fix_task: Option<String>,
}

/// A flaky test proposed for quarantine (decision 34).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FlakyProposal {
    pub test: String,
    pub runs: u32,
    pub last_at: u64,
}

/// One stage pull request's end (milestone 9.2 decision 44): appended once, when it is
/// merged or closed, or when a `pr`-mode run is cancelled with it open, keyed
/// `"<run>/stage/<n>"`. Named like [`BisectLine`]; the daemon's
/// `run::model_stages::StageRecord` is a different type.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StageLine {
    pub v: u32,
    pub record_id: String,
    pub run_id: String,
    pub stage: u16,
    pub pr: u64,
    /// From the stage's tasks all finishing to its PR opening.
    pub time_to_open_secs: u64,
    pub human_review_secs: u64,
    /// The stage's `ci` fix tasks.
    pub ci_rounds: u32,
    pub review_rounds: u32,
    pub sync_tasks: u32,
    pub outcome: crate::delivery::StageOutcome,
    pub merge_method: crate::delivery::MergeMethod,
    pub at: u64,
}
