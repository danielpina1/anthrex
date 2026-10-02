//! Decision 31 (task M9.2.10), step 3 of `review.rs`: a stage's review batch closes
//! `review_batch_secs` after its last thread, and never while a thread waits for its
//! author's permission. Past `review_fix_max` rounds its threads go to the user; a live
//! orchestrator is woken once; otherwise each thread gets a fix task from the template
//! (the fast path), held for approval when it owns files outside the stage (decision
//! 26). A batch waits while its PR is closed or its stage paused (the final fix wave's
//! I-3), and a thread whose fix task was cancelled joins the next batch ([`untask`]).
//! Pure.

use proto::{PrState, TaskState};

use super::super::requests::log;
use super::super::{Effect, wake};
use super::review::{PERMISSION_WAIT_SECS, author, plural, thread_mut};
use super::stage_mut;
use super::watch::named;
use crate::run::delivery::{Batch, PrRecord, ThreadState};
use crate::run::model::{FixOf, Run};

/// Whether the run's orchestrator window is live (M9 decision 13): a planned run's
/// review batch is its to decide; otherwise the engine's template decides.
fn orchestrator_live(run: &Run) -> bool {
    run.orch.orchestrator.as_ref().is_some_and(|o| o.live)
}

/// Step 3: stage `n`'s batch closes when quiet for `review_batch_secs`, and no thread
/// of the stage still waits for its author's permission.
pub(super) fn close(run: &mut Run, n: u16, now: u64, fx: &mut Vec<Effect>) {
    let Some(stage) = run.delivery.stage(n) else {
        return;
    };
    let Some(batch) = stage.batch.clone() else {
        return;
    };
    // The fix round's m4: a login unanswered for `PERMISSION_WAIT_SECS` holds it no more.
    // A thread that counts and is `new` is in the batch already, whatever the answer.
    let asking = (stage.threads.iter()).any(|t| {
        let in_batch = t.state == ThreadState::New && t.counted;
        let due = t.waiting_since.saturating_add(PERMISSION_WAIT_SECS);
        !t.candidates.is_empty() && !in_batch && now < due
    });
    let quiet_at = batch
        .last_at
        .saturating_add(run.delivery.limits.review_batch_secs);
    // The final fix wave's I-3: a batch waits while its PR is closed or its stage is
    // paused, so a reopen or an unpause hands its threads out then.
    let closed = run
        .delivery
        .pr(n)
        .is_some_and(|p| p.state == PrState::Closed);
    if asking || now < quiet_at || closed || stage.paused_by.is_some() {
        return;
    }
    let stage = stage_mut(run, n);
    stage.batch = None;
    stage.batches += 1;
    let b = stage.batches;
    let mut keys = Vec::new();
    for t in stage.threads.iter_mut() {
        let live = t.state == ThreadState::New && t.counted;
        if live && batch.threads.contains(&t.key) {
            t.batch = b;
            keys.push(t.key.clone());
        }
    }
    let Some(pr) = run.delivery.pr(n).cloned() else {
        return;
    };
    if keys.is_empty() || pr.state != PrState::Open {
        return;
    }
    let rounds = run.delivery.stage(n).map_or(0, |s| s.review_rounds);
    let max = run.delivery.limits.review_fix_max;
    if rounds >= max {
        return over_cap(run, n, &pr, &keys, rounds + 1, now);
    }
    let text = format!("{}: review batch {b}: {}", named(n, &pr), keys.join(", "));
    log(run, now, text);
    if orchestrator_live(run) {
        let mut logins: Vec<String> = Vec::new();
        for key in &keys {
            let login = author(run, n, key);
            if !logins.contains(&login) {
                logins.push(login);
            }
        }
        let from: Vec<String> = logins.iter().map(|l| format!("@{l}")).collect();
        let threads = plural(keys.len(), "new review thread", "new review threads");
        wake::note(
            run,
            format!(
                "PR #{} (stage {n}) has {threads} from {}; read them in run_status and add fix tasks, reply, or escalate",
                pr.number,
                from.join(", ")
            ),
        );
        return;
    }
    fast_path(run, n, &pr, &keys, b, now, fx);
}

/// Decision 31's cap: `review_fix_max` rounds were had; each thread is the user's.
fn over_cap(run: &mut Run, n: u16, pr: &PrRecord, keys: &[String], round: u32, now: u64) {
    for key in keys {
        let login = author(run, n, key);
        let line = format!(
            "PR #{}: review round {round} is over the cap; thread {}:{key} by @{login} is yours",
            pr.number, pr.number
        );
        log(run, now, line.clone());
        run.delivery.alerts.insert(format!("{n}/cap/{key}"), line);
    }
}

/// Decision 31's fast path (and the fallback for a planned run whose orchestrator is
/// not live): one fix task per thread from the template; a thread whose fix task
/// already exists never gets a second one. A batch that made one is a review round.
fn fast_path(
    run: &mut Run,
    n: u16,
    pr: &PrRecord,
    keys: &[String],
    b: u32,
    now: u64,
    fx: &mut Vec<Effect>,
) {
    let mut made = false;
    for key in keys {
        let reference = format!("{}:{key}", pr.number);
        let tasked = (run.tasks.iter())
            .filter(|t| t.state != proto::TaskState::Cancelled)
            .any(|t| matches!(&t.fixes, Some(FixOf::Review { threads, .. }) if threads.contains(&reference)));
        let Some(thread) = (!tasked)
            .then(|| {
                run.delivery
                    .stage(n)?
                    .threads
                    .iter()
                    .find(|t| &t.key == key)
            })
            .flatten()
            .cloned()
        else {
            continue;
        };
        match super::review_fix::add_review_fix(run, n, pr, &thread, now, fx) {
            Ok(id) => {
                made = true;
                if let Some(t) = thread_mut(run, n, key) {
                    t.state = ThreadState::Tasked { task: id.clone() };
                }
                super::review_fix::hold_outside(run, n, &id, now);
            }
            Err(message) => {
                let login = author(run, n, key);
                let line = format!(
                    "PR #{}: thread {reference} by @{login} is yours; its fix task was refused: {message}",
                    pr.number
                );
                log(run, now, line.clone());
                run.delivery
                    .alerts
                    .insert(format!("{n}/review/{key}"), line);
            }
        }
    }
    let stage = stage_mut(run, n);
    if made && b > stage.round_batch {
        stage.round_batch = b;
        stage.review_rounds += 1;
    }
}

/// The final fix wave's I-3: whenever a review fix ends cancelled (a `cancel_task`, a
/// rejected hold, a split, its PR closed), each thread it had tasked is `new` again and
/// joins the stage's open batch, so it is handed out again: to a live orchestrator, or
/// to a new fix task (a cancelled task is never resurrected).
pub(super) fn untask(run: &mut Run, n: u16, now: u64) {
    let Some(pr) = run.delivery.pr(n).cloned() else {
        return;
    };
    let ended = |run: &Run, id: &str| run.task(id).is_none_or(|t| t.state == TaskState::Cancelled);
    let keys: Vec<(String, String)> = (run.delivery.stage(n).into_iter())
        .flat_map(|s| s.threads.iter())
        .filter_map(|t| match &t.state {
            ThreadState::Tasked { task } if ended(run, task) => Some((t.key.clone(), task.clone())),
            _ => None,
        })
        .collect();
    for (key, task) in keys {
        if let Some(t) = thread_mut(run, n, &key) {
            t.state = ThreadState::New;
            t.counted = true;
            t.batch = 0;
        }
        let stage = stage_mut(run, n);
        let batch = stage.batch.get_or_insert_with(|| Batch {
            started_at: now,
            last_at: now,
            threads: Vec::new(),
        });
        if !batch.threads.contains(&key) {
            batch.threads.push(key.clone());
        }
        batch.last_at = now;
        let text = format!(
            "{}: thread {key} is new again: its fix task {task} was cancelled",
            named(n, &pr)
        );
        log(run, now, text);
    }
}
