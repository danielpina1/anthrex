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
            // The last `done` of an op wins. An op runs once, but a replayed accept's
            // clean-up appends a second `Done` for it (ruling T22-N3), which wins.
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
            git::worktree(g, root, branch, path, setup.is_some(), &own, notes)
        }
        OpKind::PrepareWorktree {
            root,
            branch,
            from,
            path,
            setup,
            ..
        } => {
            // Final fix batch F1c (3a): a task's checkout is its own repository in the
            // run's data directory.
            let repo = crate::run::git::checkout_repo_dir(&run.data_dir, path);
            git::task_checkout(g, root, branch, from, path, &repo, setup.is_some())
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
            also_integration,
            ..
        } => git::merge_candidate(
            g,
            root,
            integration,
            (run_branch, *also_integration),
            expected_run_head,
            task_head,
            notes,
        ),
        OpKind::HandBack {
            worktree,
            run_head,
            list_merged,
            ..
        } => git::hand_back(g, worktree, (run_head, *list_merged), notes),
        // Milestone 9.1 decision 53.
        OpKind::CreateStageBranch { root, branch, from } => {
            git::created_at(g, root, (branch, from), OpResult::StageCreated)
        }
        // Decision 53: a propagate is reconciled as a merge candidate, its second
        // parent the lower stage's head.
        OpKind::Propagate(spec) => git::merge_candidate(
            g,
            &spec.root,
            &spec.integration,
            (&spec.to_branch, spec.also_integration),
            &spec.expected_to_head,
            &spec.from_head,
            notes,
        ),
        OpKind::AbortMerge { worktree } => git::abort_merge(g, worktree),
        OpKind::RemoveWorktree {
            root,
            path,
            salvage_ref,
            keep_path,
            ..
        } => {
            let repo = crate::run::git::checkout_repo_dir(&run.data_dir, path);
            git::remove_worktree(g, root, (path, *keep_path), &repo, salvage_ref, notes)
        }
        // Milestone 9.5 decision 27: the task branch alone.
        OpKind::CrownRacer {
            root,
            task_branch,
            lane_head,
            ..
        } => {
            let crowned = OpResult::Crowned {
                head: lane_head.clone(),
            };
            git::created_at(g, root, (task_branch, lane_head), crowned)
        }
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
        // Milestone 9.1 decision 29: a tier job and a bisect probe only read, as a check.
        // Milestone 9.2 decision 10: a host op needs the network; it is re-issued.
        OpKind::Tier(_) | OpKind::TestAt(_) | OpKind::Host { .. } => Ok(Reconciled::NotStarted),
        // M8b decision 18: a decider only reads its prompt; it is simply asked again.
        OpKind::Decide { .. } => Ok(Reconciled::NotStarted),
        // Milestone 9 decisions 11 and 20: a restored orchestrator window is the op's
        // result (dormant: `run resume` restarts it). A restart and a target resolve are
        // simply issued again. A run scout or a sub-planner is never replayed and nothing is
        // killed; its record is failed on `Restore`.
        OpKind::CreateOrchestrator { .. } => Ok(sessions::restored_orchestrator(run, windows)),
        OpKind::RestartOrchestrator { .. }
        | OpKind::ResolveTarget { .. }
        | OpKind::StartScout { .. }
        | OpKind::StartPlanner { .. }
        | OpKind::StartDesignAgent { .. } => Ok(Reconciled::NotStarted),
        // M8b decision 32: a diff is only read, and simply measured again.
        OpKind::MeasureDiff { .. } => Ok(Reconciled::NotStarted),
        // M8b decision 33: a history line is appended again unless the file holds its
        // record already.
        OpKind::AppendHistory {
            path, record_id, ..
        } => super::history_io::contains_record(path, record_id)
            .map(|found| match found {
                true => Reconciled::Replay(OpResult::HistoryAppended),
                false => Reconciled::NotStarted,
            })
            .map_err(|error| error.to_string()),
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

#[cfg(test)]
#[path = "orch_tests.rs"]
mod orch_tests;

#[cfg(test)]
mod tests {
    use std::ffi::OsStr;
    use std::time::Duration;

    use super::{Reconciled, reconcile};
    use crate::decider::fallback::fallback_decision;
    use crate::decider::{BlockedReasonInput, DeciderRequest};
    use crate::run::engine::{OpKind, OpResult};
    use crate::run::journal::JournalLine;
    use crate::run::model::PendingOp;
    use crate::run::test_support::{EXAMPLE_PLAN, run_ok};

    /// M8b decision 18: a decider's intent with no result is `NotStarted` (it is asked
    /// again), with no note and no git; its journaled result is replayed.
    #[test]
    fn decide_is_not_started() {
        let mut run = run_ok(EXAMPLE_PLAN);
        // No round, so no recorded pid: reconcile's leftover-session step has nothing.
        assert!(run.tasks.iter().all(|t| t.rounds.is_empty()));
        let request = DeciderRequest::BlockedReason(BlockedReasonInput {
            task_id: "t1".into(),
            title: "Reset token model".into(),
            reason: "which port?".into(),
        });
        for op in [4, 5] {
            let kind = OpKind::Decide {
                decider_id: op,
                task_ids: vec!["t1".into()],
                request: request.clone(),
            };
            let pending = PendingOp {
                op,
                task_id: Some("t1".into()),
                kind,
                lane: None,
            };
            run.pending_ops.insert(op, pending);
        }
        let decided = OpResult::Decided(Box::new(fallback_decision(&request, "x".into())));
        let journal: Vec<JournalLine> = run
            .pending_ops
            .values()
            .map(|p| JournalLine::Intent {
                op: p.op,
                kind: p.kind.clone(),
            })
            .chain([JournalLine::Done {
                op: 5,
                result: decided.clone(),
            }])
            .collect();
        let git = OsStr::new("/nonexistent/anthrex-test/git");
        let out = reconcile(git, &run, &journal, &[], Duration::from_secs(1));
        assert_eq!(
            out.ops,
            vec![
                (4, Reconciled::NotStarted),
                (5, Reconciled::Replay(decided))
            ]
        );
        assert!(out.notes.is_empty(), "{:?}", out.notes);
    }

    /// Milestone 9.1 decision 53: a `CreateStageBranch` with an intent only is done when
    /// its branch is at `from`, not started when it is absent, and a moved ref when it
    /// is anywhere else (a real repository of the test's own).
    #[test]
    fn reconcile_create_stage_branch_is_done_when_the_ref_exists_at_from() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        let git = |args: &[&str]| {
            let out = std::process::Command::new("git")
                .arg("-C")
                .arg(root)
                .args(args)
                .env("GIT_CONFIG_GLOBAL", "/dev/null")
                .env("GIT_CONFIG_NOSYSTEM", "1")
                .env_remove("GIT_DIR")
                .env_remove("GIT_WORK_TREE")
                .env_remove("GIT_INDEX_FILE")
                .env_remove("GIT_COMMON_DIR")
                .env_remove("GIT_PREFIX")
                .output()
                .unwrap();
            assert!(out.status.success(), "git {args:?}: {out:?}");
            String::from_utf8(out.stdout).unwrap().trim().to_string()
        };
        git(&["init", "-q", "-b", "main"]);
        git(&["config", "user.name", "Stage Test"]);
        git(&["config", "user.email", "stage@test"]);
        git(&["commit", "-q", "--allow-empty", "-m", "one"]);
        let one = git(&["rev-parse", "HEAD"]);
        git(&["commit", "-q", "--allow-empty", "-m", "two"]);
        let two = git(&["rev-parse", "HEAD"]);
        git(&["branch", "anthrex/r1/stage-2", &one]);
        git(&["branch", "anthrex/r1/stage-3", &two]);
        // Controller ruling C-15 (M-4): a symbolic ref, though it resolves to `from`.
        git(&["branch", "base-one", &one]);
        git(&[
            "symbolic-ref",
            "refs/heads/anthrex/r1/stage-5",
            "refs/heads/base-one",
        ]);
        let mut run = run_ok(EXAMPLE_PLAN);
        for (op, n) in [(1, 2), (2, 3), (3, 4), (4, 5)] {
            let kind = OpKind::CreateStageBranch {
                root: root.to_path_buf(),
                branch: format!("anthrex/r1/stage-{n}"),
                from: one.clone(),
            };
            let pending = PendingOp {
                op,
                task_id: None,
                kind,
                lane: None,
            };
            run.pending_ops.insert(op, pending);
        }
        let journal: Vec<JournalLine> = run
            .pending_ops
            .values()
            .map(|p| JournalLine::Intent {
                op: p.op,
                kind: p.kind.clone(),
            })
            .collect();
        let out = reconcile(
            OsStr::new("git"),
            &run,
            &journal,
            &[],
            Duration::from_secs(20),
        );
        let moved = format!(
            "refs/heads/anthrex/r1/stage-3 exists at {}, not {}",
            &two[..7],
            &one[..7]
        );
        assert_eq!(
            out.ops,
            vec![
                (1, Reconciled::Replay(OpResult::StageCreated)),
                (2, Reconciled::Replay(OpResult::RefMoved { reason: moved })),
                (3, Reconciled::NotStarted),
                (
                    4,
                    Reconciled::Replay(OpResult::RefMoved {
                        reason:
                            "refs/heads/anthrex/r1/stage-5 is a symbolic ref to refs/heads/base-one"
                                .into()
                    })
                ),
            ]
        );
        assert!(out.notes.is_empty(), "{:?}", out.notes);
    }
}

#[cfg(test)]
#[path = "stage_tests.rs"]
mod stage_tests;
