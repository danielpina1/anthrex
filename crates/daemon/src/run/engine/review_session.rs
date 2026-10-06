//! Decision 35's reviewer sessions (M8a.13): a reviewer's turn ends, failed turns,
//! exits and resumes, the verdict-less round, and the reviewer's watchdog. Pure (design
//! decision 2).

use proto::BlockReason;

use super::clock::{not_before, stall_due};
use super::dispatch::{block, history};
use super::review::{drop_mail, give_up, mailbox, owes_verdict, reviewer_round, stop_reviewers};
use super::signals::{count_rate_limit, end_round};
use super::{Effect, OpKind, TurnOutcome, emit_op, next_op, outbox};
use crate::headless::FailureKind;
use crate::run::contract::{
    REVIEW_NUDGE, REVIEWER_RESUME_AFTER_EXIT, REVIEWER_STOPPED_TWICE, rate_limit_continue,
};
use crate::run::model::{FailedTurn, Run};
use crate::run::role_launch::jitter_ms;

/// A reviewer's turn ended (decision 35): with no verdict, `REVIEW_NUDGE` is one more
/// turn; a second verdict-less turn ends the round. A failed turn follows decision 32's
/// failed-turn rules, as a worker's does (ruling T13-I1), and is no verdict-less turn.
#[allow(clippy::too_many_arguments)]
pub(super) fn turn_ended(
    run: &mut Run,
    i: usize,
    r: usize,
    outcome: TurnOutcome,
    streak: bool,
    now: u64,
    fx: &mut Vec<Effect>,
) {
    if !owes_verdict(run, i, r) {
        return;
    }
    let round = &mut run.tasks[i].rounds[r];
    if let TurnOutcome::Failed { error, kind } = outcome {
        return failed_turn(run, i, r, error, kind, streak, now, fx);
    }
    if matches!(round.failed_turn, FailedTurn::ContinueSent { .. }) {
        round.failed_turn = FailedTurn::None;
        round.failed_error = None;
    }
    if round.review_nudged {
        return verdictless(run, i, r, "its turn ended twice without a verdict", now, fx);
    }
    round.review_nudged = true;
    let address = mailbox(run.tasks[i].id());
    outbox::queue_to(run, &address, r, REVIEW_NUDGE.to_string(), now);
}

/// Ruling T13-I1: decision 32's failed turns for a reviewer. A rate limit is counted
/// and waited out (`rate_limit_retry_secs`, then `rate_limit_continue` as its next
/// turn); authentication and billing failures, and ruling F-1's client errors, block the
/// task on its environment at once, with the error; any other failure gets one continue after the same wait, and
/// the second in a row blocks. A reviewer has no sandbox, so an unavailable one is
/// treated as an environment failure too. The reviewer of a blocked task is stopped.
#[allow(clippy::too_many_arguments)]
fn failed_turn(
    run: &mut Run,
    i: usize,
    r: usize,
    error: String,
    kind: FailureKind,
    streak: bool,
    now: u64,
    fx: &mut Vec<Effect>,
) {
    let wait = run.limits.rate_limit_retry_secs;
    let round = &mut run.tasks[i].rounds[r];
    let rate_limit = match kind {
        FailureKind::RateLimit => true,
        FailureKind::Other
            if !matches!(
                round.failed_turn,
                FailedTurn::ContinueSent { rate_limit: false }
            ) =>
        {
            false
        }
        _ => {
            stop_reviewers(run, i, now, fx);
            run.tasks[i].rounds[r].environment_failed = true;
            return block(run, i, BlockReason::Environment, error, now);
        }
    };
    round.failed_turn = FailedTurn::WaitingContinue {
        at: not_before(now, wait),
        rate_limit,
    };
    if rate_limit {
        round.set_rate_limited(Some(not_before(now, wait)), now);
    }
    super::rounds::note_failed_turn(run, i, r, &error, not_before(now, wait), now);
    run.tasks[i].rounds[r].failed_error = Some(error);
    // One event, unless a retry streak ran straight into this failure (decision 32).
    if rate_limit && !streak {
        count_rate_limit(run, i, r, now);
    }
}

/// A reviewer's process exited without the engine killing it (decision 35, with
/// decision 32's resume rule): a Claude reviewer between turns is marked ended, and a
/// delivery resumes it; mid-turn, the first death in the round resumes the session and
/// the second ends the round without a verdict. A round given up, or a task no longer
/// under review, just ends.
pub(super) fn exited(
    run: &mut Run,
    i: usize,
    r: usize,
    killed: bool,
    now: u64,
    fx: &mut Vec<Effect>,
) {
    let owes = owes_verdict(run, i, r);
    let round = &mut run.tasks[i].rounds[r];
    if killed || !owes || !round.turn_open {
        return end_round(round, now);
    }
    round.deaths = round.deaths.saturating_add(1);
    if round.deaths >= 2 {
        end_round(round, now);
        let why = super::signals::exit_reason(round, "its process exited twice in one round");
        return verdictless(run, i, r, &why, now, fx);
    }
    let Some((window_id, session_id)) = round.window_id.zip(round.session_id.clone()) else {
        end_round(round, now);
        let why = "its process exited before its session started";
        let why = super::signals::exit_reason(round, why);
        return verdictless(run, i, r, &why, now, fx);
    };
    round.last_event = now;
    let id = run.tasks[i].id().to_string();
    let kind = OpKind::ResumeSession {
        window_id,
        session_id,
        message: REVIEWER_RESUME_AFTER_EXIT.to_string(),
        jitter_ms: jitter_ms(&run.id, &format!("{id}.r"), round_no(run, i, r)),
    };
    let op = next_op(run);
    run.tasks[i].rounds[r].resume_op = Some(op);
    emit_op(run, op, Some(&id), kind, fx);
    history(
        run,
        i,
        now,
        "its reviewer's process exited mid-turn; resuming it",
    );
}

fn round_no(run: &Run, i: usize, r: usize) -> u32 {
    run.tasks[i].rounds[r].round
}

/// A reviewer's `ResumeSession` failed: the round ends without a verdict.
pub(super) fn resume_failed(
    run: &mut Run,
    i: usize,
    r: usize,
    error: &str,
    now: u64,
    fx: &mut Vec<Effect>,
) {
    end_round(&mut run.tasks[i].rounds[r], now);
    if owes_verdict(run, i, r) {
        let why = format!("its session could not be resumed: {error}");
        verdictless(run, i, r, &why, now, fx);
    }
}

/// The round ends without a verdict (decision 35): the reviewer is stopped and a new
/// round starts at the same level, with no failure counted; the second such round in a
/// row blocks the task as `blocked(environment)`.
fn verdictless(run: &mut Run, i: usize, r: usize, why: &str, now: u64, fx: &mut Vec<Effect>) {
    give_up(&mut run.tasks[i].rounds[r], now, fx);
    drop_mail(run, i);
    let task = &mut run.tasks[i];
    task.review_misses = task.review_misses.saturating_add(1);
    let misses = task.review_misses;
    let no = task.rounds[r].round;
    history(
        run,
        i,
        now,
        format!("review round {no} ended without a verdict: {why}"),
    );
    if misses >= 2 {
        run.tasks[i].review_misses = 0;
        block(
            run,
            i,
            BlockReason::Environment,
            REVIEWER_STOPPED_TWICE.to_string(),
            now,
        );
    }
}

/// The reviewer's watchdog, on every scheduler pass of a running run: an open reviewer
/// turn with no stream event for `stall_after_secs` (suspended while a rate-limit
/// retry is pending) ends the round without a verdict (invented: decision 32's
/// interrupt-and-nudge is the worker's; a reviewer has its nudge already).
pub(super) fn watch(run: &mut Run, now: u64, fx: &mut Vec<Effect>) {
    for i in 0..run.tasks.len() {
        let Some(r) = reviewer_round(run, i) else {
            continue;
        };
        if !owes_verdict(run, i, r) {
            continue;
        }
        // Ruling T13-I1: a failed turn's continue, once its wait is over.
        let round = &mut run.tasks[i].rounds[r];
        if let FailedTurn::WaitingContinue { at, rate_limit } = round.failed_turn
            && now >= at
        {
            round.failed_turn = FailedTurn::ContinueSent { rate_limit };
            round.set_rate_limited(None, now);
            let reason = round.failed_error.clone().unwrap_or_default();
            let address = mailbox(run.tasks[i].id());
            outbox::queue_to(run, &address, r, rate_limit_continue(&reason), now);
        }
        let round = &run.tasks[i].rounds[r];
        if round.ended || !round.turn_open {
            continue;
        }
        let stall_after = run.limits.stall_after_secs;
        if now >= stall_due(round, stall_after) {
            let why = format!("no stream event for {} minutes", stall_after / 60);
            verdictless(run, i, r, &why, now, fx);
        }
    }
}
