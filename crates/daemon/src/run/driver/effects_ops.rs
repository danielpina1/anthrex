//! Starting one op (decision 43): its intent line, decision 48's crash injection, the
//! op on its own task, then its `done` line and its `OpDone`. Moved from `effects.rs`
//! (M9.11.13, move-only).

use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::Duration;

use super::{OpCtx, RunService};
use crate::run::engine::{EventKind, OpKind, OpResult};
use crate::run::journal::{self, JournalLine};
use crate::run::model::OpId;

impl RunService {
    /// Decision 43: the intent line (fsynced), decision 48's crash injection, then the
    /// op on its own task, whose result is journaled before its `OpDone` is sent.
    pub(super) async fn start_op(self: &Arc<Self>, ctx: OpCtx, op: OpId, kind: OpKind) {
        let order = self.op_lock(&ctx.run_id);
        let name = kind.name();
        // Left when the op's task ends, however it ends (`stop_and_settle`).
        let entered = self.in_flight.enter(&ctx.run_id, op, name);
        let exclusive = matches!(kind, OpKind::Accept { .. } | OpKind::Discard { .. });
        // Taken now, in emission order, so a later `Accept` or `Discard` waits for it.
        let shared = if exclusive {
            None
        } else {
            order.clone().try_read_owned().ok()
        };
        let hold = self.done_holds.of(&kind);
        let line = JournalLine::Intent {
            op,
            kind: kind.clone(),
        };
        if !self.append(&ctx, line).await {
            return self.not_started(ctx, op, "its intent line could not be appended");
        }
        self.crash_injection(name);
        let service = self.clone();
        tokio::spawn(async move {
            let _entered = entered;
            // Held for the life of the op.
            let _order = match (exclusive, shared) {
                (true, _) => (None, Some(order.write_owned().await)),
                (false, Some(guard)) => (Some(guard), None),
                (false, None) => (Some(order.read_owned().await), None),
            };
            // Milestone 9.2 (task M9.2.10's fix round): a PR view is cut to what
            // run.json keeps before it is journaled and handed to the engine.
            let result =
                crate::run::delivery::view_trim::journaled(service.run_op(&ctx, op, kind).await);
            #[cfg(test)]
            if let Some(hook) = crate::lock(&service.after_op).as_ref() {
                hook(&service, name);
            }
            // Milestone 9.11 decision 19 (B6): a session process now exists, so its pid
            // goes to the journal even while the daemon stops, where restore finds it
            // (`reconcile::sessions`); only the engine send is skipped then.
            let spawned = matches!(result, OpResult::Window { pid: Some(_), .. });
            if service.stopped.load(Ordering::SeqCst) && !spawned {
                return;
            }
            // The final fix wave's A4: a CI log's text stays out of the journal.
            let line = JournalLine::Done {
                op,
                result: crate::run::delivery::view_trim::journal_line(&result),
            };
            service.append_done(&ctx, line, hold).await;
            if service.stopped.load(Ordering::SeqCst) {
                return;
            }
            service.send(EventKind::OpDone {
                run_id: ctx.run_id,
                op,
                result,
            });
        });
    }

    /// Final review B-3: an op the journal cannot know of is not started (decision 43);
    /// it is answered `Failed`, which the engine handles as any failed op.
    pub(super) fn not_started(&self, ctx: OpCtx, op: OpId, why: &str) {
        tracing::error!(run = %ctx.run_id, op, why, "an op was not started");
        self.send(EventKind::OpDone {
            run_id: ctx.run_id,
            op,
            result: crate::run::engine::OpResult::Failed {
                message: format!("could not journal the op ({why}); it was not started"),
            },
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

    /// An op's `done` line, after its test hold (`DoneHolds`), if any.
    async fn append_done(&self, ctx: &OpCtx, line: JournalLine, hold: Option<Duration>) {
        if let Some(hold) = hold {
            tokio::time::sleep(hold).await;
        }
        self.append(ctx, line).await;
    }

    /// Appends one journal line, fsynced, on a blocking thread. `false` when it failed.
    pub(super) async fn append(&self, ctx: &OpCtx, line: JournalLine) -> bool {
        let lock = self.journal.clone();
        let dir = ctx.data_dir.clone();
        let appended = tokio::task::spawn_blocking(move || {
            let _held = crate::lock(&lock);
            journal::append(&dir, &line)
        })
        .await;
        match appended {
            Ok(Ok(())) => true,
            Ok(Err(error)) => {
                tracing::error!(run = %ctx.run_id, %error, "journal append failed");
                false
            }
            Err(error) => {
                tracing::error!(run = %ctx.run_id, %error, "journal append panicked");
                false
            }
        }
    }
}
