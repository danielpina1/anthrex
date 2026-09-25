//! Decision 44 on start (M8a.22): load every run, reconcile each unfinished op against
//! the journal and reality, step `Event::Restore` before any other event, then compact
//! the journals, watch the runs' live worktrees again, remove the restored windows no
//! live run has. An accept whose merge the old daemon completed is held pending, and
//! finished by its clean-up once the event loop runs (ruling T22-N3).

use std::collections::BTreeSet;
use std::sync::Arc;
use std::time::Duration;

use super::guard::{guarded_step, prepare_guarded};
use super::{OpCtx, RunService, cleanup, effects, unix_now};
use crate::run::engine::{Event, EventKind, OpKind, OpResult};
use crate::run::git;
use crate::run::journal;
use crate::run::model::{LogEntry, OpId, Run};
use crate::run::reconcile;
use crate::worktree::pinned::PinAs;
use proto::RunState;

/// What a replayed accept's clean-up adds to its outcome.
const CLEAN_UP_AFTER_RESTART: &str = "; clean-up after the restart: ";

/// The merge's own outcome of a replayed accept's `Finished` (T22-P3, F4): a crash after
/// a clean-up's `Done` replays that `Done`, whose outcome already carries one clean-up
/// clause, and the next clean-up replaces it rather than nesting a second.
pub(super) fn merge_outcome(merged: &str) -> &str {
    merged
        .split_once(CLEAN_UP_AFTER_RESTART)
        .map_or(merged, |(merge, _)| merge)
}

/// The engine's cap on a run's log (Interfaces, `LogEntry`).
const LOG_MAX: usize = 500;

/// What a replayed `Accept` still owes: its clean-up (M8a.21's concern).
pub(super) struct AcceptCleanUp {
    op: OpId,
    /// The replayed `Finished`'s outcome: what the old daemon's merge did.
    merged: String,
    ctx: OpCtx,
    root: std::path::PathBuf,
    worktrees: Vec<(std::path::PathBuf, String)>,
    branch_prefix: String,
}

impl RunService {
    /// Restores the runs of an earlier daemon. Called once, before the socket is bound
    /// and before [`RunService::spawn`]; the effects of the restore run here.
    pub async fn restore(self: &Arc<Self>) {
        let data_dir = self.ctx.data_dir.clone();
        let loaded = tokio::task::spawn_blocking(move || journal::load_all(&data_dir)).await;
        let (loaded, problems) = match loaded {
            Ok(loaded) => loaded,
            Err(error) => {
                tracing::error!(%error, "loading the runs panicked");
                return;
            }
        };
        for problem in problems {
            tracing::warn!(%problem, "run restore");
        }
        let windows = self.manager.list();
        let now = unix_now();
        let mut runs = Vec::new();
        let mut replay = Vec::new();
        let mut held = Vec::new();
        // Final review B-5: each run as loaded, and the runs with an empty journal and
        // nothing pending (nothing to compact), so an unchanged run costs no write.
        let mut as_loaded = std::collections::HashMap::new();
        let mut quiet = BTreeSet::new();
        for (mut run, lines) in loaded {
            if lines.is_empty() && run.pending_ops.is_empty() {
                quiet.insert(run.id.clone());
            }
            // F3 review N4: as on disk, before the restore's own log notes, so a run
            // whose only change is a note is still saved.
            as_loaded.insert(run.id.clone(), run.clone());
            let (git, windows) = (self.ctx.git.clone(), windows.clone());
            let timeout = Duration::from_secs(run.limits.git_timeout_secs);
            let snapshot = run.clone();
            let checked = tokio::task::spawn_blocking(move || {
                // Fix round 1 of final fix batch F1 (N2): every run worktree is pinned to
                // its git directory before any git call in it, reconcile's included.
                git::pin_worktrees(&snapshot.git_common_dir, &run_worktree_paths(&snapshot));
                reconcile::reconcile(&git, &snapshot, &lines, &windows, timeout)
            })
            .await;
            let Ok(reconciled) = checked else {
                // Ruling T22-minors, m7: said in the run's own log, and held.
                tracing::error!(run = %run.id, "reconcile panicked; the run is held");
                hold_unreconciled(&mut run, now);
                runs.push(run);
                continue;
            };
            let mut answers = reconciled.replay(&run.id);
            for note in reconciled.notes {
                run.log.push(LogEntry {
                    at: now,
                    text: format!("restore: {note}"),
                });
            }
            let excess = run.log.len().saturating_sub(LOG_MAX);
            run.log.drain(..excess);
            // Ruling T22-N3: a replayed accept's clean-up waits for the socket; its op
            // is held pending until the clean-up answers it (`finish_held_accepts`).
            if let Some(cleanup) = replayed_accept(&run, &answers) {
                answers.retain(|(_, op, _)| *op != cleanup.op);
                held.push((run.id.clone(), cleanup.op));
                crate::lock(&self.held_accepts).push(cleanup);
            }
            replay.extend(answers);
            runs.push(run);
        }
        if runs.is_empty() {
            // Final review B-4: windows of runs that are gone go all the same.
            self.remove_stale_windows(&BTreeSet::new());
            return;
        }
        let (mut prepared, skipped) = self.restore_step(runs, replay, held, now);
        crate::lock(&self.held_accepts).retain(|c| !skipped.contains(&c.ctx.run_id));
        // A run the restore left exactly as loaded is already on disk.
        prepared.retain(|item| match item {
            effects::Ready::Save(run) => as_loaded.get(&run.id) != Some(&**run),
            _ => true,
        });
        self.execute(prepared, now).await;
        self.compact_after_restore(&quiet).await;
        self.watch_live_worktrees();
        self.remove_stale_windows(&skipped);
    }

    /// Steps `Event::Restore` for every run at once; when that panics (final review
    /// B-I2), for each run on its own, leaving out any run whose own restore panics.
    /// A run left out stays on disk as it was (nothing writes its `run.json`), and the
    /// next start tries it again; its windows are kept. Returns the effects to execute
    /// and the ids of the runs left out.
    fn restore_step(
        &self,
        runs: Vec<Run>,
        replay: Vec<(String, OpId, OpResult)>,
        held: Vec<(String, OpId)>,
        now: u64,
    ) -> (Vec<effects::Ready>, BTreeSet<String>) {
        let mut state = crate::lock(&self.state);
        let all = Event {
            now,
            kind: EventKind::Restore {
                runs: runs.clone(),
                replay: replay.clone(),
                held: held.clone(),
            },
        };
        let panic = match guarded_step(&mut state, all) {
            Ok(fx) => return (prepare_guarded(&state, fx, now), BTreeSet::new()),
            Err(panic) => panic,
        };
        tracing::error!(%panic, "restoring the runs panicked; restoring them one at a time");
        let mut fx = Vec::new();
        let mut skipped = BTreeSet::new();
        for run in runs {
            let id = run.id.clone();
            let one = Event {
                now,
                kind: EventKind::Restore {
                    runs: vec![run],
                    replay: replay.iter().filter(|(r, ..)| *r == id).cloned().collect(),
                    held: held.iter().filter(|(r, _)| *r == id).cloned().collect(),
                },
            };
            match guarded_step(&mut state, one) {
                Ok(more) => fx.extend(more),
                Err(panic) => {
                    tracing::error!(run = %id, %panic, "restoring the run panicked; it is left out until the next start");
                    skipped.insert(id);
                }
            }
        }
        (prepare_guarded(&state, fx, now), skipped)
    }

    /// Ruling T22-N3, with m5's report: each accept whose merge landed before the
    /// restart gets its clean-up on a task of its own, started with the event loop, so
    /// after the socket is bound. It holds its run's ops exclusively, as the accept did;
    /// its `Finished` says what the clean-up did and which branches it really kept, and
    /// is journaled, then stepped, as any op's result is (decision 43).
    pub(super) fn finish_held_accepts(self: &Arc<Self>) {
        let held = std::mem::take(&mut *crate::lock(&self.held_accepts));
        for cleanup in held {
            let service = self.clone();
            tokio::spawn(async move {
                let order = service.op_lock(&cleanup.ctx.run_id);
                let _order = order.write_owned().await;
                let (outcome, kept_branches) = cleanup::clean_up(
                    &service,
                    &cleanup.ctx,
                    cleanup.root,
                    &cleanup.worktrees,
                    cleanup.branch_prefix,
                )
                .await;
                tracing::info!(run = %cleanup.ctx.run_id, %outcome, ?kept_branches, "accept clean-up after restore");
                if service.stopped.load(std::sync::atomic::Ordering::SeqCst) {
                    return;
                }
                let result = OpResult::Finished {
                    outcome: format!(
                        "{}{CLEAN_UP_AFTER_RESTART}{outcome}",
                        merge_outcome(&cleanup.merged)
                    ),
                    kept_branches,
                };
                let line = journal::JournalLine::Done {
                    op: cleanup.op,
                    result: result.clone(),
                };
                service.append(&cleanup.ctx, line).await;
                service.send(EventKind::OpDone {
                    run_id: cleanup.ctx.run_id,
                    op: cleanup.op,
                    result,
                });
            });
        }
    }

    /// Every restored run's journal, rewritten with only its pending ops (decision 43,
    /// M8a.21's "compacting the journal after the restore"). The restore's `Persist`
    /// has been written by now. A run loaded with an empty journal that still has
    /// nothing pending (`quiet`) has nothing to compact.
    async fn compact_after_restore(&self, quiet: &BTreeSet<String>) {
        let runs: Vec<_> = {
            let state = crate::lock(&self.state);
            state
                .runs
                .values()
                .filter(|run| !(quiet.contains(&run.id) && run.pending_ops.is_empty()))
                .map(|run| {
                    (
                        run.id.clone(),
                        run.data_dir.clone(),
                        run.pending_ops.clone(),
                    )
                })
                .collect()
        };
        let lock = self.journal.clone();
        let _ = tokio::task::spawn_blocking(move || {
            let _held = crate::lock(&lock);
            for (id, dir, pending) in runs {
                if let Err(error) = journal::compact(&dir, &pending) {
                    tracing::error!(run = %id, %error, "journal compaction after restore failed");
                }
            }
        })
        .await;
    }

    /// Decision 22: the integration worktree and each live task worktree of every run
    /// not yet finished are watched again.
    fn watch_live_worktrees(&self) {
        let roots: Vec<_> = {
            let state = crate::lock(&self.state);
            state
                .runs
                .values()
                .filter(|run| !run.state.is_terminal())
                .flat_map(|run| {
                    let tasks = run
                        .tasks
                        .iter()
                        .filter(|t| t.worktree_live)
                        .map(|t| t.worktree.clone());
                    std::iter::once(run.integration_path()).chain(tasks)
                })
                .collect()
        };
        for root in roots.into_iter().filter(|root| root.exists()) {
            self.watch_root(&root);
        }
    }

    /// The M8a.17 carry: a restored headless window whose run is gone or finished is
    /// removed; only the engine can remove one (decision 49). A run whose restore
    /// panicked (`skipped`) keeps its windows.
    fn remove_stale_windows(&self, skipped: &BTreeSet<String>) {
        let live: BTreeSet<String> = {
            let state = crate::lock(&self.state);
            state
                .runs
                .values()
                .filter(|run| !run.state.is_terminal())
                .map(|run| run.id.clone())
                .chain(skipped.iter().cloned())
                .collect()
        };
        for window in self.manager.list() {
            let Some(run) = window.run else { continue };
            if !live.contains(&run.run_id) {
                let _ = self.manager.remove(window.id);
            }
        }
    }
}

/// Ruling T22-minors, m7: a run whose ops reconcile could not answer (it panicked) is
/// not restored as if nothing were pending. Its log says so, and an unfinished run is
/// halted, retryably: its pending ops are dropped by the restore, and `run resume`
/// re-issues whatever its tasks need.
fn hold_unreconciled(run: &mut Run, now: u64) {
    let text = "restore: reconcile failed; the run's unfinished ops were not checked";
    run.log.push(LogEntry {
        at: now,
        text: text.to_string(),
    });
    let excess = run.log.len().saturating_sub(LOG_MAX);
    run.log.drain(..excess);
    if run.state.is_terminal() {
        return;
    }
    run.state = RunState::Halted;
    run.paused_from = None;
    run.halted_reason = Some(
        "the daemon could not reconcile this run's unfinished ops at restart; run resume retries them"
            .to_string(),
    );
    run.halt_retryable = true;
}

/// Every engine worktree `run` can have, each with what it is pinned as: its
/// integration worktree (its `HEAD` on the run branch), each task's worktree (detached,
/// with the task's engine-owned branch and private object directory; final fix batch
/// F1b), and its review and proof (detached) worktrees.
fn run_worktree_paths(run: &Run) -> Vec<(std::path::PathBuf, PinAs)> {
    let mut paths = vec![(
        run.integration_path(),
        PinAs {
            head: Some(format!("refs/heads/{}", run.run_branch())),
            ..PinAs::default()
        },
    )];
    for task in &run.tasks {
        // Final fix batch F1c (3a): each checkout is its own repository in the run's
        // data directory.
        let repo = git::Repo::at(&git::checkout_repo_dir(&run.data_dir, &task.worktree));
        paths.push((
            task.worktree.clone(),
            PinAs {
                head: None,
                own: Some(format!("refs/heads/{}", task.branch)),
                objects: Some(repo.objects()),
                engine: Some(repo.engine()),
                repo: Some(repo.git_dir()),
            },
        ));
        for path in [run.review_path(task.id()), run.proof_path(task.id())] {
            let repo = git::Repo::at(&git::checkout_repo_dir(&run.data_dir, &path));
            paths.push((
                path,
                PinAs {
                    repo: Some(repo.git_dir()),
                    ..PinAs::default()
                },
            ));
        }
    }
    paths
}

/// A pending `Accept` whose `Finished` the journal replays: the old daemon merged, and
/// may have died before its clean-up.
fn replayed_accept(run: &Run, answers: &[(String, u64, OpResult)]) -> Option<AcceptCleanUp> {
    answers.iter().find_map(|(_, op, result)| {
        let OpResult::Finished { outcome, .. } = result else {
            return None;
        };
        let OpKind::Accept {
            root,
            worktrees,
            branch_prefix,
            ..
        } = &run.pending_ops.get(op)?.kind
        else {
            return None;
        };
        Some(AcceptCleanUp {
            op: *op,
            merged: outcome.clone(),
            ctx: OpCtx::of(run),
            root: root.clone(),
            worktrees: worktrees.clone(),
            branch_prefix: branch_prefix.clone(),
        })
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(state: RunState) -> Run {
        use crate::run::test_support::{PROFILE, plan_with, run_ok, task_toml};
        let mut run = run_ok(&plan_with(
            PROFILE,
            &[task_toml("t1", "S", "[\"crates/a/**\"]", "")],
        ));
        run.state = state;
        run
    }

    #[test]
    fn an_unreconciled_run_is_held_and_says_so() {
        let mut running = run(RunState::Running);
        hold_unreconciled(&mut running, 7);
        assert_eq!(running.state, RunState::Halted);
        assert!(running.halt_retryable);
        assert!(
            running
                .halted_reason
                .as_deref()
                .unwrap()
                .contains("run resume")
        );
        let last = running.log.last().unwrap();
        assert_eq!(last.at, 7);
        assert!(
            last.text.starts_with("restore: reconcile failed"),
            "{last:?}"
        );

        let mut accepted = run(RunState::Accepted);
        hold_unreconciled(&mut accepted, 7);
        assert_eq!(accepted.state, RunState::Accepted);
        assert!(
            accepted
                .log
                .last()
                .unwrap()
                .text
                .starts_with("restore: reconcile failed")
        );
    }
}
