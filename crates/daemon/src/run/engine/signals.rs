//! Decision 27's engine side and decision 32's session rules: each window signal
//! applied to its round (turns, counters, rate-limit streaks, sub-agents, usage,
//! denials, failed turns, process exits), and the watchdog each scheduler pass runs
//! (the stall clock, the failed turn's continue, the minutes budget). Pure (design
//! decision 2).

use proto::{AgentRole, BlockReason, Runtime, TaskState, TokenUsage};

use super::dispatch::{block, history};
use super::ladder::{self, check_budget, kill_worker, live, worker_round};
use super::{
    AgentSignal, Effect, EngineState, OpKind, TurnOutcome, done, emit_op, next_op, outbox,
};
use crate::headless::FailureKind;
use crate::run::contract::{
    RESUME_AFTER_EXIT, denied_text, rate_limit_continue, sandbox_unavailable_text, stall_nudge,
};
use crate::run::model::{AgentRound, FailedTurn, FreshSession, Run, StallState};
use crate::run::role_launch::jitter_ms;

/// Decision 32: an interrupt that has not ended the turn this many seconds later gets
/// the session killed (the driver's `INTERRUPT_GRACE`, in the reducer's unix seconds).
pub const INTERRUPT_GRACE_SECS: u64 = 30;

/// A rate-limit retry's error (decision 32; M8a.7's `ApiRetry.error`).
const RATE_LIMIT: &str = "rate_limit";

/// Applies `signal` to the round of `window_id` (the latest round with that window).
pub(super) fn on_signal(
    state: &mut EngineState,
    window_id: u32,
    signal: AgentSignal,
    now: u64,
    fx: &mut Vec<Effect>,
) {
    for run in state.runs.values_mut() {
        let found = run.tasks.iter().enumerate().find_map(|(i, t)| {
            t.rounds
                .iter()
                .rposition(|r| r.window_id == Some(window_id))
                .map(|r| (i, r))
        });
        if let Some((i, r)) = found {
            apply(run, i, r, signal, now, fx);
            return;
        }
    }
}

fn runtime_label(runtime: Runtime) -> String {
    serde_json::to_value(runtime)
        .ok()
        .and_then(|v| v.as_str().map(str::to_string))
        .unwrap_or_default()
}

fn apply(run: &mut Run, i: usize, r: usize, signal: AgentSignal, now: u64, fx: &mut Vec<Effect>) {
    let round = &mut run.tasks[i].rounds[r];
    if round.ended {
        return;
    }
    match signal {
        AgentSignal::ProcessStarted { pid } => {
            round.pid = Some(pid);
            return;
        }
        AgentSignal::ProcessExited {
            killed_by_engine, ..
        } => return exited(run, i, r, killed_by_engine, now, fx),
        _ if round.retiring => return,
        _ => {}
    }
    // Every stream event: the clock, and the end of a rate-limit streak (decision 32).
    round.last_event = now;
    let streak = round.in_retry_streak;
    if !matches!(signal, AgentSignal::ApiRetry { .. }) {
        round.in_retry_streak = false;
        if !matches!(round.failed_turn, FailedTurn::WaitingContinue { .. }) {
            round.rate_limited_until = None;
        }
    }
    let worker = round.role == AgentRole::Worker;
    match signal {
        AgentSignal::Init { session_id } => round.session_id = Some(session_id),
        AgentSignal::TurnStarted => round.turn_open = true,
        AgentSignal::ToolUse { .. } => {
            round.tool_calls += 1;
            let task = &mut run.tasks[i];
            task.spent_total.tool_calls = task.spent_total.tool_calls.saturating_add(1);
            if worker {
                check_budget(run, i, now, fx);
            }
        }
        AgentSignal::ApiRetry { error, delay_ms } => {
            round.rate_limited_until = Some(now + delay_ms.div_ceil(1000));
            if error == RATE_LIMIT {
                round.in_retry_streak = true;
                if !streak {
                    count_rate_limit(run, i, r);
                }
            }
        }
        AgentSignal::PermissionDenied { tool, reason } => {
            round.denials += 1;
            round.turn_denials += 1;
            round.last_denial = Some(format!("{tool}: {reason}"));
            if worker {
                check_denials(run, i, r, now, fx);
            }
        }
        AgentSignal::SubagentStart { agent_id } => {
            round.open_subagents.insert(agent_id);
        }
        AgentSignal::SubagentStop { agent_id } => {
            round.open_subagents.remove(&agent_id);
            if round.open_subagents.is_empty() && round.fallback_waiting && !round.turn_open {
                round.fallback_waiting = false;
                done::fallback(run, i, fx);
            }
        }
        AgentSignal::TurnEnded {
            outcome,
            usage,
            denials,
        } => turn_ended(run, i, r, outcome, usage, denials, streak, now, fx),
        AgentSignal::Activity
        | AgentSignal::ProcessStarted { .. }
        | AgentSignal::ProcessExited { .. } => {}
    }
}

fn count_rate_limit(run: &mut Run, i: usize, r: usize) {
    let label = runtime_label(run.tasks[i].rounds[r].route.runtime);
    *run.rate_limits.entry(label).or_insert(0) += 1;
}

fn add_usage(total: &mut TokenUsage, usage: TokenUsage) {
    total.input += usage.input;
    total.output += usage.output;
    total.cache_read += usage.cache_read;
    total.cache_write += usage.cache_write;
}

/// Decision 32's denial rule: `denials_before_block` in one session block the task as
/// `blocked(environment)` and kill the session. Returns whether it did.
fn check_denials(run: &mut Run, i: usize, r: usize, now: u64, fx: &mut Vec<Effect>) -> bool {
    let round = &run.tasks[i].rounds[r];
    if run.tasks[i].state != TaskState::Working || round.denials < run.limits.denials_before_block {
        return false;
    }
    let last = round
        .last_denial
        .clone()
        .unwrap_or_else(|| "a tool: reported in the turn's permission_denials".to_string());
    let (tool, reason) = last.split_once(": ").unwrap_or((&last, ""));
    let text = denied_text(round.denials, tool, reason);
    kill_worker(run, i, fx);
    block(run, i, BlockReason::Environment, text, now);
    true
}

#[allow(clippy::too_many_arguments)]
fn turn_ended(
    run: &mut Run,
    i: usize,
    r: usize,
    outcome: TurnOutcome,
    usage: Option<TokenUsage>,
    denials: u32,
    streak: bool,
    now: u64,
    fx: &mut Vec<Effect>,
) {
    let round = &mut run.tasks[i].rounds[r];
    round.turn_open = false;
    if let Some(usage) = usage {
        add_usage(&mut round.usage, usage);
        let task = &mut run.tasks[i];
        task.spent_total.tokens = task.spent_total.tokens.saturating_add(usage.billable());
    }
    let round = &mut run.tasks[i].rounds[r];
    // Decision 27: only the denials the turn's events did not already report.
    let new = denials.saturating_sub(std::mem::take(&mut round.turn_denials));
    round.denials += new;
    let had_done = std::mem::take(&mut round.turn_had_task_done);
    if round.role != AgentRole::Worker || run.tasks[i].state != TaskState::Working {
        return;
    }
    if check_denials(run, i, r, now, fx) || check_budget(run, i, now, fx) {
        return;
    }
    let claimed = run.tasks[i].claim.is_some();
    let round = &mut run.tasks[i].rounds[r];
    let interrupted = matches!(round.stall, StallState::Interrupted { .. });
    if interrupted {
        // The interrupted turn ended: its queued `stall_nudge` goes next.
        round.stall = StallState::Nudged;
    }
    match outcome {
        TurnOutcome::Completed => {
            if matches!(round.failed_turn, FailedTurn::ContinueSent { .. }) {
                round.failed_turn = FailedTurn::None;
                round.failed_error = None;
            }
            if interrupted || had_done || claimed {
                return;
            }
            // Decision 32: open sub-agents defer the fallback to the last
            // `SubagentStop`, or to the next turn end.
            if !round.open_subagents.is_empty() && !round.fallback_waiting {
                round.fallback_waiting = true;
                return;
            }
            round.fallback_waiting = false;
            done::fallback(run, i, fx);
        }
        TurnOutcome::Interrupted => {}
        TurnOutcome::Failed { error, kind } => failed_turn(run, i, r, error, kind, streak, now, fx),
    }
}

/// Decision 32's failed turns (and decision 54's unavailable sandbox).
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
    match kind {
        FailureKind::SandboxUnavailable => {
            kill_worker(run, i, fx);
            block(
                run,
                i,
                BlockReason::Environment,
                sandbox_unavailable_text(&error),
                now,
            );
        }
        FailureKind::Authentication | FailureKind::Billing => {
            block(run, i, BlockReason::Environment, error, now);
        }
        FailureKind::RateLimit => {
            // One event, unless a retry streak ran straight into this failure.
            if !streak {
                count_rate_limit(run, i, r);
            }
            let round = &mut run.tasks[i].rounds[r];
            round.failed_turn = FailedTurn::WaitingContinue {
                at: now + wait,
                rate_limit: true,
            };
            round.rate_limited_until = Some(now + wait);
            round.failed_error = Some(error);
        }
        FailureKind::Other => {
            let round = &mut run.tasks[i].rounds[r];
            if matches!(
                round.failed_turn,
                FailedTurn::ContinueSent { rate_limit: false }
            ) {
                return block(run, i, BlockReason::Environment, error, now);
            }
            round.failed_turn = FailedTurn::WaitingContinue {
                at: now + wait,
                rate_limit: false,
            };
            round.failed_error = Some(error);
        }
    }
}

/// A session process that exited (decision 32). The engine's own kill ends the round.
/// A Codex exit between turns is the normal end of its turn's process. Otherwise, for
/// a worker of a working task: mid-turn, the first exit in the round resumes the
/// session with `RESUME_AFTER_EXIT` and the second is a stall; between turns a Claude
/// round is marked ended, and the next delivery resumes it. Any other task's worker
/// session is marked ended too, so a held task is never resumed here (M8a.6 ruling
/// N5): only a delivery resumes it, once it takes messages again.
fn exited(run: &mut Run, i: usize, r: usize, killed: bool, now: u64, fx: &mut Vec<Effect>) {
    let working = run.tasks[i].state == TaskState::Working;
    let round = &mut run.tasks[i].rounds[r];
    let worker = round.role == AgentRole::Worker;
    // Codex runs one process per turn: its exit between turns is the normal end of one.
    if !killed && !round.turn_open && round.route.runtime == Runtime::Codex {
        return;
    }
    round.pid = None;
    // A reviewer's unexpected exit is M8a.13's (decision 35's resume rule).
    if !killed && !worker {
        return;
    }
    if killed || !working || !round.turn_open {
        end_round(round, now);
        return;
    }
    round.deaths = round.deaths.saturating_add(1);
    if round.deaths >= 2 {
        end_round(round, now);
        let reason = "its process exited twice in one round".to_string();
        return ladder::stall(run, i, reason, now, fx);
    }
    let Some((window_id, session_id)) = round.window_id.zip(round.session_id.clone()) else {
        // Nothing to resume: a fresh session at the same rung, no failure counted.
        end_round(round, now);
        run.tasks[i].fresh_session = Some(FreshSession {
            reason: "its process exited before its session started".into(),
            append: None,
        });
        return;
    };
    round.last_event = now;
    let (id, session) = (run.tasks[i].id().to_string(), run.tasks[i].session);
    let kind = OpKind::ResumeSession {
        window_id,
        session_id,
        message: RESUME_AFTER_EXIT.to_string(),
        jitter_ms: jitter_ms(&run.id, &id, session),
    };
    let op = next_op(run);
    emit_op(run, op, Some(&id), kind, fx);
    history(
        run,
        i,
        now,
        "its process exited mid-turn; resuming the session",
    );
}

pub(super) fn end_round(round: &mut AgentRound, now: u64) {
    round.ended = true;
    round.ended_at = Some(now);
    round.turn_open = false;
    round.pid = None;
}

/// The watchdog, run on every scheduler pass of a running run: for each working task's
/// live worker session, a failed turn's continue once its wait is over, the minutes
/// budget, then the stall clock (decision 32): no stream event for `stall_after_secs`
/// in an open turn (the clock suspended until `rate_limited_until`) interrupts the
/// turn and queues `stall_nudge`; an interrupt that has not ended the turn within
/// [`INTERRUPT_GRACE_SECS`], or a second silence in the same session, is a stall.
pub(super) fn watch(run: &mut Run, now: u64, fx: &mut Vec<Effect>) {
    for i in 0..run.tasks.len() {
        let task = &run.tasks[i];
        if task.state != TaskState::Working {
            continue;
        }
        let Some(r) = worker_round(task).filter(|&r| live(&task.rounds[r])) else {
            continue;
        };
        if let FailedTurn::WaitingContinue { at, rate_limit } = task.rounds[r].failed_turn
            && now >= at
        {
            let round = &mut run.tasks[i].rounds[r];
            round.failed_turn = FailedTurn::ContinueSent { rate_limit };
            round.rate_limited_until = None;
            let reason = round.failed_error.clone().unwrap_or_default();
            let id = run.tasks[i].id().to_string();
            outbox::queue(run, &id, rate_limit_continue(&reason), now);
        }
        if check_budget(run, i, now, fx) {
            continue;
        }
        let round = &run.tasks[i].rounds[r];
        if !round.turn_open {
            continue;
        }
        let stall_after = run.limits.stall_after_secs;
        let quiet = round.last_event.max(round.rate_limited_until.unwrap_or(0));
        let silent = now >= quiet + stall_after;
        match round.stall {
            StallState::Interrupted { deadline } if now >= deadline => {
                let reason = "the interrupt did not end its turn".to_string();
                ladder::stall(run, i, reason, now, fx);
            }
            StallState::Watching if silent => {
                let window_id = round.window_id.unwrap_or_default();
                fx.push(Effect::Interrupt { window_id });
                run.tasks[i].rounds[r].stall = StallState::Interrupted {
                    deadline: now + INTERRUPT_GRACE_SECS,
                };
                let id = run.tasks[i].id().to_string();
                outbox::queue(run, &id, stall_nudge(stall_after / 60), now);
                history(run, i, now, "no progress; interrupting its turn");
            }
            StallState::Nudged if silent => {
                let reason = format!("no stream event for {} minutes", stall_after / 60);
                ladder::stall(run, i, reason, now, fx);
            }
            _ => {}
        }
    }
}
