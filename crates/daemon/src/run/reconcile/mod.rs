//! Decision 44 (M8a.21): reconcile on start. For one run loaded from `run.json`, every
//! op still in `pending_ops` is answered from its journal and from reality:
//! - a `done` line → its result, replayed as it is, without touching git;
//! - an `intent` line only → the per-kind check of the Interfaces table (`git` and
//!   `sessions` below), which yields a result to replay or `NotStarted`;
//! - neither (the daemon died between the `Persist` and the intent line) →
//!   `NotStarted`.
//!
//! Before any of that, decision 28's leftover session processes are killed (only the
//! recorded pids of live rounds, orphaned, same uid, the session id in their argv;
//! `sessions.rs`), so nothing is still writing into a worktree while it is read.
//!
//! Does I/O (design decision 2), all of it **blocking**: `lifecycle::run` runs it before
//! the socket is bound, and the driver (M8a.22) calls it on `spawn_blocking`, never under
//! the manager lock. Every git call goes through `run::git`'s helpers (AGENTS.md rules
//! 10 and 11, decision 18's write flags) with `timeout` per command.
//!
//! **For M8a.22.** [`Reconciliation::replay`] gives `Event::Restore`'s `replay` entries
//! for one run; the ops answered `NotStarted` are exactly the pending ops left out of
//! it, which the reducer's `restore` drops and re-issues (`engine/restore.rs`). The
//! notes say what reconcile killed, removed, re-attached or could not read; they are
//! meant for the run's log. A check that cannot read reality answers `NotStarted` with a
//! note: every op is idempotent (decision 44), and a merge candidate re-issued over a
//! run branch that did move is caught by its own ref guard.

mod git;
mod sessions;

use std::collections::{BTreeMap, BTreeSet};
use std::ffi::OsStr;
use std::time::Duration;

use proto::WindowInfo;

use super::engine::{OpKind, OpResult};
use super::journal::JournalLine;
use super::model::{OpId, PendingOp, Run};

pub use sessions::{ORPHAN_PARENT, SESSION_KILL_GRACE};

/// What one pending op turned out to be.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Reconciled {
    /// It ran (or ran far enough); the engine applies this result as the op's `OpDone`.
    Replay(OpResult),
    /// It did not run, or is safe to run again: the engine drops it and re-issues
    /// whatever the task's state needs.
    NotStarted,
}

/// Every pending op's answer, in op order, and what reconcile did or could not read.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Reconciliation {
    pub ops: Vec<(OpId, Reconciled)>,
    pub notes: Vec<String>,
}

impl Reconciliation {
    /// `Event::Restore`'s `replay` entries for the run `run_id`.
    pub fn replay(&self, run_id: &str) -> Vec<(String, OpId, OpResult)> {
        self.ops
            .iter()
            .filter_map(|(op, answer)| match answer {
                Reconciled::Replay(result) => Some((run_id.to_string(), *op, result.clone())),
                Reconciled::NotStarted => None,
            })
            .collect()
    }
}

/// Decision 44 for one run: `journal` is its `journal.jsonl` as `journal::load_all`
/// read it, `windows` the manager's windows after `WindowManager::restore`.
pub fn reconcile(
    git: &OsStr,
    run: &Run,
    journal: &[JournalLine],
    windows: &[WindowInfo],
    timeout: Duration,
) -> Reconciliation {
    reconcile_with_orphan_parent(git, run, journal, windows, timeout, ORPHAN_PARENT)
}

/// [`reconcile`], with the parent pid an orphaned session has: [`ORPHAN_PARENT`] (1)
/// in the daemon; a test on a platform that reparents orphans to a subreaper passes
/// the one it observes.
pub fn reconcile_with_orphan_parent(
    git: &OsStr,
    run: &Run,
    journal: &[JournalLine],
    windows: &[WindowInfo],
    timeout: Duration,
    orphan_parent: u32,
) -> Reconciliation {
    let mut notes = sessions::kill_leftovers(run, orphan_parent, timeout);
    let mut intents: BTreeSet<OpId> = BTreeSet::new();
    let mut dones: BTreeMap<OpId, &OpResult> = BTreeMap::new();
    for line in journal {
        match line {
            JournalLine::Intent { op, .. } => {
                intents.insert(*op);
            }
            // The last `done` of an op wins; an op runs once, so there is one.
            JournalLine::Done { op, result } => {
                dones.insert(*op, result);
            }
        }
    }
    let mut ops = Vec::with_capacity(run.pending_ops.len());
    for (op, pending) in &run.pending_ops {
        let answer = if let Some(result) = dones.get(op) {
            Reconciled::Replay((*result).clone())
        } else if intents.contains(op) {
            check(git, run, pending, windows, timeout, &mut notes)
        } else {
            Reconciled::NotStarted
        };
        ops.push((*op, answer));
    }
    Reconciliation { ops, notes }
}

/// The Interfaces table's "Reality checked" column for an op with an intent only.
fn check(
    git: &OsStr,
    run: &Run,
    pending: &PendingOp,
    windows: &[WindowInfo],
    timeout: Duration,
    notes: &mut Vec<String>,
) -> Reconciled {
    let g = super::git::Git::new(git, timeout);
    let checked = match &pending.kind {
        OpKind::CreateRunBranch {
            root,
            branch,
            path,
            setup,
            ..
        } => {
            let own = run.wt_dir.join("runs").join(&run.id);
            git::worktree(g, root, branch, false, path, setup.is_some(), &own, notes)
        }
        OpKind::PrepareWorktree {
            root,
            branch,
            path,
            setup,
            ..
        } => {
            let own = run.wt_dir.join("runs").join(&run.id);
            git::worktree(g, root, branch, true, path, setup.is_some(), &own, notes)
        }
        OpKind::CreateWindow { spec, .. } => Ok(sessions::restored_window(
            run,
            pending,
            spec.run_ref.as_ref(),
            windows,
        )),
        OpKind::MergeCandidate {
            root,
            integration,
            run_branch,
            expected_run_head,
            task_head,
            ..
        } => git::merge_candidate(
            g,
            root,
            integration,
            run_branch,
            expected_run_head,
            task_head,
            notes,
        ),
        OpKind::HandBack {
            worktree, run_head, ..
        } => git::hand_back(g, worktree, run_head, notes),
        OpKind::AbortMerge { worktree } => git::abort_merge(g, worktree),
        OpKind::RemoveWorktree {
            root,
            path,
            salvage_ref,
        } => git::remove_worktree(g, root, path, salvage_ref, notes),
        OpKind::Accept {
            root,
            base_branch,
            run_branch,
            expected_run_head,
            branch_prefix,
            ..
        } => git::accept(
            g,
            root,
            base_branch,
            run_branch,
            expected_run_head,
            branch_prefix,
            notes,
        ),
        // Decision 44: a resumed session's process was killed above; the rest read, or
        // are idempotent, and are simply issued again.
        OpKind::ResumeSession { .. }
        | OpKind::VerifyDone { .. }
        | OpKind::CountCommits { .. }
        | OpKind::DiffSoFar { .. }
        | OpKind::Proof { .. }
        | OpKind::Check { .. }
        | OpKind::PrepareReview { .. }
        | OpKind::VerifyRefs { .. }
        | OpKind::Discard { .. } => Ok(Reconciled::NotStarted),
    };
    checked.unwrap_or_else(|err| {
        notes.push(format!(
            "op {} ({}): could not check it after the restart: {err}; treated as not started",
            pending.op,
            pending.kind.name()
        ));
        Reconciled::NotStarted
    })
}
