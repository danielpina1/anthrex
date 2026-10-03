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

use proto::{AgentRole, DoneSignal, Finding, ModelEntry, Route, RunState, TokenUsage, Verdict};
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

// The resolved task (split out to keep this file under the 600-line rule).
#[path = "model_task.rs"]
mod task;
pub use task::*;

// Milestone 9.3's rounds (decision 18); `rounds` is the session rounds' module.
#[path = "model_goal_rounds.rs"]
mod goal_rounds;
pub use goal_rounds::Round;

// Milestone 9.5's race, pair and writer caps (decisions 16, 19, 20, 25; ruling I3).
#[path = "model_tuning.rs"]
mod tuning;
pub use tuning::*;

// `Run`'s lookups and path helpers, and the task branch and path (split out to keep
// this file under the 600-line rule, milestone 9.5); re-exported, so every `model::`
// path stays.
#[path = "model_paths.rs"]
mod paths;
pub use paths::{task_branch, task_path};

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

/// An engine operation that has been emitted and whose result has not come back
/// (decision 43).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PendingOp {
    pub op: OpId,
    pub task_id: Option<String>,
    pub kind: OpKind,
    /// Milestone 9.5 decision 20: the race lane the op is for.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lane: Option<proto::RaceLane>,
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
    /// W1 fix round 2 of milestone 9.3's final fix wave: `finish_edit` was set by a
    /// round's cancel (decision 16), not by the user's `finish`; the round's end
    /// clears it, and only such a finish is the round's alone.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub round_finish: bool,
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
    /// Milestone 9.1 decision 17: the one tier-3 job in flight (`engine::full`).
    #[serde(default)]
    pub full_op: Option<OpId>,
    /// Decision 17(b): since when the merge queue has been idle (tiered profiles only).
    #[serde(default)]
    pub queue_idle_since: Option<u64>,
    /// Milestone 9.1 decision 39: the next fix task's number (`engine::fixes`).
    #[serde(default)]
    pub fix_seq: u32,
    /// Milestone 9.1 decision 49: the stages due a propagate from the stage below
    /// (`engine::propagate`).
    #[serde(default)]
    pub propagate_due: BTreeSet<u16>,
    /// Milestone 9.2 decisions 3 and 16: how the run is delivered, frozen at start.
    #[serde(default)]
    pub delivery: super::delivery::RunDelivery,
    /// Milestone 9.3 decision 18: one record a round, oldest first. A run from before
    /// rounds gets round 1 at restore (`engine::goal_rounds::ensure_first`).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub rounds: Vec<Round>,
    /// Milestone 9.3 decision 19: the chain whose orchestrator session the run uses.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub chain: Option<String>,
    /// Milestone 9.3 D17: the run a next goal started from this run's chain, its
    /// orchestrator's current run since; this run iterates no more (decision 9).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub continued_by: Option<String>,
    /// Milestone 9.3's final fix wave (review A, M1): the run's chain left the table
    /// while this run was its current one (the project's older idle chain, dropped when
    /// a newer one went idle), so a restart's `chain::rebuild` leaves it out too.
    /// Cleared when the chain comes back (an iterate, D17).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub chain_left: bool,
    /// Milestone 9.5 decision 16: the writer cap per runtime label, once one is tracked.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub concurrency: BTreeMap<String, RuntimeConcurrency>,
}
