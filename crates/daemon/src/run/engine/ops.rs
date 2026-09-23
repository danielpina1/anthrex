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
        /// Ruling T14-R2: set for a claim after the merge queue's conflicted hand-back.
        /// **Executor contract (M8a.22):** after `verify_done`, run
        /// `git::resolution_only(worktree, head, onto, run_head, files)` (a read) and
        /// return it as `DoneChecked.resolution_only`.
        #[serde(default)]
        resolution: Option<ResolutionAt>,
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
    /// Decision 33's fail-to-pass proof (M8a.13), `run::proof::ProofOp`'s fields: the
    /// engine fills `command` with `proof::proof_command(single_test, test)` and `passed`
    /// with `proof::proof_pattern(test_passed, test)` (`{test}` alone when the profile
    /// has no `test_passed`), `path` is the task's `<task>.proof` worktree and `env` the
    /// profile's with `{worktree}` that path. **Executor contract (M8a.22):** run
    /// `run_proof` on `spawn_blocking` with the `git_write` hook
    /// `|step| handle.block_on(queue.write(&project, step))`, where `handle` is the
    /// daemon's `tokio::runtime::Handle`. `Handle::block_on` inside `spawn_blocking`
    /// needs the multi-thread runtime (the daemon's): on a current-thread runtime it
    /// would deadlock, because the queue's future could never be polled. Map
    /// `Ok(ProofRuns)` to `OpResult::Proof`, `ProofError::SetupFailed` to
    /// `OpResult::SetupFailed` and `ProofError::Failed` to `OpResult::Failed`.
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
    /// Decision 34's check. With `scratch` (ruling T13-I3), `dir` is the task's scratch
    /// worktree (`<task>.proof`, the proof's) and the check runs on the claimed commit,
    /// never the branch tip: **executor contract (M8a.22)** — as `run_proof` does, through
    /// the same `git_write` hook, `prepare_scratch(root, dir, commit)`, `setup` once per
    /// new worktree (the `anthrex-setup-ok` marker), then `materialize(dir, commit)`,
    /// then the command. A setup failure is `OpResult::SetupFailed`. Without `scratch`
    /// (M8a.14's final check in the integration worktree) it runs in `dir` as it is.
    Check {
        dir: PathBuf,
        command: String,
        timeout_secs: u64,
        env: Vec<(String, String)>,
        #[serde(default)]
        scratch: Option<ScratchAt>,
    },
    PrepareReview {
        root: PathBuf,
        head_ref: String,
        base_ref: String,
        path: PathBuf,
    },
    /// Decision 36, one attempt of the merge queue (M8a.14): the ref guard (decision
    /// 21), `merge_tree(expected_run_head, task_head)`, then on a clean tree
    /// `commit_tree` with `message`, `materialize` in `integration` and `check` there
    /// (none: straight on), then the ref guard again, `cas_update(run_branch,
    /// candidate, expected_run_head)` and `reattach`; a red check `reattach`es only.
    /// `task_head` is the claimed commit that passed the gates (`Task.head`), never the
    /// task branch's tip (ruling T13-R2, N3). **Executor contract (M8a.22):**
    /// `commit_tree`, `materialize`, `cas_update` and `reattach` are writes, each through
    /// `GitQueue::write` keyed by the run's `project`; `merge_tree` and `guard_refs` may
    /// bypass it. A guard that finds the base advanced sends `Event::BaseAdvanced`
    /// before carrying on; one that halts returns `RefMoved`. A CAS that finds the run
    /// ref moved is `RefMoved` too. Results: `Merged { commit }`, `Conflict { files }`,
    /// `CandidateRed`, `RefMoved`, `Failed`.
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
    /// Decision 36 step 6 (the merge queue's, M8a.14) and M8a.6 ruling N5's (M8a.11):
    /// `git::hand_back(worktree, run_head)`, a write through `GitQueue::write`. The
    /// result is `HandedBack { files, head, onto }` from `git::HandBack`: `onto` the tip
    /// the merge was made onto, `head` the worktree's `HEAD` after it. A clean or
    /// resolved hand-back goes straight back to the merge queue only when `onto` is
    /// `task_head` (ruling T14-C1).
    HandBack {
        worktree: PathBuf,
        run_head: String,
        /// The claimed commit (`Task.head`) the engine expects the merge to be made
        /// onto (ruling T14-C1). The executor reports the tip it actually merged onto
        /// in `HandedBack.onto`.
        #[serde(default)]
        task_head: Option<String>,
    },
    /// Ruling T11-N1(b): `git::abort_merge` in a held task's worktree, undoing a
    /// hand-back that conflicted while another dependency was still unfinished.
    AbortMerge { worktree: PathBuf },
    /// Decision 20: `salvage(path, salvage_ref, …)` then `remove_worktree`, both writes
    /// through `GitQueue::write`. The result is `Removed { salvage_ref }` (`Some` only
    /// when the worktree was dirty).
    RemoveWorktree {
        root: PathBuf,
        path: PathBuf,
        salvage_ref: String,
    },
    /// Decision 37's ref guard before `complete` (M8a.14): `guard_refs`, a read.
    /// `RefCheck::Ok` is `RefsOk`; `BaseAdvanced` sends `Event::BaseAdvanced`, then
    /// `RefsOk`; `Halt { reason }` is `RefMoved { reason }`.
    VerifyRefs {
        root: PathBuf,
        base_branch: String,
        expected_base: String,
        run_branch: String,
        expected_run_head: String,
    },
    /// Decision 20's accept (M8a.14 emits it for `run accept` on a complete run):
    /// `git::accept`, then `salvage` and `remove_worktree` for every one of
    /// `worktrees`, then `delete_branches(branch_prefix)`; every step is a write through
    /// `GitQueue::write`. **The executor must never wrap `accept` in a timeout shorter
    /// than `git::ACCEPT_MERGE_TIMEOUT` (10 minutes)**: the user's hooks and signing run
    /// inside its merge, and it aborts that merge itself when its own deadline passes.
    /// `expected_base` is the base head the user confirmed (`Run.base_moved`'s `to` when
    /// the base advanced; the driver sends `Event::BaseAdvanced` before `Finish` when
    /// its read finds a newer one). Results: `Finished { outcome, kept_branches }`,
    /// `AcceptConflict { files }` (the merge aborted, the base untouched), `Failed`.
    /// `Failed` means the base branch is untouched (review m5): once `git::accept` has
    /// merged, the run is accepted whatever follows, so a failure in the salvage,
    /// removal or branch deletion after it is still `Finished`, with an `outcome` that
    /// names it (`accepted; clean-up failed: <error>`) and the branches kept.
    Accept {
        root: PathBuf,
        base_branch: String,
        expected_base: String,
        run_branch: String,
        message: String,
        worktrees: Vec<(PathBuf, String)>,
        branch_prefix: String,
    },
    /// Decision 20's discard: `salvage` and `remove_worktree` for every one of
    /// `worktrees`, `worktree prune`, then `delete_branches(branch_prefix)`; writes
    /// through `GitQueue::write`. The result is `Finished { outcome, kept_branches }`.
    Discard {
        root: PathBuf,
        worktrees: Vec<(PathBuf, String)>,
        branch_prefix: String,
    },
}

/// The merge queue's conflicted hand-back a claim may resolve (ruling T14-R2): the tip
/// the run head was merged onto, that run head, and the files that conflicted.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResolutionAt {
    pub onto: String,
    pub run_head: String,
    pub files: Vec<String>,
}

/// Where a scratch-worktree check runs (ruling T13-I3): the repository, the claimed
/// commit to materialize, and the profile's `setup` for a new scratch worktree.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScratchAt {
    pub root: PathBuf,
    pub commit: String,
    pub setup: Option<String>,
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
        /// Ruling T14-R2: `git::resolution_only`'s answer when `VerifyDone` carried a
        /// resolution; `None` otherwise.
        #[serde(default)]
        resolution_only: Option<bool>,
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
}
