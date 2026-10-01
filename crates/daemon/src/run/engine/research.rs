//! Milestone 9 decision 35, engine side: a research task's session. It is an M8a task
//! round of role `Scout`, launched read-only in the user's checkout, whose mailbox is
//! `<task>.research`. Its `submit_scout_report` ends the task `reported`; a turn with
//! no report gets `SCOUT_NUDGE` once and the second blocks it. Budgets, stalls, deaths
//! and failed turns follow M8a's worker rules (decision 32's rate-limit wait and
//! continue, the interrupt-and-nudge stall ladder, decision 38's stall and breach
//! counts, decision 40's budgets), with a fresh research session where a worker gets
//! rung 2's (M9.9 review fixes, I3). Pure (design decision 2).

use proto::{AgentRole, BlockReason, RunState, Runtime, ScoutKind, Spend, TaskState, ToolCall};

use super::clock::{not_before, stall_due};
use super::dispatch::{block, history, new_round, window_limit_reached};
use super::ladder::{breached, ceiling, reached, round_spend};
use super::schedule::op_in_flight;
use super::signals::{INTERRUPT_GRACE_SECS, count_rate_limit, end_round};
use super::{Effect, OpKind, OpResult, ReplyId, TurnOutcome, emit_op, next_op, outbox};
use crate::headless::FailureKind;
use crate::run::contract::rate_limit_continue;
use crate::run::model::{FailedTurn, Run, StallState};
use crate::run::orch::contract::research_prompt;
use crate::run::orch::launch::research_spec;
use crate::run::phases::set_state;
use crate::run::role_launch::{jitter_ms, session_uuid_of};
use crate::scout::contract::SCOUT_NUDGE;
use crate::scout::service::REPORT_ACCEPTED;

/// Decision 35: the second turn with no report.
pub const NO_REPORT: &str = "the research task ended two turns without a report";

/// A research session whose process stopped mid-turn is resumed with this (decision
/// 35's "deaths follow M8a's worker rules").
pub const RESEARCH_RESUME_AFTER_EXIT: &str = "[anthrex] Your session's process stopped in the middle of a turn and has been resumed. Finish your research and call submit_scout_report, exactly once.";

/// After a daemon restart, a research session that owes its report is resumed with this.
pub const RESEARCH_RESUME: &str =
    "[anthrex] The daemon restarted. Finish your research and call submit_scout_report.";

/// Decision 32's stall nudge, for a research session.
pub fn research_stall_nudge(minutes: u64) -> String {
    format!(
        "[anthrex] {minutes} minutes without any progress. Finish your research and call submit_scout_report."
    )
}

/// Decision 40's soft budget message, for a research session.
pub fn research_wrap_up(spent: Spend, budget: proto::Budget) -> String {
    format!(
        "[anthrex] This research task has used its budget ({}/{} tool calls, {}/{} minutes). Wrap up now: call submit_scout_report with what you have found.",
        spent.tool_calls,
        budget.tool_calls,
        spent.secs / 60,
        budget.minutes
    )
}

/// The outbox address of research task `task`'s session (task ids cannot contain `.`).
pub(super) fn mailbox(task: &str) -> String {
    format!("{task}.research")
}

/// The task whose research session `address` is, if it is a research mailbox.
pub(super) fn mailbox_task(address: &str) -> Option<&str> {
    address.strip_suffix(".research")
}

/// Task `i`'s current research round, if it has one.
pub(super) fn research_round(run: &Run, i: usize) -> Option<usize> {
    run.tasks[i]
        .rounds
        .iter()
        .rposition(|r| r.role == AgentRole::Scout)
}

/// Decision 35: research session `n + 1` of task `i`, in the user's checkout.
pub(super) fn launch(run: &mut Run, i: usize, now: u64, fx: &mut Vec<Effect>) {
    if window_limit_reached(run, i, now) {
        return;
    }
    let op = next_op(run);
    run.tasks[i].session += 1;
    let task = &run.tasks[i];
    let spec = research_spec(run, task);
    let mut first_turn = research_prompt(run, task);
    // M9.9 second review, M-d: a worker's `FreshSession::append`, for research.
    if let Some(append) = &task.orch.research_append {
        first_turn.push_str("\n\n");
        first_turn.push_str(append);
    }
    let name = format!("{}/{}.s{}", run.short(), task.id(), task.session);
    let uuid = (task.route.runtime == Runtime::Claude).then(|| session_uuid_of(run, op));
    let jitter = jitter_ms(&run.id, task.id(), task.session);
    let round = new_round(
        AgentRole::Scout,
        task.session,
        task.route.clone(),
        op,
        uuid.clone(),
        now,
    );
    let (id, session) = (task.id().to_string(), task.session);
    run.tasks[i].orch.research_append = None;
    run.tasks[i].rounds.push(round);
    run.windows_created += 1;
    set_state(&mut run.tasks[i], TaskState::Working, now);
    history(run, i, now, format!("research session {session} starting"));
    let kind = OpKind::CreateWindow {
        name,
        spec: Box::new(spec),
        session_uuid: uuid,
        first_turn,
        project: run.project.clone(),
        worktree: run.root.clone(),
        jitter_ms: jitter,
        extract: None,
    };
    emit_op(run, op, Some(&id), kind, fx);
}

fn reply(fx: &mut Vec<Effect>, reply: ReplyId, result: Result<String, String>) {
    fx.push(Effect::Reply { reply, result });
}

/// `submit_scout_report` from a research task's session (decision 35), in order: the
/// caller is the task's current, live research round; the report validates as an area
/// scout's (M8b's `scout::report::validate`). The report is kept, the task `reported`
/// and the session retired.
pub(super) fn submit_research(
    run: &mut Run,
    id: ReplyId,
    call: &ToolCall,
    now: u64,
    fx: &mut Vec<Effect>,
) {
    let task_id = call.task_id.clone().unwrap_or_default();
    let not_session = format!("this window is not the research session of task {task_id}");
    let found = run.tasks.iter().position(|t| t.id() == task_id);
    let current = found.and_then(|i| {
        let task = &run.tasks[i];
        let r = research_round(run, i)?;
        let round = &task.rounds[r];
        let live = !round.retiring && !round.ended && round.window_id == Some(call.window_id);
        (call.role == AgentRole::Scout && task.state == TaskState::Working && live)
            .then_some((i, r))
    });
    let Some((i, r)) = current else {
        return reply(fx, id, Err(not_session));
    };
    let report = match crate::scout::report::validate(&call.args, ScoutKind::Area) {
        Ok(report) => report,
        Err(text) => return reply(fx, id, Err(text)),
    };
    let task = &mut run.tasks[i];
    task.orch.research = Some(report);
    set_state(task, TaskState::Reported, now);
    task.block = None;
    let round = &mut task.rounds[r];
    round.retiring = true;
    round.resume_op = None;
    if let Some(window_id) = round.window_id {
        fx.push(Effect::RetireWindow { window_id });
    }
    drop_mail(run, i);
    history(run, i, now, "reported: its research report was recorded");
    reply(fx, id, Ok(REPORT_ACCEPTED.to_string()));
}

fn drop_mail(run: &mut Run, i: usize) {
    let address = mailbox(run.tasks[i].id());
    run.outbox.retain(|m| m.task_id != address);
}

/// Stops task `i`'s live research session: the engine's kill, and its mail dropped (a
/// cancel, a block, a fresh session).
pub(super) fn stop_research(run: &mut Run, i: usize, fx: &mut Vec<Effect>) {
    for round in run.tasks[i]
        .rounds
        .iter_mut()
        .filter(|r| r.role == AgentRole::Scout && !r.retiring)
    {
        round.retiring = true;
        round.resume_op = None;
        if round.ended {
            continue;
        }
        if round.route.runtime == Runtime::Codex && !round.turn_open && round.pid.is_none() {
            round.ended = true;
            continue;
        }
        if let Some(window_id) = round.window_id {
            fx.push(Effect::KillWindow { window_id });
        }
    }
    drop_mail(run, i);
}

/// The session is stopped and the task `blocked(environment)` with `text`.
fn give_up(run: &mut Run, i: usize, text: String, now: u64, fx: &mut Vec<Effect>) {
    stop_research(run, i, fx);
    block(run, i, BlockReason::Environment, text, now);
}

/// A worker's rung 2 for a research task: the session is stopped and a fresh research
/// session is launched by the next running pass ([`watch`]), no failure counted here.
fn fresh(run: &mut Run, i: usize, reason: String, now: u64, fx: &mut Vec<Effect>) {
    stop_research(run, i, fx);
    run.tasks[i].rung = 2;
    history(
        run,
        i,
        now,
        format!("rung 2: a fresh research session ({reason})"),
    );
}

/// Decision 38's stall for a research task: `stalls += 1; failures += 1`; a fresh
/// session, or at three failures the task `blocked(environment)` (a research task has
/// no size to raise).
fn stall(run: &mut Run, i: usize, reason: String, now: u64, fx: &mut Vec<Effect>) {
    let task = &mut run.tasks[i];
    task.stalls = task.stalls.saturating_add(1);
    task.failures = task.failures.saturating_add(1);
    let (stalls, failures) = (task.stalls, task.failures);
    history(run, i, now, format!("stalled: {reason}"));
    if failures >= 3 {
        let text = format!("stalled {stalls} times ({failures} failures in all); last: {reason}");
        give_up(run, i, text, now, fx);
    } else {
        fresh(
            run,
            i,
            format!("the last session stalled: {reason}"),
            now,
            fx,
        );
    }
}

/// Whether round `r` of task `i` is its research session still owing its report.
fn owes_report(run: &Run, i: usize, r: usize) -> bool {
    run.tasks[i].state == TaskState::Working
        && research_round(run, i) == Some(r)
        && !run.tasks[i].rounds[r].retiring
}

/// A research session's turn ended (decision 35). A failed turn follows decision 32:
/// a rate limit, or a first other failure, waits `rate_limit_retry_secs` and is then
/// continued ([`watch`]); a second other failure in a row, or an authentication,
/// billing or sandbox failure, blocks. A completed turn with no report gets
/// `SCOUT_NUDGE`, and a second such turn blocks the task; an interrupted one lets the
/// stall nudge go next. `(streak, interrupted)`: whether a rate-limit retry streak ran
/// into the turn's end, and whether the stall ladder interrupted it.
pub(super) fn turn_ended(
    run: &mut Run,
    (i, r): (usize, usize),
    outcome: TurnOutcome,
    (streak, interrupted): (bool, bool),
    now: u64,
    fx: &mut Vec<Effect>,
) {
    if !owes_report(run, i, r) {
        return;
    }
    let wait = run.limits.rate_limit_retry_secs;
    match outcome {
        TurnOutcome::Interrupted => return,
        TurnOutcome::Failed {
            error,
            kind:
                FailureKind::Authentication
                | FailureKind::Billing
                | FailureKind::SandboxUnavailable
                | FailureKind::ClientError,
        } => {
            return give_up(run, i, error, now, fx);
        }
        TurnOutcome::Failed {
            error,
            kind: FailureKind::RateLimit,
        } => {
            if !streak {
                count_rate_limit(run, i, r);
            }
            let round = &mut run.tasks[i].rounds[r];
            round.failed_turn = FailedTurn::WaitingContinue {
                at: not_before(now, wait),
                rate_limit: true,
            };
            round.set_rate_limited(Some(not_before(now, wait)), now);
            super::rounds::note_failed_turn(run, i, r, &error, not_before(now, wait), now);
            run.tasks[i].rounds[r].failed_error = Some(error);
            return;
        }
        TurnOutcome::Failed {
            error,
            kind: FailureKind::Other,
        } => {
            let round = &mut run.tasks[i].rounds[r];
            if matches!(
                round.failed_turn,
                FailedTurn::ContinueSent { rate_limit: false }
            ) {
                return give_up(run, i, error, now, fx);
            }
            round.failed_turn = FailedTurn::WaitingContinue {
                at: not_before(now, wait),
                rate_limit: false,
            };
            super::rounds::note_failed_turn(run, i, r, &error, not_before(now, wait), now);
            run.tasks[i].rounds[r].failed_error = Some(error);
            return;
        }
        TurnOutcome::Completed => {
            let round = &mut run.tasks[i].rounds[r];
            if matches!(round.failed_turn, FailedTurn::ContinueSent { .. }) {
                round.failed_turn = FailedTurn::None;
                round.failed_error = None;
            }
        }
    }
    if interrupted {
        return;
    }
    let round = &mut run.tasks[i].rounds[r];
    if round.review_nudged {
        return give_up(run, i, NO_REPORT.to_string(), now, fx);
    }
    round.review_nudged = true;
    let address = mailbox(run.tasks[i].id());
    outbox::queue_to(run, &address, r, SCOUT_NUDGE.to_string(), now);
}

/// A research session's process exited without the engine killing it (decision 32's
/// worker rules): an exit while an interrupt is pending ends the interrupted turn, and
/// the queued stall nudge goes next; between turns the round is marked ended (a
/// delivery resumes it); mid-turn, the first death in the round resumes the session
/// and the second is a stall. A session with nothing to resume gets a fresh one.
pub(super) fn exited(
    run: &mut Run,
    i: usize,
    r: usize,
    killed: bool,
    now: u64,
    fx: &mut Vec<Effect>,
) {
    let owes = owes_report(run, i, r);
    let round = &mut run.tasks[i].rounds[r];
    let interrupted = round.interrupted || matches!(round.stall, StallState::Interrupted { .. });
    if !killed && owes && round.turn_open && interrupted {
        round.interrupted = false;
        if round.session_id.is_none() {
            end_round(round, now);
            // As a worker's rung 2 (`signals::exited`): the nudge ends its first turn.
            let nudge = research_stall_nudge(run.limits.stall_after_secs / 60);
            run.tasks[i].orch.research_append = Some(nudge);
            let reason = "its turn was interrupted before its session had an id".to_string();
            return fresh(run, i, reason, now, fx);
        }
        round.stall = StallState::Nudged;
        round.turn_open = false;
        if round.route.runtime != Runtime::Codex {
            end_round(round, now);
        }
        return;
    }
    if killed || !owes || !round.turn_open {
        return end_round(round, now);
    }
    round.deaths = round.deaths.saturating_add(1);
    if round.deaths >= 2 {
        end_round(round, now);
        let reason = "its process exited twice in one round".to_string();
        return stall(run, i, reason, now, fx);
    }
    let Some((window_id, session_id)) = round.window_id.zip(round.session_id.clone()) else {
        end_round(round, now);
        let reason = "its process exited before its session started".to_string();
        return fresh(run, i, reason, now, fx);
    };
    round.last_event = now;
    let (id, session) = (run.tasks[i].id().to_string(), run.tasks[i].session);
    let kind = OpKind::ResumeSession {
        window_id,
        session_id,
        message: RESEARCH_RESUME_AFTER_EXIT.to_string(),
        jitter_ms: jitter_ms(&run.id, &id, session),
    };
    let op = next_op(run);
    run.tasks[i].rounds[r].resume_op = Some(op);
    emit_op(run, op, Some(&id), kind, fx);
    history(
        run,
        i,
        now,
        "its research session exited mid-turn; resuming it",
    );
}

/// A research session's `ResumeSession` result: resumed, or, as a worker's failed
/// resume is (decision 28), a fresh session.
pub(super) fn resumed(
    run: &mut Run,
    i: usize,
    r: usize,
    result: OpResult,
    now: u64,
    fx: &mut Vec<Effect>,
) {
    let round = &mut run.tasks[i].rounds[r];
    round.resume_op = None;
    let carried = std::mem::take(&mut round.carried);
    run.outbox.retain(|m| !carried.contains(&m.id));
    let error = match result {
        OpResult::Resumed => return run.tasks[i].rounds[r].delivery_failures = 0,
        OpResult::ResumeFailed { error } => error,
        OpResult::Failed { message } => message,
        _ => return,
    };
    end_round(&mut run.tasks[i].rounds[r], now);
    if owes_report(run, i, r) {
        let reason = format!("its session could not be resumed: {error}");
        fresh(run, i, reason, now, fx);
    }
}

/// Decision 40 for task `i`'s research session `r`, in order: the task's total over
/// its research sessions at the next size's budget blocks it `human` (rung 4); a hard
/// breach of the session's budget gets a fresh session, the second blocks the task;
/// the soft wrap-up goes once per session. Returns whether the session was stopped.
fn check_budget(run: &mut Run, i: usize, r: usize, now: u64, fx: &mut Vec<Effect>) -> bool {
    let task = &run.tasks[i];
    let stopped = task.clock.stopped;
    let mut total = Spend::default();
    for round in task.rounds.iter().filter(|x| x.role == AgentRole::Scout) {
        let spend = round_spend(round, stopped, now);
        total.tool_calls = total.tool_calls.saturating_add(spend.tool_calls);
        total.secs = total.secs.saturating_add(spend.secs);
        total.tokens = total.tokens.saturating_add(spend.tokens);
    }
    let next = ceiling(run, task);
    if reached(total, next) {
        let text = format!(
            "the task's total spend reached the next size's budget ({}/{} tool calls, {}/{} minutes)",
            total.tool_calls,
            next.tool_calls,
            total.secs / 60,
            next.minutes
        );
        stop_research(run, i, fx);
        block(run, i, BlockReason::Human, text, now);
        return true;
    }
    let spend = round_spend(&task.rounds[r], stopped, now);
    let budget = task.budget;
    if let Some(what) = breached(spend, budget) {
        let task = &mut run.tasks[i];
        task.budget_exceeded = task.budget_exceeded.saturating_add(1);
        if task.budget_exceeded >= 2 {
            let text = format!("exceeded its budget twice; last: {what}");
            give_up(run, i, text, now, fx);
        } else {
            let reason = format!("the last session exceeded its budget: {what}");
            fresh(run, i, reason, now, fx);
        }
        return true;
    }
    if reached(spend, budget) && !task.rounds[r].wrap_up_sent {
        run.tasks[i].rounds[r].wrap_up_sent = true;
        let address = mailbox(run.tasks[i].id());
        outbox::queue_to(run, &address, r, research_wrap_up(spend, budget), now);
    }
    false
}

/// Every scheduler pass of a running run, for each working research task: a failed
/// turn's continue once its wait is
/// over; the budget; a session a restart ended while it owed its report resumed with
/// [`RESEARCH_RESUME`] (or a fresh one when it has no id); then decision 32's stall
/// ladder on an open turn: silence interrupts it and queues the stall nudge, and an
/// interrupt that did not end the turn, or a second silence, is a stall. Last, a
/// fresh session for a task whose session was stopped.
pub(super) fn watch(run: &mut Run, now: u64, fx: &mut Vec<Effect>) {
    if run.state != RunState::Running {
        return;
    }
    let stall_after = run.limits.stall_after_secs;
    for i in 0..run.tasks.len() {
        let task = &run.tasks[i];
        if task.spec.kind != proto::TaskKind::Research || task.state != TaskState::Working {
            continue;
        }
        let launching = launching(run, i);
        let Some(r) = research_round(run, i).filter(|&r| !task.rounds[r].retiring) else {
            continue;
        };
        let round = &run.tasks[i].rounds[r];
        if let FailedTurn::WaitingContinue { at, rate_limit } = round.failed_turn
            && now >= at
        {
            let round = &mut run.tasks[i].rounds[r];
            round.failed_turn = FailedTurn::ContinueSent { rate_limit };
            round.set_rate_limited(None, now);
            let reason = round.failed_error.clone().unwrap_or_default();
            let address = mailbox(run.tasks[i].id());
            outbox::queue_to(run, &address, r, rate_limit_continue(&reason), now);
        }
        if check_budget(run, i, r, now, fx) {
            continue;
        }
        let round = &run.tasks[i].rounds[r];
        let address = mailbox(run.tasks[i].id());
        let waiting = run
            .outbox
            .iter()
            .any(|m| m.task_id == address && m.delivered_at.is_none())
            || matches!(round.failed_turn, FailedTurn::WaitingContinue { .. });
        let busy = round.resume_op.is_some() || launching;
        if round.ended && !waiting && !busy && round.relaunch.is_none() {
            if round.session_id.is_none() {
                let reason = "its session ended before it had an id to resume".to_string();
                fresh(run, i, reason, now, fx);
            } else {
                outbox::queue_to(run, &address, r, RESEARCH_RESUME.to_string(), now);
            }
            continue;
        }
        if round.ended || !round.turn_open {
            continue;
        }
        let silent = now >= stall_due(round, stall_after);
        match round.stall {
            StallState::Interrupted { deadline } if now >= deadline => {
                let reason = "the interrupt did not end its turn".to_string();
                stall(run, i, reason, now, fx);
            }
            StallState::Watching if silent => {
                let window_id = round.window_id.unwrap_or_default();
                fx.push(Effect::Interrupt { window_id });
                let round = &mut run.tasks[i].rounds[r];
                round.stall = StallState::Interrupted {
                    deadline: not_before(now, INTERRUPT_GRACE_SECS),
                };
                round.interrupted = true;
                let nudge = research_stall_nudge(stall_after / 60);
                outbox::queue_to(run, &address, r, nudge, now);
                history(run, i, now, "no progress; interrupting its research turn");
            }
            StallState::Nudged if silent => {
                let reason = format!("no stream event for {} minutes", stall_after / 60);
                stall(run, i, reason, now, fx);
            }
            _ => {}
        }
    }
    // A fresh session for each working research task whose session was stopped.
    for i in 0..run.tasks.len() {
        let task = &run.tasks[i];
        let stopped = research_round(run, i).is_some_and(|r| task.rounds[r].retiring);
        if task.spec.kind == proto::TaskKind::Research
            && task.state == TaskState::Working
            && stopped
            && !launching(run, i)
        {
            launch(run, i, now, fx);
        }
    }
}

fn launching(run: &Run, i: usize) -> bool {
    op_in_flight(run, run.tasks[i].id(), |k| {
        matches!(
            k,
            OpKind::CreateWindow { .. } | OpKind::ResumeSession { .. }
        )
    })
}
