//! Executing one step's effects (decision 43's order), and the work the tick owes:
//! persists, reports and journal compaction. See `driver.rs` for the lock rules.

use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::Instant;

use proto::RunsSnapshot;

use super::{INTERRUPT_GRACE, OpCtx, REPORT_EVERY, RETIRE_AFTER, Retiring, RunService, ops};
use crate::run::engine::{AgentSignal, Effect, EngineState, EventKind, OpKind};
use crate::run::journal::{self, JournalLine};
use crate::run::model::{OpId, Run};
use crate::run::report;
use crate::run::snapshot::snapshot;

/// One effect, with what it needs from the state of the step that emitted it.
pub(super) enum Ready {
    Effect(Effect),
    /// An urgent `Persist`: the run as that step left it.
    Save(Box<Run>),
    /// A counter-only `Persist` (decision 43: at most every 5 s).
    Dirty(String),
    Op {
        ctx: OpCtx,
        op: OpId,
        kind: OpKind,
    },
    /// A structural `Publish`: the snapshot of that step.
    Publish(Box<RunsSnapshot>),
    /// A counter-only `Publish`: at most once a second (decision 47).
    PublishLater,
}

/// Under the engine lock: turns `fx` into [`Ready`] effects, in order. No I/O.
pub(super) fn prepare(state: &EngineState, fx: Vec<Effect>, now: u64) -> Vec<Ready> {
    let mut snap: Option<RunsSnapshot> = None;
    let mut out = Vec::with_capacity(fx.len());
    for effect in fx {
        out.push(match effect {
            Effect::Persist { run_id, urgent } => match state.runs.get(&run_id) {
                Some(run) if urgent => Ready::Save(Box::new(run.clone())),
                _ => Ready::Dirty(run_id),
            },
            Effect::Op { run_id, op, kind } => {
                let Some(run) = state.runs.get(&run_id) else {
                    continue;
                };
                Ready::Op {
                    ctx: OpCtx {
                        run_id,
                        project: run.project.clone(),
                        data_dir: run.data_dir.clone(),
                        git_timeout: std::time::Duration::from_secs(run.limits.git_timeout_secs),
                        check_timeout: std::time::Duration::from_secs(
                            run.profile.check_timeout_secs,
                        ),
                    },
                    op,
                    kind,
                }
            }
            Effect::Publish { structural: true } => Ready::Publish(Box::new(
                snap.get_or_insert_with(|| snapshot(state, now)).clone(),
            )),
            Effect::Publish { structural: false } => Ready::PublishLater,
            other => Ready::Effect(other),
        });
    }
    out
}

/// `journal::save_run` on a blocking thread; a failure is logged (the next persist of
/// the run writes it whole again).
pub(super) async fn save(run: Run) {
    let id = run.id.clone();
    match tokio::task::spawn_blocking(move || journal::save_run(&run)).await {
        Ok(Ok(())) => {}
        Ok(Err(error)) => tracing::error!(run = %id, %error, "could not save run.json"),
        Err(error) => tracing::error!(run = %id, %error, "saving run.json panicked"),
    }
}

impl RunService {
    /// Executes one step's effects in order, the engine lock released.
    pub(super) async fn execute(self: &Arc<Self>, ready: Vec<Ready>, now: u64) {
        for item in ready {
            match item {
                Ready::Save(run) => {
                    crate::lock(&self.book).dirty.remove(&run.id);
                    save(*run).await;
                }
                Ready::Dirty(run_id) => {
                    crate::lock(&self.book).dirty.insert(run_id);
                }
                Ready::Op { ctx, op, kind } => self.start_op(ctx, op, kind).await,
                Ready::Publish(snap) => self.publish(*snap),
                Ready::PublishLater => crate::lock(&self.book).publish_due = true,
                Ready::Effect(effect) => self.apply(effect, now).await,
            }
        }
    }

    async fn apply(self: &Arc<Self>, effect: Effect, now: u64) {
        match effect {
            Effect::Reply { reply, result } => {
                let waiting = crate::lock(&self.replies).remove(&reply);
                if let Some(tx) = waiting {
                    let _ = tx.send(result);
                }
            }
            Effect::Deliver {
                run_id,
                message_ids,
                window_id,
                text,
            } => {
                let service = self.clone();
                tokio::spawn(async move {
                    // Any error, a NUL byte included, is a failed delivery (M8a.18).
                    let (ok, error) = match service.manager.headless_send(window_id, &text).await {
                        Ok(()) => (true, None),
                        Err(error) => (false, Some(error.to_string())),
                    };
                    if !service.stopped.load(Ordering::SeqCst) {
                        service.send(EventKind::Delivered {
                            run_id,
                            message_ids,
                            ok,
                            error,
                        });
                    }
                });
            }
            Effect::Interrupt { window_id } => {
                if let Err(error) = self.manager.headless_interrupt(window_id) {
                    tracing::debug!(window_id, %error, "headless interrupt");
                }
            }
            Effect::KillWindow { window_id } => self.kill_effect(window_id),
            Effect::RetireWindow { window_id } => {
                if let Err(error) = self.manager.headless_retire(window_id) {
                    tracing::debug!(window_id, %error, "headless retire");
                }
                let at = Instant::now();
                crate::lock(&self.book).retiring.insert(
                    window_id,
                    Retiring {
                        kill_at: at + INTERRUPT_GRACE,
                        remove_at: at + RETIRE_AFTER,
                    },
                );
            }
            Effect::RemoveWindow { window_id } => {
                self.forget_window(window_id);
                if let Err(error) = self.manager.remove(window_id) {
                    tracing::debug!(window_id, %error, "remove a run window");
                }
            }
            Effect::WatchWorktree { root } => self.watch_root(&root),
            Effect::UnwatchWorktree { root } => self.unwatch_root(&root),
            Effect::WriteReport { run_id } => {
                crate::lock(&self.book).reports_due.insert(run_id);
                self.write_due_reports(now).await;
            }
            Effect::Persist { .. } | Effect::Op { .. } | Effect::Publish { .. } => {}
        }
    }

    /// `KillWindow`: the group is killed and its process recorded (its exit is then
    /// `killed_by_engine`). A window with no process whose exit the engine still awaits
    /// gets that exit now, so the round ends (a Codex window between turns, a window
    /// whose send was waiting out its jitter: ruling T18-N3).
    fn kill_effect(&self, window_id: u32) {
        let spawned = self.kill_window(window_id);
        if self.manager.child_pid(window_id).ok().flatten().is_some() {
            return;
        }
        let round_pid = {
            let state = crate::lock(&self.state);
            state
                .runs
                .values()
                .flat_map(|run| run.tasks.iter())
                .flat_map(|task| task.rounds.iter())
                .rfind(|round| round.window_id == Some(window_id))
                .and_then(|round| round.pid)
        };
        if round_pid.is_some() && round_pid == spawned {
            // That process's own exit is on its way, and is marked as the engine's.
            return;
        }
        self.send(EventKind::Signal {
            window_id,
            signal: AgentSignal::ProcessExited {
                code: None,
                killed_by_engine: true,
                pid: round_pid.unwrap_or(0),
            },
        });
    }

    /// Decision 43: the intent line (fsynced), decision 48's crash injection, then the
    /// op on its own task, whose result is journaled before its `OpDone` is sent.
    async fn start_op(self: &Arc<Self>, ctx: OpCtx, op: OpId, kind: OpKind) {
        let order = self.op_lock(&ctx.run_id);
        let exclusive = matches!(kind, OpKind::Accept { .. } | OpKind::Discard { .. });
        // Taken now, in emission order, so a later `Accept` or `Discard` waits for it.
        let shared = if exclusive {
            None
        } else {
            order.clone().try_read_owned().ok()
        };
        let name = kind.name();
        let line = JournalLine::Intent {
            op,
            kind: kind.clone(),
        };
        self.append(&ctx, line).await;
        self.crash_injection(name);
        let service = self.clone();
        tokio::spawn(async move {
            // Held for the life of the op.
            let _order = match (exclusive, shared) {
                (true, _) => (None, Some(order.write_owned().await)),
                (false, Some(guard)) => (Some(guard), None),
                (false, None) => (Some(order.read_owned().await), None),
            };
            let result = ops::run(&service, &ctx, kind).await;
            if service.stopped.load(Ordering::SeqCst) {
                return;
            }
            let line = JournalLine::Done {
                op,
                result: result.clone(),
            };
            service.append(&ctx, line).await;
            service.send(EventKind::OpDone {
                run_id: ctx.run_id,
                op,
                result,
            });
        });
    }

    /// Decision 48 (debug builds): abort after the n-th intent line of one kind.
    fn crash_injection(&self, name: &'static str) {
        let Some((kind, n)) = &self.abort_after else {
            return;
        };
        if kind != name {
            return;
        }
        let count = {
            let mut book = crate::lock(&self.book);
            let count = book.intents.entry(name).or_insert(0);
            *count += 1;
            *count
        };
        if count == *n {
            tracing::warn!(kind = name, n, "ANTHREX_TEST_ABORT_AFTER_INTENT: aborting");
            std::process::abort();
        }
    }

    /// Appends one journal line, fsynced, on a blocking thread.
    async fn append(&self, ctx: &OpCtx, line: JournalLine) {
        let lock = self.journal.clone();
        let dir = ctx.data_dir.clone();
        let appended = tokio::task::spawn_blocking(move || {
            let _held = crate::lock(&lock);
            journal::append(&dir, &line)
        })
        .await;
        match appended {
            Ok(Ok(())) => {}
            Ok(Err(error)) => tracing::error!(run = %ctx.run_id, %error, "journal append failed"),
            Err(error) => tracing::error!(run = %ctx.run_id, %error, "journal append panicked"),
        }
    }

    /// Writes every due report not written in the last [`REPORT_EVERY`]: a temp file,
    /// then a rename.
    pub(super) async fn write_due_reports(&self, now: u64) {
        let due: Vec<Run> = {
            let mut book = crate::lock(&self.book);
            let ready: Vec<String> = book
                .reports_due
                .iter()
                .filter(|id| {
                    book.reports_written
                        .get(*id)
                        .is_none_or(|at| at.elapsed() >= REPORT_EVERY)
                })
                .cloned()
                .collect();
            for id in &ready {
                book.reports_due.remove(id);
                book.reports_written.insert(id.clone(), Instant::now());
            }
            drop(book);
            let state = crate::lock(&self.state);
            ready
                .iter()
                .filter_map(|id| state.runs.get(id).cloned())
                .collect()
        };
        for run in due {
            let id = run.id.clone();
            let written = tokio::task::spawn_blocking(move || {
                let text = report::render(&run, now);
                let path = run.report_path();
                std::fs::create_dir_all(&run.data_dir)?;
                let tmp = run.data_dir.join("REPORT.md.tmp");
                std::fs::write(&tmp, text)?;
                std::fs::rename(&tmp, &path)
            })
            .await;
            if !matches!(written, Ok(Ok(()))) {
                tracing::error!(run = %id, ?written, "could not write the report");
            }
        }
    }

    /// Decision 43: a journal past 1 MiB is rewritten with only its pending ops.
    pub(super) async fn compact_journals(&self) {
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
            for (id, dir, pending) in runs {
                if !journal::needs_compact(&dir) {
                    continue;
                }
                let _held = crate::lock(&lock);
                match journal::compact(&dir, &pending) {
                    Ok(problems) => {
                        for problem in problems {
                            tracing::warn!(run = %id, %problem, "journal compacted");
                        }
                    }
                    Err(error) => tracing::error!(run = %id, %error, "journal compaction failed"),
                }
            }
        })
        .await;
    }
}
