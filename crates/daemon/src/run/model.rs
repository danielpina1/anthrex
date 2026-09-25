//! Pure engine model types. No `std::fs`, `std::process`, `std::thread`, `tokio` or
//! `std::time::SystemTime` — design decision 2.
//!
//! M8a.4 added [`ReviewLevel`] (refresh note C41); M8a.5 adds the rest of the model the
//! Interfaces list for `run/model.rs`: [`Profile`], [`RunLimits`], [`Task`], [`Run`] and
//! the per-round records a task carries. M8a.11 adds [`PendingOp`] and `Run.pending_ops`
//! (they hold the engine's `OpKind`), `Task.worktree_live`, `Task.awaiting_deps`,
//! `Task.held_answered` and `RunLimits.api_key_helper`.

use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

use proto::{
    AgentRole, BlockInfo, Budget, DoneSignal, Finding, GateCounts, ModelEntry, PlanTask, Route,
    RunState, Runtime, Size, Spend, TaskState, TestMode, TokenUsage, Verdict,
};
use serde::{Deserialize, Serialize};

use super::engine::OpKind;

/// How thoroughly a task is reviewed, decision 35: `S` tasks get `Small`, `M` tasks
/// `Medium`, hub tasks `Frontier`, each possibly raised by the level rule (no `check` in
/// the profile, or a non-`tdd` task whose `owns` touch `source`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ReviewLevel {
    Small,
    Medium,
    Frontier,
}

impl ReviewLevel {
    /// One level up, capped at `Frontier`.
    pub fn raised(self) -> ReviewLevel {
        match self {
            ReviewLevel::Small => ReviewLevel::Medium,
            ReviewLevel::Medium | ReviewLevel::Frontier => ReviewLevel::Frontier,
        }
    }
}

/// An engine operation's id.
pub type OpId = u64;

/// The resolved project profile, decision 7: each key from the plan's `[profile]`, else
/// `[orchestrator.profile]`, else empty; `protected` is the one key that merges
/// (built-ins + config + plan, decision 56). A blank `check`, `single_test`,
/// `test_passed` or `setup` resolves to `None`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Profile {
    pub modules: Vec<String>,
    pub hub: Vec<String>,
    pub source: Vec<String>,
    pub check: Option<String>,
    pub check_timeout_secs: u64,
    pub single_test: Option<String>,
    pub test_passed: Option<String>,
    pub setup: Option<String>,
    pub generated: Vec<String>,
    pub protected: Vec<String>,
    pub env: BTreeMap<String, String>,
    /// Directories a confined check or proof may also write (final fix batch F1c, I2;
    /// `run::confine`). Absent from a run recorded before F1c: none.
    #[serde(default)]
    pub cache_dirs: Vec<String>,
    /// Whether confined checks, proofs and `setup` have the network (final fix batch
    /// F1d, R4): the user's own `[orchestrator.confined_network]` for the repository,
    /// never a plan's. Absent from a run recorded before F1d: off.
    #[serde(default)]
    pub confined_network: bool,
    /// The Unix sockets confined commands may connect to (F1d round 2, S1): the user's
    /// own `[orchestrator.confined_unix_sockets]` for the repository.
    #[serde(default)]
    pub confined_unix_sockets: Vec<String>,
    /// The loopback ports confined commands may use (F1d round 2, S1/S2): the user's
    /// own `[orchestrator.confined_localhost_ports]` for the repository.
    #[serde(default)]
    pub confined_localhost_ports: Vec<u16>,
}

/// `[orchestrator.claude] auth`, mirrored here with serde because `config::ClaudeAuth`
/// has no serde derive (the config crate does not depend on serde) and [`RunLimits`] is
/// persisted with the run.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ClaudeAuth {
    #[default]
    Login,
    ApiKey,
}

impl From<config::ClaudeAuth> for ClaudeAuth {
    fn from(auth: config::ClaudeAuth) -> Self {
        match auth {
            config::ClaudeAuth::Login => ClaudeAuth::Login,
            config::ClaudeAuth::ApiKey => ClaudeAuth::ApiKey,
        }
    }
}

impl From<ClaudeAuth> for config::ClaudeAuth {
    fn from(auth: ClaudeAuth) -> Self {
        match auth {
            ClaudeAuth::Login => config::ClaudeAuth::Login,
            ClaudeAuth::ApiKey => config::ClaudeAuth::ApiKey,
        }
    }
}

/// The limits a run is frozen with at start: `[orchestrator]`, with the plan's
/// `max_writers`, `max_readers` and `max_bounces` winning when set.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RunLimits {
    pub max_writers: u8,
    pub max_readers: u8,
    pub max_bounces: u8,
    pub max_tasks: u32,
    pub max_windows: u32,
    pub default_runtime: Runtime,
    pub review_small: bool,
    pub budget_s: Budget,
    pub budget_m: Budget,
    pub budget_l: Budget,
    pub stall_after_secs: u64,
    pub rate_limit_retry_secs: u64,
    pub denials_before_block: u32,
    pub git_timeout_secs: u64,
    pub worker_permission_mode: String,
    pub worker_allowed_tools: Vec<String>,
    pub worker_codex_sandbox: String,
    pub worker_sandbox: bool,
    /// M8a final fix batch F1c round 2: checks, proofs and `setup` run unconfined
    /// (the platform cannot confine them, and the user allowed it at `run start`).
    /// Absent from a run recorded before: `false`.
    #[serde(default)]
    pub unconfined_checks: bool,
    pub claude_auth: ClaudeAuth,
    /// `[orchestrator.claude] api_key_helper`, passed to Claude sessions under
    /// `auth = "api_key"` (decision 50). Added by M8a.11: a session spec is built from
    /// the run alone.
    #[serde(default)]
    pub api_key_helper: Option<String>,
}

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

/// A resolved task: the planner's spec plus everything decisions 8–10 and 35 derive
/// from it, and the engine's running state.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Task {
    pub spec: PlanTask,
    pub size: Size,
    pub hub: bool,
    pub test_mode: TestMode,
    pub notes: Vec<String>,
    /// `None`: not reviewed (`review.small = "off"` on a non-hub `S` task).
    pub review_level: Option<ReviewLevel>,
    pub route: Route,
    pub review_route: Option<Route>,
    pub budget: Budget,
    pub implicit_deps: Vec<String>,
    pub state: TaskState,
    pub block: Option<BlockInfo>,
    pub rung: u8,
    /// The size rung 3 raised the task to (decision 38), set by the engine when it
    /// raises. An amend never leaves the task below it (M8a.6 fix round 2).
    #[serde(default)]
    pub raised_size: Option<Size>,
    pub failures: u8,
    pub bounces: GateCounts,
    pub stalls: u8,
    pub budget_exceeded: u8,
    pub conflicts: u8,
    pub session: u32,
    pub spent_total: Spend,
    pub branch: String,
    pub worktree: PathBuf,
    /// The worktree was created while the plan gate was open (decision 14), from
    /// `base_sha`, and its `setup` succeeded.
    pub prewarmed: bool,
    /// The task's worktree exists: set when a `PrepareWorktree` succeeds or its setup
    /// fails, cleared when it is removed (M8a.11; the cancel clean-up of M8a.6's F5).
    #[serde(default)]
    pub worktree_live: bool,
    /// An answer or other message is held for this started task until every dependency
    /// has finished (M8a.6 ruling N5); the task stays `blocked` meanwhile.
    #[serde(default)]
    pub awaiting_deps: bool,
    /// The held task was answered: it resumes once handed back. Without an answer
    /// (only an amendment waits, say) it goes back to its question instead (M8a.11 fix
    /// round 2, ruling T11-N2).
    #[serde(default)]
    pub held_answered: bool,
    /// M8a.13: the `Proof`, `Check` or `PrepareReview` op whose result the task awaits;
    /// any other result of those kinds is dropped (ruling T12-N's correlation).
    #[serde(default)]
    pub gate_op: Option<OpId>,
    /// M8a.13: review rounds in a row that ended without a verdict (decision 35); the
    /// second blocks the task, and a verdict resets it.
    #[serde(default)]
    pub review_misses: u8,
    /// M8a.14: the `MergeCandidate`, or the merge queue's `HandBack` (decision 36), the
    /// task awaits; any other result of those kinds for it is dropped (ruling T12-N's
    /// correlation). An N5 `HandBack` (`holds.rs`) never sets it.
    #[serde(default)]
    pub merge_op: Option<OpId>,
    /// M8a.14 fix round 1 (ruling T14-I1): a cancel arrived while the task's
    /// `MergeCandidate` ran. It applies when that merge does not land; a merge that
    /// lands makes the task `merged` and the cancel too late.
    #[serde(default)]
    pub cancel_deferred: bool,
    /// Ruling T14-I2: the worktree a dispatch prepared from this commit came back while
    /// the run was not running; the worker is launched (or the worktree re-pointed) by
    /// the first running pass.
    #[serde(default)]
    pub ready_from: Option<String>,
    /// Ruling T14-I3: the worker was told of a conflict the merge queue handed back,
    /// and resolves it in its worktree (a merge in progress) until its next accepted
    /// `task_done`.
    #[serde(default)]
    pub resolving: bool,
    /// Ruling T14-I3: its dependencies finished while it was resolving that conflict;
    /// the run head is handed back at its next accepted `task_done`, before any gate.
    #[serde(default)]
    pub handback_due: bool,
    /// Ruling T14-I3: the hand-back in flight is that due one: a clean result goes
    /// through the gates, not straight to the merge queue.
    #[serde(default)]
    pub gates_after_handback: bool,
    /// Ruling T14-R2: the conflicted hand-back `handed_back` refers to. A claim that is
    /// only its resolution goes straight back to the merge queue.
    #[serde(default)]
    pub resolution: Option<super::engine::ResolutionAt>,
    /// M8a.15: `run override` of a blocked task no claim recorded a head for, waiting
    /// for its `CountCommits` (decision 35).
    #[serde(default)]
    pub override_count: Option<super::engine::OverrideCount>,
    /// The task's clock (rulings T15-I2, T15-I3, T15-R2).
    #[serde(default)]
    pub clock: super::engine::TaskClock,
    /// The spend before the last `run retry`; rung 4 counts from it (ruling T15-C1).
    #[serde(default)]
    pub epoch: Option<super::engine::BudgetEpoch>,
    pub start_commit: Option<String>,
    pub head: Option<String>,
    pub done: Option<DoneClaim>,
    /// The claim being verified (decision 32; M8a.12).
    #[serde(default)]
    pub claim: Option<PendingClaim>,
    /// A fresh session waiting to start (M8a.12).
    #[serde(default)]
    pub fresh_session: Option<FreshSession>,
    pub rounds: Vec<AgentRound>,
    pub reviews: Vec<ReviewRecord>,
    pub checks: Vec<CheckRecord>,
    pub proofs: Vec<ProofRecord>,
    pub handed_back: bool,
    pub merge_commit: Option<String>,
    pub merged_without_approval: Option<String>,
    pub salvage_refs: Vec<String>,
    pub failure_log: Vec<String>,
    pub history: Vec<TaskEvent>,
}

impl Task {
    pub fn id(&self) -> &str {
        &self.spec.id
    }
}

/// An engine operation that has been emitted and whose result has not come back
/// (decision 43).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PendingOp {
    pub op: OpId,
    pub task_id: Option<String>,
    pub kind: OpKind,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Outgoing {
    pub id: u64,
    pub window_id: u32,
    pub task_id: String,
    pub text: String,
    pub queued_at: u64,
    pub delivered_at: Option<u64>,
}

/// At most 500 per run.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LogEntry {
    pub at: u64,
    pub text: String,
}

/// Decision 21: the base branch advanced while the run went on.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BaseMoved {
    pub from: String,
    pub to: String,
    pub commits: u32,
    pub seen_at: u64,
}

/// One orchestration run.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Run {
    pub id: String,
    pub goal: String,
    pub root: PathBuf,
    pub project: PathBuf,
    pub git_common_dir: PathBuf,
    pub wt_dir: PathBuf,
    /// The run's own data directory, `<data_dir>/runs/<id>`.
    pub data_dir: PathBuf,
    pub base_branch: String,
    pub base_sha: String,
    pub run_head: String,
    pub last_green_candidate: Option<String>,
    #[serde(default)]
    pub base_moved: Option<BaseMoved>,
    pub state: RunState,
    pub paused_from: Option<RunState>,
    pub halted_reason: Option<String>,
    pub approved_by: Option<String>,
    pub profile: Profile,
    pub limits: RunLimits,
    pub roster: Vec<ModelEntry>,
    pub tasks: Vec<Task>,
    pub merge_queue: Vec<String>,
    pub outbox: Vec<Outgoing>,
    pub next_message: u64,
    #[serde(default)]
    pub pending_ops: BTreeMap<OpId, PendingOp>,
    pub next_op: OpId,
    pub windows_created: u32,
    pub revision: u64,
    pub unverified: bool,
    pub final_check_failed: bool,
    pub trusted_project: Vec<String>,
    /// Tracked files at `base_sha` matching `profile.protected` (decision 56).
    pub protected_files: Vec<String>,
    pub rate_limits: BTreeMap<String, u32>,
    pub outcome: Option<String>,
    pub log: Vec<LogEntry>,
    pub created_at: u64,
    /// M8a.14: decision 37's `finish` edit: nothing new starts; done once live tasks end.
    #[serde(default)]
    pub finish_edit: bool,
    /// M8a.14: the accept or discard request its in-flight op answers; a restore clears it.
    #[serde(default)]
    pub finish_reply: Option<u64>,
    /// M8a.14 fix round 1: `run cancel` applied; halted, it discards with no rebaseline.
    #[serde(default)]
    pub cancelled: bool,
    /// Review m1: `VerifyRefs` failures in a row; the second halts (retryable).
    #[serde(default)]
    pub verify_failures: u8,
    /// Review m1: halted over unreadable refs, so `run resume` needs no `--rebaseline`.
    #[serde(default)]
    pub halt_retryable: bool,
    /// M8a.15: when a daemon restart ended the sessions; the next resume resumes them.
    #[serde(default)]
    pub restored: Option<u64>,
    /// M8a.22: drawn at start, mixed into session uuids (`role_launch::session_uuid_of`).
    #[serde(default)]
    pub session_nonce: u64,
    #[serde(default)] // M8a.23, ruling T23-C1: decision 53's Codex branch at start.
    pub codex_project_config: Option<crate::headless::argv::CodexProjectConfig>,
    /// Final fix batch F2 (C-I1): `base_sha`'s `.codex` entries, read at `run start`;
    /// every Codex session's checkout must match them (`headless::codex_guard`).
    #[serde(default)]
    pub codex_config_base: Vec<crate::headless::codex_guard::GuardEntry>,
}

impl Run {
    /// `anthrex/<id>/integration`.
    pub fn run_branch(&self) -> String {
        format!("anthrex/{}/integration", self.id)
    }

    /// `<wt_dir>/runs/<id>/integration`.
    pub fn integration_path(&self) -> PathBuf {
        self.task_path("integration")
    }

    /// `<wt_dir>/runs/<id>/<task>`.
    pub fn task_path(&self, task: &str) -> PathBuf {
        task_path(&self.wt_dir, &self.id, task)
    }

    pub fn review_path(&self, task: &str) -> PathBuf {
        self.task_path(&format!("{task}.review"))
    }

    pub fn proof_path(&self, task: &str) -> PathBuf {
        self.task_path(&format!("{task}.proof"))
    }

    /// `<data_dir>/REPORT.md`.
    pub fn report_path(&self) -> PathBuf {
        self.data_dir.join("REPORT.md")
    }

    /// The run id's 4 hex digits.
    pub fn short(&self) -> &str {
        let cut = self.id.len().saturating_sub(4);
        self.id.get(cut..).unwrap_or(&self.id)
    }

    pub fn task(&self, id: &str) -> Option<&Task> {
        self.tasks.iter().find(|t| t.spec.id == id)
    }
}

/// `anthrex/<run>/<task>`.
pub fn task_branch(run_id: &str, task: &str) -> String {
    format!("anthrex/{run_id}/{task}")
}

/// `<wt_dir>/runs/<run>/<task>`.
pub fn task_path(wt_dir: &std::path::Path, run_id: &str, task: &str) -> PathBuf {
    wt_dir.join("runs").join(run_id).join(task)
}
