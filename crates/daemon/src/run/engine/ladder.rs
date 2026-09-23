//! Decision 38's escalation ladder — gate failures, stalls and hard budget breaches, and
//! rungs 1 to 4 — and decision 40's budgets. Rung 2's fresh session is started here
//! once the old session is gone (`DiffSoFar`, then decision 30's hand-over prompt).
//! Pure (design decision 2).

use proto::{AgentRole, BlockReason, Budget, GateKind, Size, Spend, TaskState};

use super::dispatch::{block, history, launch_fresh};
use super::schedule::op_in_flight;
use super::{Effect, OpKind, OpResult, done, emit_op, next_op, outbox};
use crate::run::contract::budget_wrap_up;
use crate::run::model::{AgentRound, FreshSession, Run, Task};
use crate::run::roster::escalate;

/// The note rung 3 adds to a task it raises (M8a.6's `rung3` fixture uses the same).
const RAISED_NOTE: &str = "size raised by rung 3 (decision 38)";

/// The task's current worker round, if it has one.
pub(super) fn worker_round(task: &Task) -> Option<usize> {
    task.rounds
        .iter()
        .rposition(|r| r.role == AgentRole::Worker)
}

/// A worker round whose session the engine still counts on: started, not ended and
/// not being killed.
pub(super) fn live(round: &AgentRound) -> bool {
    round.window_id.is_some() && !round.ended && !round.retiring
}

/// Kills every live worker session of task `i` (decision 38, rungs 2 to 4), ending its
/// claim (ruling T12-I1) and every op its sessions awaited (ruling T12-N).
pub(super) fn kill_worker(run: &mut Run, i: usize, fx: &mut Vec<Effect>) {
    done::drop_claim(
        run,
        i,
        "this session is being stopped; its task_done no longer applies",
        fx,
    );
    supersede(run, i);
    for round in run.tasks[i]
        .rounds
        .iter_mut()
        .filter(|r| r.role == AgentRole::Worker && !r.ended && !r.retiring)
    {
        if let Some(window_id) = round.window_id {
            round.retiring = true;
            fx.push(Effect::KillWindow { window_id });
        }
    }
}

/// Ruling T12-N: task `i`'s worker sessions so far are superseded. The results of the
/// ops they awaited (a resume, a commit count) are dropped when they come, and the
/// messages in flight to them (a `Deliver`'s or a resume's) leave the outbox, so their
/// `Delivered` finds nothing either.
pub(super) fn supersede(run: &mut Run, i: usize) {
    for round in run.tasks[i]
        .rounds
        .iter_mut()
        .filter(|r| r.role == AgentRole::Worker)
    {
        round.resume_op = None;
        round.count_op = None;
    }
    let id = run.tasks[i].id().to_string();
    run.outbox
        .retain(|m| m.task_id != id || m.delivered_at.is_none());
}

/// Undelivered messages to task `i`'s worker are dropped when its session is replaced
/// or stopped: the hand-over prompt carries the failure record instead.
fn drop_queued(run: &mut Run, i: usize) {
    let id = run.tasks[i].id().to_string();
    run.outbox
        .retain(|m| m.task_id != id || m.delivered_at.is_some());
}

fn first_line(text: &str) -> &str {
    text.lines()
        .find(|l| !l.trim().is_empty())
        .unwrap_or(text)
        .trim()
}

fn gate_label(gate: GateKind) -> &'static str {
    match gate {
        GateKind::Done => "done",
        GateKind::Proof => "proof",
        GateKind::Check => "check",
        GateKind::Review => "review",
        GateKind::Merge => "merge",
    }
}

/// A gate failure (decision 38): `bounces[gate] += 1; failures += 1`; rung 3 past
/// `max_bounces` or at three failures, rung 2 at two, else rung 1 — `text` to the same
/// session as its next turn, unless `told` (the tool reply already carried it, decision
/// 55). The text joins the failure record either way. Returns the rung taken.
pub(super) fn gate_failure(
    run: &mut Run,
    i: usize,
    gate: GateKind,
    text: String,
    told: bool,
    now: u64,
    fx: &mut Vec<Effect>,
) -> u8 {
    let task = &mut run.tasks[i];
    let bounces = match gate {
        GateKind::Done => &mut task.bounces.done,
        GateKind::Proof => &mut task.bounces.proof,
        GateKind::Check => &mut task.bounces.check,
        GateKind::Review => &mut task.bounces.review,
        GateKind::Merge => &mut task.bounces.merge,
    };
    *bounces = bounces.saturating_add(1);
    let bounced = *bounces;
    task.failures = task.failures.saturating_add(1);
    task.failure_log.push(text.clone());
    let failures = task.failures;
    let label = gate_label(gate);
    if bounced > run.limits.max_bounces || failures >= 3 {
        let cause = format!(
            "the {label} gate failed {bounced} times ({failures} failures in all); last: {}",
            first_line(&text)
        );
        rung3(run, i, cause, now, fx);
        3
    } else if failures == 2 {
        let reason = format!("the {label} gate failed again: {}", first_line(&text));
        rung2(run, i, reason, now, fx);
        2
    } else {
        let task = &mut run.tasks[i];
        task.rung = 1;
        task.state = TaskState::Working;
        history(run, i, now, format!("the {label} gate bounced it (rung 1)"));
        if !told {
            let id = run.tasks[i].id().to_string();
            outbox::queue(run, &id, text, now);
        }
        1
    }
}

/// A stall (decision 38): `stalls += 1; failures += 1`; rung 3 at three failures, else
/// rung 2.
pub(super) fn stall(run: &mut Run, i: usize, reason: String, now: u64, fx: &mut Vec<Effect>) {
    let task = &mut run.tasks[i];
    task.stalls = task.stalls.saturating_add(1);
    task.failures = task.failures.saturating_add(1);
    let (stalls, failures) = (task.stalls, task.failures);
    history(run, i, now, format!("stalled: {reason}"));
    if failures >= 3 {
        let cause = format!("stalled {stalls} times ({failures} failures in all); last: {reason}");
        rung3(run, i, cause, now, fx);
    } else {
        rung2(
            run,
            i,
            format!("the last session stalled: {reason}"),
            now,
            fx,
        );
    }
}

/// A hard budget breach (decision 38): rung 3 at the second, else rung 2.
fn breach(run: &mut Run, i: usize, what: String, now: u64, fx: &mut Vec<Effect>) {
    let task = &mut run.tasks[i];
    task.budget_exceeded = task.budget_exceeded.saturating_add(1);
    if task.budget_exceeded >= 2 {
        rung3(
            run,
            i,
            format!("exceeded its budget twice; last: {what}"),
            now,
            fx,
        );
    } else {
        rung2(
            run,
            i,
            format!("the last session exceeded its budget: {what}"),
            now,
            fx,
        );
    }
}

/// Rung 2: the session killed; a fresh one on `roster::escalate(route)` starts in the
/// same worktree once the old one has exited ([`start_fresh_sessions`]).
pub(super) fn rung2(run: &mut Run, i: usize, reason: String, now: u64, fx: &mut Vec<Effect>) {
    done::drop_claim(
        run,
        i,
        "this session is being replaced by a fresh one; its task_done no longer applies",
        fx,
    );
    kill_worker(run, i, fx);
    drop_queued(run, i);
    let route = escalate(&run.roster, &run.tasks[i].route);
    let task = &mut run.tasks[i];
    task.rung = 2;
    task.state = TaskState::Working;
    task.route = route;
    task.fresh_session = Some(FreshSession {
        reason: reason.clone(),
        append: None,
    });
    history(run, i, now, format!("rung 2: a fresh session ({reason})"));
}

/// Rung 3: `blocked(mis_sized)`, the size raised one step, the worker killed and the
/// worktree kept.
pub(super) fn rung3(run: &mut Run, i: usize, text: String, now: u64, fx: &mut Vec<Effect>) {
    kill_worker(run, i, fx);
    drop_queued(run, i);
    let task = &mut run.tasks[i];
    task.size = task.size.raised();
    task.raised_size = Some(task.size);
    task.rung = 3;
    task.fresh_session = None;
    if !task.notes.iter().any(|n| n == RAISED_NOTE) {
        task.notes.push(RAISED_NOTE.to_string());
    }
    block(run, i, BlockReason::MisSized, text, now);
}

/// Rung 4: `blocked(human)`, the worker killed.
fn rung4(run: &mut Run, i: usize, text: String, now: u64, fx: &mut Vec<Effect>) {
    kill_worker(run, i, fx);
    drop_queued(run, i);
    let task = &mut run.tasks[i];
    task.rung = 4;
    task.fresh_session = None;
    block(run, i, BlockReason::Human, text, now);
}

/// A worker round's session spend (decision 40): its tool calls, its wall-clock
/// seconds since it started, and its billable tokens.
pub(crate) fn round_spend(round: &AgentRound, now: u64) -> Spend {
    Spend {
        tool_calls: round.tool_calls,
        secs: round
            .ended_at
            .unwrap_or(now)
            .saturating_sub(round.started_at),
        tokens: round.usage.billable(),
    }
}

/// The task's cumulative spend: tool calls and tokens as counted on the task, and the
/// seconds of every worker session it has had.
pub(crate) fn total_spend(task: &Task, now: u64) -> Spend {
    let secs = task
        .rounds
        .iter()
        .filter(|r| r.role == AgentRole::Worker)
        .map(|r| round_spend(r, now).secs)
        .sum();
    Spend {
        secs,
        ..task.spent_total
    }
}

/// Some axis reached: `tool_calls`, minutes or (when set) tokens at or past `budget`.
fn reached(spend: Spend, budget: Budget) -> bool {
    spend.tool_calls >= budget.tool_calls
        || spend.secs >= u64::from(budget.minutes) * 60
        || budget.tokens.is_some_and(|t| spend.tokens >= t)
}

/// Decision 40's hard limit: spend at 1.5 × the budget on any axis is a breach,
/// compared in integers (`2 × spend >= 3 × budget`, so 7 of 5 tool calls is not and 8
/// is; 450 seconds of 5 minutes is); what was breached.
fn breached(spend: Spend, budget: Budget) -> Option<String> {
    if u64::from(spend.tool_calls).saturating_mul(2)
        >= u64::from(budget.tool_calls).saturating_mul(3)
    {
        return Some(format!(
            "{} tool calls against a budget of {}",
            spend.tool_calls, budget.tool_calls
        ));
    }
    if spend.secs.saturating_mul(2) >= u64::from(budget.minutes).saturating_mul(180) {
        return Some(format!(
            "{} minutes against a budget of {}",
            spend.secs / 60,
            budget.minutes
        ));
    }
    match budget.tokens {
        Some(tokens) if spend.tokens.saturating_mul(2) >= tokens.saturating_mul(3) => Some(
            format!("{} tokens against a budget of {tokens}", spend.tokens),
        ),
        _ => None,
    }
}

/// Rung 4's ceiling: the next size's budget (S: M's; M or hub: L's).
fn ceiling(run: &Run, task: &Task) -> Budget {
    if task.size == Size::S && !task.hub {
        run.limits.budget_m
    } else {
        run.limits.budget_l
    }
}

/// Decisions 38 and 40 for task `i`'s live worker session, in order: rung 4 on the
/// task's total, a hard breach of the session's budget, then the soft wrap-up, once per
/// session. Returns whether the session was stopped.
pub(super) fn check_budget(run: &mut Run, i: usize, now: u64, fx: &mut Vec<Effect>) -> bool {
    let task = &run.tasks[i];
    if task.state != TaskState::Working {
        return false;
    }
    let Some(r) = worker_round(task).filter(|&r| live(&task.rounds[r])) else {
        return false;
    };
    let total = total_spend(task, now);
    let next = ceiling(run, task);
    if reached(total, next) {
        let text = format!(
            "the task's total spend reached the next size's budget ({}/{} tool calls, {}/{} minutes)",
            total.tool_calls,
            next.tool_calls,
            total.secs / 60,
            next.minutes
        );
        rung4(run, i, text, now, fx);
        return true;
    }
    let spend = round_spend(&task.rounds[r], now);
    let budget = task.budget;
    if let Some(what) = breached(spend, budget) {
        breach(run, i, what, now, fx);
        return true;
    }
    if reached(spend, budget) && !task.rounds[r].wrap_up_sent {
        run.tasks[i].rounds[r].wrap_up_sent = true;
        let id = run.tasks[i].id().to_string();
        outbox::queue(run, &id, budget_wrap_up(spend, budget), now);
    }
    false
}

/// Rung 2, or a resume that failed (decision 28): a working task with a fresh session
/// decided on, no live worker session left and none being prepared gets `DiffSoFar`
/// for its hand-over prompt. A held task never does (M8a.6 ruling N5): it is not
/// `working` until its hand-back.
pub(super) fn start_fresh_sessions(run: &mut Run, fx: &mut Vec<Effect>) {
    for i in 0..run.tasks.len() {
        let task = &run.tasks[i];
        if !fresh_due(task)
            || op_in_flight(run, task.id(), |k| {
                matches!(k, OpKind::DiffSoFar { .. } | OpKind::CreateWindow { .. })
            })
        {
            continue;
        }
        let (id, worktree) = (task.id().to_string(), task.worktree.clone());
        let start = task
            .start_commit
            .clone()
            .unwrap_or_else(|| run.run_head.clone());
        let kind = OpKind::DiffSoFar {
            worktree,
            start,
            run_head: run.run_head.clone(),
        };
        let op = next_op(run);
        emit_op(run, op, Some(&id), kind, fx);
    }
}

fn fresh_due(task: &Task) -> bool {
    task.state == TaskState::Working
        && !task.awaiting_deps
        && task.fresh_session.is_some()
        && task
            .rounds
            .iter()
            .filter(|r| r.role == AgentRole::Worker)
            .all(|r| r.ended)
}

/// `DiffSoFar`'s result: the fresh session, with decision 30's hand-over prompt, when
/// it is still due. A diff that could not be computed is named in the prompt.
pub(super) fn fresh_diff(
    run: &mut Run,
    i: usize,
    result: OpResult,
    now: u64,
    fx: &mut Vec<Effect>,
) {
    if !fresh_due(&run.tasks[i]) {
        return;
    }
    let (stat, patch) = match result {
        OpResult::Diff { stat, patch } => (stat, patch),
        OpResult::Failed { message } => (
            format!("(the diff could not be read: {message})"),
            String::new(),
        ),
        _ => return,
    };
    let Some(fresh) = run.tasks[i].fresh_session.clone() else {
        return;
    };
    let rounds = run.tasks[i].rounds.len();
    launch_fresh(run, i, &fresh, &stat, &patch, now, fx);
    // Kept when the launch was refused (the window limit blocks the task), so the
    // messages it carries are not lost.
    if run.tasks[i].rounds.len() > rounds {
        run.tasks[i].fresh_session = None;
    }
}
