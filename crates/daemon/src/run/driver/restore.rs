//! Decision 44 on start (M8a.22): load every run, reconcile each unfinished op against
//! the journal and reality, step `Event::Restore` before any other event, then compact
//! the journals, watch the runs' live worktrees again, remove the restored windows no
//! live run has, and finish an accept whose merge the old daemon completed.

use std::collections::BTreeSet;
use std::sync::Arc;
use std::time::Duration;

use super::{OpCtx, RunService, cleanup, effects, unix_now};
use crate::run::engine::{Event, EventKind, OpKind, OpResult, step};
use crate::run::journal;
use crate::run::model::{LogEntry, Run};
use crate::run::reconcile;

/// The engine's cap on a run's log (Interfaces, `LogEntry`).
const LOG_MAX: usize = 500;

/// What a replayed `Accept` still owes: its clean-up (M8a.21's concern).
struct AcceptCleanUp {
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
        let mut cleanups = Vec::new();
        for (mut run, lines) in loaded {
            let (git, windows) = (self.ctx.git.clone(), windows.clone());
            let timeout = Duration::from_secs(run.limits.git_timeout_secs);
            let snapshot = run.clone();
            let checked = tokio::task::spawn_blocking(move || {
                reconcile::reconcile(&git, &snapshot, &lines, &windows, timeout)
            })
            .await;
            let Ok(reconciled) = checked else {
                tracing::error!(run = %run.id, "reconcile panicked; restoring without it");
                runs.push(run);
                continue;
            };
            let answers = reconciled.replay(&run.id);
            for note in reconciled.notes {
                run.log.push(LogEntry {
                    at: now,
                    text: format!("restore: {note}"),
                });
            }
            let excess = run.log.len().saturating_sub(LOG_MAX);
            run.log.drain(..excess);
            cleanups.extend(replayed_accept(&run, &answers));
            replay.extend(answers);
            runs.push(run);
        }
        if runs.is_empty() {
            return;
        }
        let prepared = {
            let mut state = crate::lock(&self.state);
            let (next, fx) = step(
                std::mem::take(&mut *state),
                Event {
                    now,
                    kind: EventKind::Restore { runs, replay },
                },
            );
            *state = next;
            effects::prepare(&state, fx, now)
        };
        self.execute(prepared, now).await;
        self.compact_after_restore().await;
        self.watch_live_worktrees();
        self.remove_stale_windows();
        for cleanup in cleanups {
            let service = self.clone();
            tokio::spawn(async move {
                let (outcome, kept) = cleanup::clean_up(
                    &service,
                    &cleanup.ctx,
                    cleanup.root,
                    &cleanup.worktrees,
                    cleanup.branch_prefix,
                )
                .await;
                tracing::info!(run = %cleanup.ctx.run_id, %outcome, ?kept, "accept clean-up after restore");
            });
        }
    }

    /// Every restored run's journal, rewritten with only its pending ops (decision 43,
    /// M8a.21's "compacting the journal after the restore"). The restore's `Persist`
    /// has been written by now.
    async fn compact_after_restore(&self) {
        let runs: Vec<_> = {
            let state = crate::lock(&self.state);
            state
                .runs
                .values()
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
    /// removed; only the engine can remove one (decision 49).
    fn remove_stale_windows(&self) {
        let live: BTreeSet<String> = {
            let state = crate::lock(&self.state);
            state
                .runs
                .values()
                .filter(|run| !run.state.is_terminal())
                .map(|run| run.id.clone())
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

/// A pending `Accept` whose `Finished` the journal replays: the old daemon merged, and
/// may have died before its clean-up.
fn replayed_accept(run: &Run, answers: &[(String, u64, OpResult)]) -> Option<AcceptCleanUp> {
    answers.iter().find_map(|(_, op, result)| {
        let OpResult::Finished { .. } = result else {
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
            ctx: OpCtx {
                run_id: run.id.clone(),
                project: run.project.clone(),
                data_dir: run.data_dir.clone(),
                git_timeout: Duration::from_secs(run.limits.git_timeout_secs),
                check_timeout: Duration::from_secs(run.profile.check_timeout_secs),
            },
            root: root.clone(),
            worktrees: worktrees.clone(),
            branch_prefix: branch_prefix.clone(),
        })
    })
}
