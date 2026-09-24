//! Decision 35's review gate (M8a.13): each round a fresh, read-only reviewer session
//! in a fresh review worktree, `submit_review` with severities, the rejection's rung-1
//! message, the nudge and replacement of a reviewer that ends without a verdict, and a
//! reviewer's exits and silence. Reviewer messages (the nudge) go through the run's
//! outbox addressed to the mailbox `<task>.review`: task ids cannot contain `.`
//! (decision 16), so every worker-side rule that matches messages by task id leaves
//! them alone. Pure (design decision 2).

use proto::{
    AgentRole, BlockReason, GateKind, RunState, Runtime, Severity, TaskState, ToolCall, Verdict,
};

use super::dispatch::{block, history, new_round, window_limit_reached};
use super::schedule::{hub_holds_slot, needs_reviewer, readers_busy};
use super::signals::{count_rate_limit, end_round};
use super::tools::parse_review;
use super::{
    Effect, OpId, OpKind, OpResult, ReplyId, TurnOutcome, emit_op, gates, ladder, next_op, outbox,
};
use crate::headless::FailureKind;
use crate::run::contract::{
    APPROVE_WITH_BLOCKING, REVIEW_NUDGE, REVIEW_RECORDED, REVIEWER_RESUME_AFTER_EXIT,
    REVIEWER_STOPPED_TWICE, rate_limit_continue, review_changes_message, reviewer_prompt,
};
use crate::run::model::{AgentRound, FailedTurn, ReviewLevel, ReviewRecord, Run};
use crate::run::role_launch::{jitter_ms, reviewer_spec, session_uuid};
use crate::run::roster::pick_reviewer;

/// The outbox address of task `task`'s reviewer.
pub(super) fn mailbox(task: &str) -> String {
    format!("{task}.review")
}

/// The task whose reviewer `address` is, if it is a reviewer mailbox.
pub(super) fn mailbox_task(address: &str) -> Option<&str> {
    address.strip_suffix(".review")
}

/// Task `i`'s current reviewer round, if it has one.
pub(super) fn reviewer_round(run: &Run, i: usize) -> Option<usize> {
    run.tasks[i]
        .rounds
        .iter()
        .rposition(|r| r.role == AgentRole::Reviewer)
}

/// Reader dispatch: a `PrepareReview` for each task in `review` without a reviewer,
/// while reader slots are free (decision 41). A hub task holding a writer slot blocks
/// every dispatch, reviews included. The task awaits that op (`gate_op`).
pub(super) fn dispatch_reviewers(run: &mut Run, fx: &mut Vec<Effect>) {
    if hub_holds_slot(run) {
        return;
    }
    for i in super::schedule::dispatch_order(run) {
        if readers_busy(run) >= usize::from(run.limits.max_readers) {
            break;
        }
        if !needs_reviewer(run, &run.tasks[i]) || run.tasks[i].gate_op.is_some() {
            continue;
        }
        let op = next_op(run);
        let task = &run.tasks[i];
        let kind = OpKind::PrepareReview {
            root: run.root.clone(),
            // Ruling T13-I3: the claimed commit, never the branch tip.
            head_ref: task.head.clone().unwrap_or_else(|| task.branch.clone()),
            base_ref: task
                .start_commit
                .clone()
                .unwrap_or_else(|| run.run_head.clone()),
            path: run.review_path(task.id()),
        };
        let id = task.id().to_string();
        run.tasks[i].gate_op = Some(op);
        emit_op(run, op, Some(&id), kind, fx);
    }
}

/// The result of the `PrepareReview` the task awaits: a fresh reviewer session
/// (decision 35) on `pick_reviewer` against the author's current route, and the
/// round's `ReviewRecord`, whose verdict `submit_review` fills. Reviewers are
/// read-only and their worktree is never watched.
pub(super) fn review_ready(
    run: &mut Run,
    i: usize,
    op: OpId,
    result: OpResult,
    now: u64,
    fx: &mut Vec<Effect>,
) {
    if run.tasks[i].gate_op != Some(op) {
        return;
    }
    run.tasks[i].gate_op = None;
    if run.tasks[i].state != TaskState::Review {
        return;
    }
    let (base, head, patch) = match result {
        OpResult::Review { base, head, patch } => (base, head, patch),
        OpResult::Failed { message } => {
            let text = format!("could not prepare the review worktree: {message}");
            return block(run, i, BlockReason::Environment, text, now);
        }
        _ => return,
    };
    // Ruling T14-I2: no session starts while the run is not running. The worktree is
    // ready; the running pass prepares the review again (cheap, and its diff is not
    // kept in the run) and starts the reviewer then.
    if run.state != RunState::Running {
        let text = "review worktree prepared; the reviewer starts once the run runs";
        return history(run, i, now, text);
    }
    if window_limit_reached(run, i, now) {
        return;
    }
    let op = next_op(run);
    let task = &run.tasks[i];
    let level = task.review_level.unwrap_or(ReviewLevel::Medium);
    // The author's route may have escalated (rung 2): the reviewer is picked against
    // the current one, so it stays on the other runtime.
    let route = pick_reviewer(&run.roster, &task.route, level);
    let spec = reviewer_spec(run, task, &route);
    let round_no = spec.run_ref.as_ref().map_or(1, |r| r.session);
    let first_turn = reviewer_prompt(run, task, round_no, &base, &head, &patch);
    let name = format!("{}/{}.r{round_no}", run.short(), task.id());
    let uuid = (route.runtime == Runtime::Claude).then(|| session_uuid(&run.id, op));
    let jitter = jitter_ms(&run.id, &format!("{}.r", task.id()), round_no);
    let mut round = new_round(
        AgentRole::Reviewer,
        round_no,
        route.clone(),
        op,
        uuid.clone(),
        now,
    );
    round.round = round_no;
    let id = task.id().to_string();
    let worktree = run.review_path(&id);
    let task = &mut run.tasks[i];
    task.review_route = Some(route.clone());
    task.rounds.push(round);
    task.reviews.push(ReviewRecord {
        round: round_no,
        route,
        base,
        head,
        verdict: None,
        summary: String::new(),
        findings: Vec::new(),
    });
    run.windows_created += 1;
    history(run, i, now, format!("review round {round_no} starting"));
    let kind = OpKind::CreateWindow {
        name,
        spec: Box::new(spec),
        session_uuid: uuid,
        first_turn,
        project: run.project.clone(),
        worktree,
        jitter_ms: jitter,
    };
    emit_op(run, op, Some(&id), kind, fx);
}

/// Stops task `i`'s live reviewer (a round given up, or a task overridden): the
/// engine's kill, and the mailbox and any resume it awaited dropped.
pub(super) fn stop_reviewers(run: &mut Run, i: usize, now: u64, fx: &mut Vec<Effect>) {
    for round in run.tasks[i]
        .rounds
        .iter_mut()
        .filter(|r| r.role == AgentRole::Reviewer && !r.retiring)
    {
        give_up(round, now, fx);
    }
    drop_mail(run, i);
}

/// The engine gives up a reviewer round: its process is killed, and the round holds
/// its reader slot until that exit (ruling T13-I2). A Codex reviewer whose last
/// process has exited (one per turn) has nothing to kill, so its round ends at once.
fn give_up(round: &mut AgentRound, now: u64, fx: &mut Vec<Effect>) {
    round.retiring = true;
    round.resume_op = None;
    if round.ended {
        return;
    }
    // Ruling T13-R2 (N1): Codex emits `turn.completed` before its process exits, so a
    // closed turn is not enough: only a round whose last process has exited (no pid)
    // has nothing to wait for.
    if round.route.runtime == Runtime::Codex && !round.turn_open && round.pid.is_none() {
        return end_round(round, now);
    }
    if let Some(window_id) = round.window_id {
        fx.push(Effect::KillWindow { window_id });
    }
}

fn drop_mail(run: &mut Run, i: usize) {
    let address = mailbox(run.tasks[i].id());
    run.outbox.retain(|m| m.task_id != address);
}

fn reply(fx: &mut Vec<Effect>, reply: ReplyId, result: Result<String, String>) {
    fx.push(Effect::Reply { reply, result });
}

/// `submit_review` (decision 35 and the MCP section's acceptance), in order: the
/// caller is a reviewer round of the task, its round has no verdict yet, it is the
/// task's live reviewer, the task is under review, the arguments are valid, and an
/// `approve` carries no blocking finding. The verdict is recorded and the reviewer
/// retired; any critical or important finding is a gate failure of `review`, else
/// the task goes to the merge queue (minor findings stay in the record).
pub(super) fn submit(run: &mut Run, id: ReplyId, call: &ToolCall, now: u64, fx: &mut Vec<Effect>) {
    let task_id = call.task_id.clone().unwrap_or_default();
    let not_reviewer = format!("this window is not the reviewer of task {task_id}");
    let Some(i) = run.tasks.iter().position(|t| t.id() == task_id) else {
        return reply(fx, id, Err(not_reviewer));
    };
    let found = (call.role == AgentRole::Reviewer)
        .then(|| {
            run.tasks[i]
                .rounds
                .iter()
                .rposition(|r| r.role == AgentRole::Reviewer && r.window_id == Some(call.window_id))
        })
        .flatten();
    let Some(r) = found else {
        return reply(fx, id, Err(not_reviewer));
    };
    let round_no = run.tasks[i].rounds[r].round;
    let decided = |rv: &ReviewRecord| rv.round == round_no && rv.verdict.is_some();
    if run.tasks[i].reviews.iter().any(decided) {
        let text = format!("a review for round {round_no} was already submitted");
        return reply(fx, id, Err(text));
    }
    let round = &run.tasks[i].rounds[r];
    if reviewer_round(run, i) != Some(r) || round.retiring || round.ended {
        return reply(fx, id, Err(not_reviewer));
    }
    let state = run.tasks[i].state;
    if state != TaskState::Review {
        let text = format!(
            "submit_review is accepted only while the task is under review (it is {})",
            state.label()
        );
        return reply(fx, id, Err(text));
    }
    let (verdict, summary, findings) = match parse_review(&call.args) {
        Ok(parsed) => parsed,
        Err(e) => return reply(fx, id, Err(format!("invalid arguments: {e}"))),
    };
    let blocking = findings.iter().any(|f| f.severity != Severity::Minor);
    if verdict == Verdict::Approve && blocking {
        return reply(fx, id, Err(APPROVE_WITH_BLOCKING.to_string()));
    }
    let task = &mut run.tasks[i];
    let route = task.rounds[r].route.clone();
    let k = match task.reviews.iter().position(|rv| rv.round == round_no) {
        Some(k) => k,
        None => {
            task.reviews.push(ReviewRecord {
                round: round_no,
                route,
                base: String::new(),
                head: String::new(),
                verdict: None,
                summary: String::new(),
                findings: Vec::new(),
            });
            task.reviews.len() - 1
        }
    };
    let record = &mut task.reviews[k];
    record.verdict = Some(verdict);
    record.summary = summary;
    record.findings = findings;
    let record = record.clone();
    task.review_misses = 0;
    let round = &mut task.rounds[r];
    round.retiring = true;
    round.resume_op = None;
    if let Some(window_id) = round.window_id {
        fx.push(Effect::RetireWindow { window_id });
    }
    drop_mail(run, i);
    reply(fx, id, Ok(REVIEW_RECORDED.to_string()));
    if blocking {
        history(
            run,
            i,
            now,
            format!("review round {round_no} asked for changes"),
        );
        let text = review_changes_message(&record);
        ladder::gate_failure(run, i, GateKind::Review, text, false, now, fx);
    } else {
        history(run, i, now, format!("review round {round_no} approved"));
        gates::enter(run, i, TaskState::MergeQueue);
    }
}

/// Whether round `r` of task `i` is its reviewer still owing a verdict: the current
/// reviewer round of a task under review, not given up, with no verdict recorded.
pub(super) fn owes_verdict(run: &Run, i: usize, r: usize) -> bool {
    let task = &run.tasks[i];
    let round = &task.rounds[r];
    task.state == TaskState::Review
        && reviewer_round(run, i) == Some(r)
        && !round.retiring
        && !task
            .reviews
            .iter()
            .any(|rv| rv.round == round.round && rv.verdict.is_some())
}

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
/// turn); authentication and billing failures block the task on its environment at
/// once, with the error; any other failure gets one continue after the same wait, and
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
            return block(run, i, BlockReason::Environment, error, now);
        }
    };
    round.failed_turn = FailedTurn::WaitingContinue {
        at: now + wait,
        rate_limit,
    };
    if rate_limit {
        round.rate_limited_until = Some(now + wait);
    }
    round.failed_error = Some(error);
    // One event, unless a retry streak ran straight into this failure (decision 32).
    if rate_limit && !streak {
        count_rate_limit(run, i, r);
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
        return verdictless(run, i, r, "its process exited twice in one round", now, fx);
    }
    let Some((window_id, session_id)) = round.window_id.zip(round.session_id.clone()) else {
        end_round(round, now);
        let why = "its process exited before its session started";
        return verdictless(run, i, r, why, now, fx);
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
            round.rate_limited_until = None;
            let reason = round.failed_error.clone().unwrap_or_default();
            let address = mailbox(run.tasks[i].id());
            outbox::queue_to(run, &address, r, rate_limit_continue(&reason), now);
        }
        let round = &run.tasks[i].rounds[r];
        if round.ended || !round.turn_open {
            continue;
        }
        let quiet = round.last_event.max(round.rate_limited_until.unwrap_or(0));
        let stall_after = run.limits.stall_after_secs;
        if now >= quiet + stall_after {
            let why = format!("no stream event for {} minutes", stall_after / 60);
            verdictless(run, i, r, &why, now, fx);
        }
    }
}
