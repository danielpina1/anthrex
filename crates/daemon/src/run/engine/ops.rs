//! The engine's operations and their results (decision 43's journal carries both).
//! Pure data (design decision 2).

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::decider::DeciderRequest;
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
        /// Milestone 9 decision 34: where the driver puts the task's scout extract into
        /// `first_turn`, when the task names scout reports.
        #[serde(default)]
        extract: Option<crate::run::orch::extract::ExtractSlot>,
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
        /// return it as `DoneChecked.resolution_only`. An error from it (a timeout, a
        /// `merge-tree` failure) counts as `Some(false)`, so the gates run; it never
        /// fails the `DoneChecked` (ruling T14-R3).
        /// Boxed, with `sync`: keeps the op under clippy's `large_enum_variant`.
        #[serde(default)]
        resolution: Option<Box<ResolutionAt>>,
        /// Milestone 9 decision 42e (M9.13a review, item 3): the refresh merge commits
        /// the count leaves out, each once (`worker_messages::not_own`).
        #[serde(default)]
        not_own: Vec<String>,
        /// The run heads the task's refreshes merged (`worker_messages::not_run`): the
        /// count leaves out all they reach (M9.13a re-review).
        #[serde(default)]
        not_run: Vec<String>,
        /// Milestone 9.1 decision 40: set for a tiered profile with `test_paths` or
        /// `skip_markers`. **Executor contract:** after `verify_done`, read
        /// `git::done_signals` over `<run_head>...<head>` and return them as
        /// `DoneChecked.{signals, signals_more}`. `None`: no `-U0` diff is read.
        #[serde(default)]
        signals: Option<crate::run::tiers::SignalsSpec>,
        /// Decision 51: a sync task's conflicted tree. **Executor contract:** the spill
        /// diff is `<spill_base> <head>`, not `<run_head>...<head>`.
        #[serde(default)]
        spill_base: Option<String>,
        /// Ruling C-21 (3, 5): a sync task's merge, checked on its claim.
        #[serde(default)]
        sync: Option<Box<crate::run::model::SyncCheck>>,
    },
    /// `run_head`: M8a.8's interface change (the task's own commits exclude a merged
    /// run head). `not_own` as `VerifyDone`'s.
    CountCommits {
        worktree: PathBuf,
        start: String,
        run_head: String,
        #[serde(default)]
        not_own: Vec<String>,
        #[serde(default)]
        not_run: Vec<String>,
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
        /// Ruling C-21 (6): a sync task's conflicted tree, the diff's base.
        #[serde(default)]
        base_tree: Option<String>,
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
        /// Milestone 9.1 decision 53: every `(branch, expected head)` the guard reads;
        /// empty (an intent from before it) is `[(run_branch, expected_run_head)]`.
        #[serde(default)]
        guarded: Vec<(String, String)>,
        /// Decision 53: `run_branch` is a `Multi` run's highest stage, so `integration`
        /// moves with it, in one `refs_tx::cas` transaction.
        #[serde(default)]
        also_integration: bool,
        /// Milestone 9.1 decision 16: a tiered profile's tier-2 job, run on the
        /// candidate where M8a runs `check` (then `None`); `None` for an untiered one.
        #[serde(default)]
        tier: Option<Box<crate::run::tiers::TierSpec>>,
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
        /// Milestone 9 decision 42e: a refresh's hand-back. The executor first checks
        /// the tracked tree is clean (`Failed { "uncommitted changes" }` otherwise) and,
        /// after a clean merge, lists the merged commits in `HandedBack.merged`.
        #[serde(default)]
        list_merged: bool,
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
        /// Milestone 9.1 decision 53: as `MergeCandidate.guarded`.
        #[serde(default)]
        guarded: Vec<(String, String)>,
    },
    /// Decision 20's accept (M8a.14 emits it for `run accept` on a complete run):
    /// `git::accept`, then `salvage` and `remove_worktree` for every one of
    /// `worktrees`, then `delete_branches(branch_prefix)`; every step is a write through
    /// `GitQueue::write`. **The executor must never wrap `accept` in a timeout shorter
    /// than `git::ACCEPT_MERGE_TIMEOUT` (10 minutes)**: the user's signing runs
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
        /// `Run.run_head`: the head the report, review and checks describe. Accept
        /// merges this commit, and refuses when the run branch has moved from it (final
        /// fix batch F1, finding D-6). Empty in an intent journaled before it existed;
        /// reconcile then reads the run branch.
        #[serde(default)]
        expected_run_head: String,
        message: String,
        worktrees: Vec<(PathBuf, String)>,
        branch_prefix: String,
    },
    /// Decision 20's discard: `salvage` and `remove_worktree` for every one of
    /// `worktrees` (no repository-wide `worktree prune`: final fix batch F1, D-12), then
    /// `delete_branches(branch_prefix)`; writes
    /// through `GitQueue::write`. The result is `Finished { outcome, kept_branches }`.
    Discard {
        root: PathBuf,
        worktrees: Vec<(PathBuf, String)>,
        branch_prefix: String,
    },
    /// M8b decision 18: one decider call, holding a reader slot while it runs.
    /// **Executor contract:** `decider::call::decide` with the adaptation's
    /// `DeciderContext` (`driver/adapt.rs`), never under a lock; the result is always
    /// `Decided`, a fallback included. `task_ids` are the tasks the answer is for.
    Decide {
        decider_id: u64,
        task_ids: Vec<String>,
        request: DeciderRequest,
    },
    /// M8b decision 32: what a task changed, `from` → `to` in the user's checkout
    /// `root` (three dots: from their merge base). **Executor contract:**
    /// `history_io::measure_diff` on `spawn_blocking` (reads, no queue); the result is
    /// `DiffMeasured`, or `Failed` (the record is then written without a diff).
    MeasureDiff {
        root: PathBuf,
        from: String,
        to: String,
        three_dot: bool,
    },
    /// M8b decision 33: one line of `history.jsonl`. **Executor contract:**
    /// `history_io::append_line` on `spawn_blocking`, after filling a `run` record's
    /// `accepted_commit` from `refs/heads/<base>` when its outcome is `accepted`; the
    /// result is `HistoryAppended`, or `Failed`.
    AppendHistory {
        path: PathBuf,
        record_id: String,
        /// Boxed: a task record would triple every op's size (clippy's
        /// `large_enum_variant`); invisible in the journal.
        line: Box<proto::HistoryLine>,
    },
    /// Milestone 9 decision 5: the run's orchestrator window, `WindowManager::
    /// create_run_window`; the result is `Window`, or `Failed`. Boxed as `CreateWindow`'s
    /// spec is.
    CreateOrchestrator {
        spec: Box<proto::WindowSpec>,
        role: Box<crate::launch::role::RoleLaunch>,
        project: PathBuf,
    },
    /// Decision 11: `WindowManager::restart` of the orchestrator window, which re-passes
    /// its role with its session; the result is `Restarted`, or `Failed`.
    RestartOrchestrator { window_id: u32 },
    /// Decision 20: a run scout on M8b's `ScoutService`; the result is `ScoutStarted`.
    StartScout {
        spec: Box<crate::scout::spec::ScoutSpec>,
    },
    /// Decision 36: a review task's target, resolved in `root`; the result is `Target`.
    ResolveTarget {
        root: PathBuf,
        target: String,
        base_branch: String,
    },
    /// Decisions 31 and 32: a sub-planner session on M8b's scout machine; the result is
    /// `PlannerStarted`, or `Failed`.
    StartPlanner {
        spec: Box<crate::scout::planner::PlannerSpec>,
    },
    /// Milestone 9.1 decisions 14 and 18: one tier job (`driver/tier.rs::run_tier`),
    /// a read, reconciled `NotStarted`. The result is `Tier`, `SetupFailed` or `Failed`.
    Tier(Box<crate::run::tiers::TierSpec>),
    /// Decision 36: one bisect probe (`driver/tier.rs::run_test_at`), a read,
    /// reconciled `NotStarted`. The result is `TestAt`, `SetupFailed` or `Failed`.
    TestAt(Box<crate::run::tiers::TestAtSpec>),
    /// Decision 48: `refs_tx::create_branch(root, branch, from)`, create-only, a write
    /// through `GitQueue::write`. The result is `StageCreated`, `RefMoved` (the branch
    /// exists elsewhere) or `Failed`.
    CreateStageBranch {
        root: PathBuf,
        branch: String,
        from: String,
    },
    /// Decision 50: a stage's head merged into the stage above, executed as a merge
    /// candidate (`driver/stage_ops.rs`). Results as `MergeCandidate`'s.
    Propagate(Box<crate::run::model::PropagateSpec>),
}

impl OpKind {
    /// The variant's name, as the journal spells it (`"MergeCandidate"`): for notes,
    /// logs and decision 48's `ANTHREX_TEST_ABORT_AFTER_INTENT=<op kind>` (M8a.21).
    pub fn name(&self) -> &'static str {
        match self {
            OpKind::CreateRunBranch { .. } => "CreateRunBranch",
            OpKind::PrepareWorktree { .. } => "PrepareWorktree",
            OpKind::CreateWindow { .. } => "CreateWindow",
            OpKind::ResumeSession { .. } => "ResumeSession",
            OpKind::VerifyDone { .. } => "VerifyDone",
            OpKind::CountCommits { .. } => "CountCommits",
            OpKind::DiffSoFar { .. } => "DiffSoFar",
            OpKind::Proof { .. } => "Proof",
            OpKind::Check { .. } => "Check",
            OpKind::PrepareReview { .. } => "PrepareReview",
            OpKind::MergeCandidate { .. } => "MergeCandidate",
            OpKind::HandBack { .. } => "HandBack",
            OpKind::AbortMerge { .. } => "AbortMerge",
            OpKind::RemoveWorktree { .. } => "RemoveWorktree",
            OpKind::VerifyRefs { .. } => "VerifyRefs",
            OpKind::Accept { .. } => "Accept",
            OpKind::Discard { .. } => "Discard",
            OpKind::Decide { .. } => "Decide",
            OpKind::MeasureDiff { .. } => "MeasureDiff",
            OpKind::AppendHistory { .. } => "AppendHistory",
            OpKind::CreateOrchestrator { .. } => "CreateOrchestrator",
            OpKind::RestartOrchestrator { .. } => "RestartOrchestrator",
            OpKind::StartScout { .. } => "StartScout",
            OpKind::ResolveTarget { .. } => "ResolveTarget",
            OpKind::StartPlanner { .. } => "StartPlanner",
            OpKind::Tier(_) => "Tier",
            OpKind::TestAt(_) => "TestAt",
            OpKind::CreateStageBranch { .. } => "CreateStageBranch",
            OpKind::Propagate(_) => "Propagate",
        }
    }
}

/// The merge queue's conflicted hand-back a claim may resolve (ruling T14-R2): the tip
/// the run head was merged onto, that run head, and the files that conflicted.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResolutionAt {
    pub onto: String,
    pub run_head: String,
    pub files: Vec<String>,
}

/// `run override` of a blocked task whose commits no accepted claim recorded (M8a.15,
/// decision 35): the `CountCommits` that learns them, the request waiting for its
/// answer, and the user's reason.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OverrideCount {
    pub op: super::OpId,
    pub reply: u64,
    pub reason: String,
}

/// Where a scratch-worktree check runs (ruling T13-I3): the repository, the claimed
/// commit to materialize, and the profile's `setup` for a new scratch worktree.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScratchAt {
    pub root: PathBuf,
    pub commit: String,
    pub setup: Option<String>,
}
