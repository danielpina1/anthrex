//! What an engine operation returned (decision 43's journal carries it). Pure data
//! (design decision 2). Moved out of `ops.rs` (milestone 9.2's file budget) before
//! task M9.2.6 appends `OpResult::Host`.

use serde::{Deserialize, Serialize};

use crate::decider::Decision;

/// What an op returned.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum OpResult {
    Worktree {
        head: String,
    },
    SetupFailed {
        output: String,
    },
    Window {
        window_id: u32,
        /// The session's first process, whose own `ProcessStarted` came before any
        /// round had the window (M8a.25). `None` in a replay.
        #[serde(default)]
        pid: Option<u32>,
    },
    Resumed,
    ResumeFailed {
        error: String,
    },
    /// `run::git::DoneChecked`'s fields; `head_branch` is M8a.8's addition (`None`:
    /// detached).
    DoneChecked {
        commits: u32,
        dirty_tracked: u32,
        merge_in_progress: bool,
        untracked_in_owns: Vec<String>,
        outside_owns: Vec<String>,
        generated_outside_owns: Vec<String>,
        protected_changed: Vec<String>,
        red_ok: Option<bool>,
        head: String,
        head_branch: Option<String>,
        /// Ruling T14-R2: `git::resolution_only`'s answer when `VerifyDone` carried a
        /// resolution; `None` otherwise.
        #[serde(default)]
        resolution_only: Option<bool>,
        /// Milestone 9.1 decision 40: the claim's test-weakening signals; `None` when
        /// none were asked for or found.
        #[serde(default)]
        signals: Option<Box<crate::run::tiers::ClaimSignals>>,
        /// Ruling C-21 (5): a sync claim's head contains its `onto`; `None` otherwise.
        #[serde(default)]
        sync_kept: Option<bool>,
    },
    Commits {
        count: u32,
        head: String,
    },
    Diff {
        stat: String,
        patch: String,
    },
    Proof {
        red_failed: bool,
        head_passed: bool,
        matched: bool,
        red_tail: String,
        head_tail: String,
    },
    Check {
        ok: bool,
        code: Option<i32>,
        timed_out: bool,
        tail: String,
        secs: u64,
    },
    Review {
        base: String,
        head: String,
        patch: String,
    },
    Merged {
        commit: String,
        /// Milestone 9.1 decision 16: the tier-2 job that passed (a tiered profile).
        #[serde(default)]
        tier: Option<Box<crate::run::tiers::TierOutcome>>,
    },
    /// Controller ruling C-22 (2): a `Propagate` whose lower head the upper stage
    /// already holds. Nothing was written.
    AlreadyHeld,
    Conflict {
        files: Vec<String>,
        /// Milestone 9.1 decision 51: the conflicted tree `merge-tree` wrote.
        #[serde(default)]
        tree: Option<String>,
    },
    CandidateRed {
        code: Option<i32>,
        timed_out: bool,
        tail: String,
        secs: u64,
        /// Milestone 9.1 decision 16: the red tier-2 job (a tiered profile).
        #[serde(default)]
        tier: Option<Box<crate::run::tiers::TierOutcome>>,
    },
    RefMoved {
        reason: String,
    },
    AcceptConflict {
        files: Vec<String>,
    },
    /// `head` (M8a.14): the task worktree's `HEAD` after the hand-back, `Some` when it
    /// could be read; a clean hand-back re-queues this commit (decision 36).
    HandedBack {
        files: Vec<String>,
        #[serde(default)]
        head: Option<String>,
        /// Ruling T14-C1: the tip the run head was merged onto (`git::HandBack.onto`).
        /// Only a merge onto the claimed commit re-queues without the gates.
        #[serde(default)]
        onto: Option<String>,
        /// Milestone 9 decision 42e: with `list_merged`, the commits a clean merge
        /// brought in, `<sha> <subject>`, newest first (at most 20).
        #[serde(default)]
        merged: Vec<String>,
        /// M9.13a review, item 6: how many commits the clean merge brought in, all of
        /// them (`git rev-list --count`), which `merged` lists at most 20 of.
        #[serde(default)]
        merged_total: u32,
    },
    /// `AbortMerge` succeeded (ruling T11-N1(b)).
    MergeAborted,
    Removed {
        salvage_ref: Option<String>,
    },
    RefsOk,
    /// `kept_branches` (M8a.14, carry T9): the branches `delete_branches` skipped
    /// because a worktree has them checked out; the run's log (and so its report)
    /// names them.
    Finished {
        outcome: String,
        #[serde(default)]
        kept_branches: Vec<String>,
    },
    Failed {
        message: String,
    },
    /// M8b decision 18: `Decide`'s answer, from the decider or its fallback. Boxed: a
    /// triage answer would triple every result's size (clippy's `large_enum_variant`).
    Decided(Box<Decision>),
    /// M8b decision 32: `MeasureDiff`'s answer.
    DiffMeasured(proto::DiffStats),
    /// M8b decision 33: `AppendHistory` wrote its line.
    HistoryAppended,
    /// Milestone 9: `RestartOrchestrator` restarted the window.
    Restarted,
    /// `StartScout`'s scout window.
    ScoutStarted {
        window_id: u32,
    },
    /// `ResolveTarget`'s `(base, head)` commits.
    Target {
        base: String,
        head: String,
    },
    /// `StartPlanner`'s sub-planner window.
    PlannerStarted {
        window_id: u32,
    },
    /// Milestone 9.1: a tier job's outcome.
    Tier(Box<crate::run::tiers::TierOutcome>),
    /// Decision 36: a probe is `red` when a command failed twice; `failing` are those
    /// commands, `tail` the last red run's output, `show` the probed merge's
    /// `show --stat` (task M9.1.15).
    TestAt {
        red: bool,
        failing: Vec<String>,
        tail: String,
        show: Option<String>,
    },
    /// Decision 48: `CreateStageBranch` made the branch at its `from`.
    StageCreated,
}
