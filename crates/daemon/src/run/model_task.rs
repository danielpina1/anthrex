//! The resolved task of a run (split out of `model.rs` for the 600-line rule, milestone
//! 9.1 task M9.1.15; re-exported there, so every `model::Task` path stays). Pure data
//! (design decision 2).

use std::path::PathBuf;

use proto::{
    BlockInfo, Budget, GateCounts, PlanTask, Route, Size, Spend, TaskState, TestMode, TokenUsage,
};
use serde::{Deserialize, Serialize};

use super::{
    AgentRound, CheckRecord, DoneClaim, FreshSession, OpId, PendingClaim, PendingFailure,
    ProofRecord, ReviewLevel, ReviewRecord, SizeCheckState, TaskEvent,
};

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
    pub resolution: Option<crate::run::engine::ResolutionAt>,
    /// M8a.15: `run override` of a blocked task no claim recorded a head for, waiting
    /// for its `CountCommits` (decision 35).
    #[serde(default)]
    pub override_count: Option<crate::run::engine::OverrideCount>,
    /// The task's clock (rulings T15-I2, T15-I3, T15-R2).
    #[serde(default)]
    pub clock: crate::run::engine::TaskClock,
    /// The spend before the last `run retry`; rung 4 counts from it (ruling T15-C1).
    #[serde(default)]
    pub epoch: Option<crate::run::engine::BudgetEpoch>,
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
    pub orch: crate::run::orch::TaskOrch,
    /// Milestone 9.1 decision 39: who made the task; `plan` for every planned one.
    #[serde(default)]
    pub origin: proto::TaskOrigin,
    /// Decision 39: what an engine-made fix task fixes.
    #[serde(default)]
    pub fixes: Option<super::FixOf>,
    /// Decision 42: the accepted claim's test-weakening signals, numbered `W1…` in
    /// the reviewer prompt; replaced by each accepted claim.
    #[serde(default)]
    pub signals: Vec<crate::run::tiers::Signal>,
    /// How many signals past `SIGNALS_MAX` the claim had (the block's `… and <n> more`).
    #[serde(default)]
    pub signals_more: u32,
    /// Decision 42: `submit_review`s of the current review round refused for leaving
    /// a signal id out; the second is accepted with the engine's findings.
    #[serde(default)]
    pub signal_refusals: u8,
    /// Milestone 9.1 decision 51: a `sync` fix task's merge, handed back into its
    /// worktree before its first session.
    #[serde(default)]
    pub sync: Option<super::SyncState>,
    /// Milestone 9.3 decision 13: the round that added the task.
    #[serde(default = "proto::first_round")]
    pub round: u32,
    /// Milestone 9.5 decision 19: the race, once the task races.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub race: Option<super::Race>,
    /// Decision 25: the test writer and its red commit, once a paired task starts.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pair: Option<super::Pair>,
    /// Decision 18: since when the racing task has waited for its second writer slot.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub race_wait_since: Option<u64>,
    /// Decision 9a: the plan's model-list choice for its worker route (made when the
    /// task was built or added), when its class has a list.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub list_pick: Option<super::ListPick>,
    /// Decision 9a: rung 2's list step, beside `escalated_from`; the next worker launch
    /// records it and clears it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub list_escalation: Option<super::ListPick>,
    /// Milestone 9.5 ruling T12-2: the run's paused time in the task's phases.
    #[serde(default, skip_serializing_if = "super::TaskPaused::is_zero")]
    pub paused: super::TaskPaused,
}

impl Task {
    pub fn id(&self) -> &str {
        &self.spec.id
    }

    /// Milestone 9.1 decision 43: the task's stage.
    pub fn stage(&self) -> u16 {
        self.spec.stage
    }

    /// Milestone 9.5 ruling RR-1: the name of the task's checkout. The task id, or, once
    /// a lane of its race is crowned (`Won`) or adopted, that lane's `<task>.<lane>`:
    /// every path keyed by a checkout name (its repository, objects, engine directory,
    /// `TMPDIR`, proof and review checkouts) is then the lane's.
    pub fn checkout_name(&self) -> String {
        let crowned = self.race.as_ref().and_then(|race| {
            race.lanes
                .iter()
                .find(|l| matches!(l.state, proto::LaneState::Won | proto::LaneState::Adopted))
        });
        match crowned {
            Some(lane) => lane.checkout.clone(),
            None => self.id().to_string(),
        }
    }
}

/// Decision 19: lane `lane`'s checkout name, `<task>.<lane>`, which a `Lane` is made
/// with; every reader takes the stored `Lane.checkout` (review m4).
pub fn lane_checkout(task: &str, lane: proto::RaceLane) -> String {
    format!("{task}.{}", lane.label())
}

#[cfg(test)]
#[path = "checkout_name_tests.rs"]
mod checkout_name_tests;
