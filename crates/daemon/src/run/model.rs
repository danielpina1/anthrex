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
    RunState, Size, Spend, TaskState, TestMode, TokenUsage, Verdict,
};
use serde::{Deserialize, Serialize};

use super::engine::OpKind;

// A session round and the per-task gate records (split out to keep this file under
// the 600-line rule, F4); re-exported, so every `model::` path stays.
#[path = "model_rounds.rs"]
mod rounds;
pub use rounds::*;

// The limits a run is frozen with and their Claude auth (split out to keep this file
// under the 600-line rule); re-exported, so every `model::` path stays.
#[path = "model_limits.rs"]
mod limits;
pub use limits::{ClaudeAuth, RunLimits, TestingLimits};

// Milestone 8b's additions (M8b decision 1). M8b.4 adds only `impl Run` items; the
// structs its later tasks add there are re-exported here with `pub use adapt::*`.
#[path = "model_adapt.rs"]
mod adapt;
pub use adapt::*;

// Milestone 9.1's stage types (decisions 46–53), kept out of this file's budget.
#[path = "model_stages.rs"]
mod stages;
pub use stages::*;

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
    /// Milestone 9.1 decision 5: the tier keys, all off in a run recorded before them.
    #[serde(default)]
    pub tiers: super::tiers::TierProfile,
    /// A stored profile's `manifests` (M8b), which key a command graph's cache
    /// (milestone 9.1 ruling C-12a); none for a plan's or config's profile.
    #[serde(default)]
    pub manifests: Vec<String>,
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
    /// M8b decision 20: a failed check's rung, deferred until its summary is decided.
    #[serde(default)]
    pub pending_failure: Option<PendingFailure>,
    /// M8b decision 21: a free-text `task_blocked` waits for the classification of this
    /// decider; any other decider's answer, or one after a retry, an override or a
    /// typed block, is not applied.
    #[serde(default)]
    pub pending_classification: Option<u64>,
    /// M8b decision 21: who classified the block (`None`: the worker typed its kind).
    #[serde(default)]
    pub block_source: Option<proto::DeciderSource>,
    /// M8b decision 18: the usage of the deciders asked about this task alone.
    #[serde(default)]
    pub decider_usage: TokenUsage,
    /// M8b decision 19: the size cross-check; a pending one keeps the task from
    /// being dispatched.
    #[serde(default)]
    pub size_check: Option<SizeCheckState>,
    /// M8b decision 31: seconds in each state, and when the current one began (0: a
    /// task from before milestone 8b, whose open state counts nowhere).
    #[serde(default)]
    pub phases: proto::PhaseSecs,
    #[serde(default)]
    pub phase_since: u64,
    /// M8b decision 31: the highest rung the task reached.
    #[serde(default)]
    pub max_rung: u8,
    /// M8b decision 32: what the task changed, measured by diff.
    #[serde(default)]
    pub diff: Option<proto::DiffStats>,
    /// M8b decision 33: its `history.jsonl` record was emitted.
    #[serde(default)]
    pub history_written: bool,
    /// M8b decision 33a: every route chosen for a session of this task, in order.
    #[serde(default)]
    pub routing_decisions: Vec<proto::RoutingDecision>,
    /// M8b decision 33a: the route rung 2 or `run retry` escalated from; the next
    /// worker launch records that escalation and clears it.
    #[serde(default)]
    pub escalated_from: Option<Route>,
    /// Milestone 9's task state (`run::orch::TaskOrch`).
    #[serde(default)]
    pub orch: super::orch::TaskOrch,
}

impl Task {
    pub fn id(&self) -> &str {
        &self.spec.id
    }

    /// Milestone 9.1 decision 43: the task's stage.
    pub fn stage(&self) -> u16 {
        self.spec.stage
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
    /// Whether the run started with `--trust-project` (M9.13 review; decision 9's
    /// `run promote` honours it). A run from before it: false.
    #[serde(default)]
    pub trust_project: bool,
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
    /// M8a.23, ruling T23-C1: decision 53's Codex branch at start.
    #[serde(default)]
    pub codex_project_config: Option<crate::headless::argv::CodexProjectConfig>,
    /// Final fix batch F2 (C-I1): `base_sha`'s `.codex` entries, read at `run start`;
    /// every Codex session's checkout must match them (`headless::codex_guard`).
    #[serde(default)]
    pub codex_config_base: Vec<crate::headless::codex_guard::GuardEntry>,
    /// M8b decision 6: where the profile came from; `None` for a run from milestone 8a.
    #[serde(default)]
    pub profile_source: Option<proto::ProfileSource>,
    /// M8b decision 28: a stored profile's filter settings, else the defaults.
    #[serde(default)]
    pub output_filter: proto::OutputFilter,
    #[serde(default)]
    pub filter_prefixes: Vec<String>,
    /// M8b decision 4: the repository's data directory (empty: a run from milestone 8a).
    #[serde(default)]
    pub repo_dir: PathBuf,
    /// M8b decision 7: files changed since the stored profile was confirmed.
    #[serde(default)]
    pub stale_profile: Vec<String>,
    /// M8b decision 18: deciders waiting for a reader slot, oldest first.
    #[serde(default)]
    pub decider_queue: Vec<QueuedDecider>,
    #[serde(default)]
    pub next_decider: u64,
    /// M8b decision 18: every `Decided`, and those that were fallbacks.
    #[serde(default)]
    pub decider_calls: u32,
    #[serde(default)]
    pub decider_fallbacks: u32,
    #[serde(default)]
    pub decider_usage: TokenUsage,
    /// M8b decision 19: the ids of the run's scout reports (stored under
    /// `<data_dir>/scouts/`), the evidence a task's `scout_refs` may name. Empty until
    /// milestone 9 starts run scouts.
    #[serde(default)]
    pub scout_reports: Vec<String>,
    /// M8b decision 19: the stored profile's onboarding report id (the `scout_refs`
    /// alias `onboarding`), when the repository has one.
    #[serde(default)]
    pub onboarding_report: Option<String>,
    /// M8b decision 22: `Some(Fast)` for a fast-path run, with what triage decided and
    /// its usage (decision 29); `None` for a run from a plan file.
    #[serde(default)]
    pub path: Option<proto::RunPath>,
    #[serde(default)]
    pub triage: Option<proto::TriageInfo>,
    #[serde(default)]
    pub triage_usage: TokenUsage,
    /// M8b decision 29: run scouts' usage (milestone 9 starts them), and the
    /// orchestrator's, the OTLP ledger's latest total (decision 30).
    #[serde(default)]
    pub scout_usage: TokenUsage,
    #[serde(default)]
    pub orchestrator_usage: TokenUsage,
    /// M8b.15 review (I5): the `orchestrator_usage` this daemon restored, which the
    /// OTLP ledger's totals add to, so a restart never lowers it. Set at restore, never
    /// stored: the next restore takes the stored usage again.
    #[serde(skip)]
    pub orchestrator_base: TokenUsage,
    /// M8b decision 25: when the user asked to promote this fast-path run (milestone 9
    /// performs it).
    #[serde(default)]
    pub promote_requested_at: Option<u64>,
    /// M8b decision 33: the run's `run` record of `history.jsonl` was emitted.
    #[serde(default)]
    pub run_record_written: bool,
    /// M8b decision 33a: the stored profile's languages, frozen at start (empty with no
    /// stored profile), for every routing decision's input.
    #[serde(default)]
    pub profile_languages: Vec<String>,
    /// Milestone 9 decision 43: role-routing records not yet appended to the history.
    #[serde(default)]
    pub role_routing_decisions: Vec<proto::RoleRoutingDecision>,
    /// M8b decision 33: the run was started with history (milestone 8b.16 on). A run
    /// started before has no phases, diffs or routing decisions, and writes none.
    #[serde(default)]
    pub history: bool,
    /// M8c: when the plan was approved (the user, `--yes` or the fast path).
    #[serde(default)]
    pub approved_at: Option<u64>,
    /// M8c: accepted plan-edit batches, oldest first, at most `edit_log::PLAN_EDITS_KEPT`.
    #[serde(default)]
    pub plan_edits: Vec<super::edit_log::PlanEditRecord>,
    /// M8c: accepted plan-edit batches since the plan was approved.
    #[serde(default)]
    pub plan_edits_since_approval: u32,
    /// Milestone 9's run state (`run::orch::RunOrch`).
    #[serde(default)]
    pub orch: super::orch::RunOrch,
    /// Milestone 9.1 decision 11: the frozen profile's hash, a result-cache key part.
    #[serde(default)]
    pub profile_hash: String,
    /// Milestone 9.1 decision 11: the toolchain id the run's first tier job read, a
    /// result-cache key part; `None` until then.
    #[serde(default)]
    pub toolchain: Option<String>,
    /// Milestone 9.1 decision 9: the note of the first unknown module graph the run
    /// met (`tiers::graph::note_once`), logged and reported once.
    #[serde(default)]
    pub graph_note: Option<String>,
    /// Milestone 9.1 decision 27: the daemon's `test_slots` (the workers' caps), which
    /// the driver stamps on a run it starts (`slots::stamp`) and on each run it loads at
    /// restore (ruling C-11); 0 before that.
    #[serde(default)]
    pub test_slots: u32,
    /// Milestone 9.1 decision 46: fixed when the plan is first approved.
    #[serde(default)]
    pub stage_layout: StageLayout,
    /// Milestone 9.1 decision 47: one record per created stage, lowest first. A run
    /// from before it gets stage 1 at restore (`engine::stages::ensure_first`).
    #[serde(default)]
    pub stages: Vec<StageRecord>,
}

impl Run {
    /// Whether `self` and `other` store the same `run.json`: equal in everything but
    /// the fields never stored (`orchestrator_base`, M8b.15 re-review minor 1).
    pub fn same_on_disk(&self, other: &Run) -> bool {
        if self.orchestrator_base == other.orchestrator_base {
            return self == other;
        }
        let mut rebased = self.clone();
        rebased.orchestrator_base = other.orchestrator_base;
        rebased == *other
    }

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

    /// Milestone 9.1 decision 47: stage `n`'s record, when it has been created.
    pub fn stage(&self, n: u16) -> Option<&StageRecord> {
        self.stages.iter().find(|s| s.n == n)
    }

    /// Stage `n`'s head: a `Single` run's one branch is `integration`, so every stage
    /// of it is `run_head`; a `Multi` run's is its record's, `None` until it is created.
    pub fn stage_head(&self, n: u16) -> Option<&str> {
        match self.stage_layout {
            StageLayout::Single => Some(&self.run_head),
            StageLayout::Multi => self.stage(n).map(|s| s.head.as_str()),
        }
    }

    /// The head task-context work starts from, merges into and is measured against:
    /// its stage's head (decision 47), `run_head` while that stage is not created.
    pub fn head_for(&self, task: &Task) -> &str {
        self.stage_head(task.stage()).unwrap_or(&self.run_head)
    }

    /// `integration` for a `Single` run, `anthrex/<run>/stage-<n>` for a `Multi` one.
    pub fn stage_branch(&self, n: u16) -> String {
        match self.stage_layout {
            StageLayout::Single => self.run_branch(),
            StageLayout::Multi => task_branch(&self.id, &format!("stage-{n}")),
        }
    }

    /// `<wt_dir>/runs/<id>/.full`, the tier-3 checkout (decision 17).
    pub fn full_path(&self) -> PathBuf {
        self.task_path(".full")
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
