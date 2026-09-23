//! The engine's operations and their results (decision 43's journal carries both).
//! Pure data (design decision 2).

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::headless::HeadlessSpec;

/// One engine operation, executed by the driver and journaled before it runs
/// (decision 43). Every git write among them goes through `GitQueue::write`, keyed by
/// the run's `project`, which the driver reads from the run named in [`Effect::Op`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum OpKind {
    /// The integration worktree at `base_sha` (decisions 14, 16), then `setup`.
    CreateRunBranch {
        root: PathBuf,
        branch: String,
        base_sha: String,
        path: PathBuf,
        setup: Option<String>,
        env: Vec<(String, String)>,
    },
    /// A task worktree on `branch` from `from` (decision 19), reused when it exists and
    /// re-pointed when its branch has no commit of its own; then `setup`, whenever it is
    /// `Some`. The engine sends `Some` on every first creation and on every dispatch of
    /// a pre-warmed branch that is stale, since a re-point drops what the earlier setup
    /// wrote to tracked files (M8a.8's ruling T8-I4).
    PrepareWorktree {
        root: PathBuf,
        branch: String,
        from: String,
        path: PathBuf,
        setup: Option<String>,
        env: Vec<(String, String)>,
    },
    /// Registers a headless window and starts its session (decisions 24–26, 49).
    CreateWindow {
        name: String,
        /// Boxed: the spec dwarfs every other op (clippy's `large_enum_variant`).
        spec: Box<HeadlessSpec>,
        session_uuid: Option<String>,
        first_turn: String,
        project: PathBuf,
        worktree: PathBuf,
        jitter_ms: u64,
    },
    ResumeSession {
        window_id: u32,
        session_id: String,
        message: String,
        jitter_ms: u64,
    },
    VerifyDone {
        worktree: PathBuf,
        start: String,
        run_head: String,
        owns: Vec<String>,
        generated: Vec<String>,
        protected: Vec<String>,
        spill_exempt: bool,
        red: Option<String>,
    },
    /// `run_head`: M8a.8's interface change (the task's own commits exclude a merged
    /// run head).
    CountCommits {
        worktree: PathBuf,
        start: String,
        run_head: String,
    },
    DiffSoFar {
        worktree: PathBuf,
        start: String,
        run_head: String,
    },
    Proof {
        root: PathBuf,
        path: PathBuf,
        red: String,
        head: String,
        command: String,
        passed: String,
        timeout_secs: u64,
        setup: Option<String>,
        env: Vec<(String, String)>,
    },
    Check {
        dir: PathBuf,
        command: String,
        timeout_secs: u64,
        env: Vec<(String, String)>,
    },
    PrepareReview {
        root: PathBuf,
        head_ref: String,
        base_ref: String,
        path: PathBuf,
    },
    MergeCandidate {
        root: PathBuf,
        integration: PathBuf,
        run_branch: String,
        expected_run_head: String,
        base_branch: String,
        expected_base: String,
        task_head: String,
        message: String,
        check: Option<String>,
        timeout_secs: u64,
        env: Vec<(String, String)>,
    },
    HandBack {
        worktree: PathBuf,
        run_head: String,
    },
    /// Ruling T11-N1(b): `git::abort_merge` in a held task's worktree, undoing a
    /// hand-back that conflicted while another dependency was still unfinished.
    AbortMerge {
        worktree: PathBuf,
    },
    RemoveWorktree {
        root: PathBuf,
        path: PathBuf,
        salvage_ref: String,
    },
    VerifyRefs {
        root: PathBuf,
        base_branch: String,
        expected_base: String,
        run_branch: String,
        expected_run_head: String,
    },
    Accept {
        root: PathBuf,
        base_branch: String,
        expected_base: String,
        run_branch: String,
        message: String,
        worktrees: Vec<(PathBuf, String)>,
        branch_prefix: String,
    },
    Discard {
        root: PathBuf,
        worktrees: Vec<(PathBuf, String)>,
        branch_prefix: String,
    },
}

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
    },
    Conflict {
        files: Vec<String>,
    },
    CandidateRed {
        code: Option<i32>,
        timed_out: bool,
        tail: String,
        secs: u64,
    },
    RefMoved {
        reason: String,
    },
    AcceptConflict {
        files: Vec<String>,
    },
    HandedBack {
        files: Vec<String>,
    },
    /// `AbortMerge` succeeded (ruling T11-N1(b)).
    MergeAborted,
    Removed {
        salvage_ref: Option<String>,
    },
    RefsOk,
    Finished {
        outcome: String,
    },
    Failed {
        message: String,
    },
}
