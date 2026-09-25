//! Decision 27's engine side and decision 32's session rules: each window signal
//! applied to its round (turns, counters, rate-limit streaks, sub-agents, usage,
//! denials, failed turns, process exits), and the watchdog each scheduler pass runs
//! (the stall clock, the failed turn's continue, the minutes budget). Pure (design
//! decision 2).

use proto::{AgentRole, BlockReason, Runtime, TaskState, TokenUsage};

use super::clock::{not_before, stall_due};
use super::dispatch::{block, history};
use super::ladder::{self, check_budget, kill_worker, live, worker_round};
use super::{
    AgentSignal, Effect, EngineState, OpKind, TurnOutcome, emit_op, fallback, next_op, outbox,
    review,
};
use crate::headless::FailureKind;
use crate::run::contract::{
    DENIAL_LISTED_REASON, RESUME_AFTER_EXIT, denied_text, rate_limit_continue,
    sandbox_unavailable_text, stall_nudge,
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
            round.exited_pid = None;
            return;
        }
        // Final review B-10 (T25-N1, both orders): the second copy of an exit the round
        // took already, reaching a round resumed since with no process yet. Pid 0 is the
        // engine's synthetic exit for a round with no process, one per kill, so it is
        // never a repeat (F3 review N2).
        AgentSignal::ProcessExited { pid, .. }
            if pid != 0 && round.pid.is_none() && round.exited_pid == Some(pid) =>
        {
            return;
        }
        // Ruling T13-R2: the exit of another process than the round's (a late exit of
        // a previous Codex process, arriving after the next one started) is dropped.
        AgentSignal::ProcessExited { pid, .. } if round.pid.is_some_and(|p| p != pid) => {
            return;
        }
        // Ruling T13-P1: a Codex send waits for the previous process's exit, which can
        // reach the engine after the next turn was delivered (and opened) but before
        // the next process starts. The exit of the process a turn closed in is that
        // turn's normal end, never a death.
        AgentSignal::ProcessExited {
            pid,
            killed_by_engine: false,
            ..
        } if round.route.runtime == Runtime::Codex
            && !round.retiring
            && round.closed_pid == Some(pid) =>
        {
            round.pid = None;
            return;
        }
        AgentSignal::ProcessExited {
            killed_by_engine,
            pid,
            ..
        } => return exited(run, i, r, (pid, killed_by_engine), now, fx),
        _ if round.retiring => return,
        _ => {}
    }
    // Every stream event: the clock, and the end of a rate-limit streak (decision 32).
    round.last_event = now;
    // Ruling T12-O1: worker activity inside the interrupt grace ends the grace; the
    // next silence is a stall. `TurnEnded` makes the same change itself.
    if !matches!(signal, AgentSignal::TurnEnded { .. })
        && matches!(round.stall, StallState::Interrupted { .. })
    {
        round.stall = StallState::Nudged;
    }
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
        AgentSignal::TurnStarted => {
            // Ruling T12-R4 (N3-2): a turn Claude starts by itself (a background
            // sub-agent finished) is a turn; a delivered one was counted already.
            if !round.turn_open {
                round.turns += 1;
            }
            round.turn_open = true;
        }
        AgentSignal::ToolUse { .. } => {
            round.tool_calls += 1;
            // Ruling T12-m5: the task's total counts its worker rounds only.
            if worker {
                let task = &mut run.tasks[i];
                task.spent_total.tool_calls = task.spent_total.tool_calls.saturating_add(1);
                check_budget(run, i, now, fx);
            }
        }
        AgentSignal::ApiRetry { error, delay_ms } => {
            round.rate_limited_until = Some(not_before(now, delay_ms.div_ceil(1000)));
            if error == RATE_LIMIT {
                round.in_retry_streak = true;
                if !streak {
                    count_rate_limit(run, i, r);
                }
            }
        }
        AgentSignal::PermissionDenied { tool, reason } => {
            round.denials += 1;
            round.turn_denied.push(tool.clone());
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
                fallback::fallback(run, i, fx);
            }
        }
        AgentSignal::TurnEnded {
            outcome,
            usage,
            denials,
        } => turn_ended(run, i, r, outcome, usage, denials, streak, now, fx),
        AgentSignal::Spend { usage } => {
            add_usage(&mut round.usage, usage);
            // Ruling T12-m5: the task's total counts its worker rounds only.
            if worker {
                let task = &mut run.tasks[i];
                task.spent_total.tokens = task.spent_total.tokens.saturating_add(usage.billable());
                check_budget(run, i, now, fx);
            }
        }
        AgentSignal::Activity
        | AgentSignal::ProcessStarted { .. }
        | AgentSignal::ProcessExited { .. } => {}
    }
}

pub(super) fn count_rate_limit(run: &mut Run, i: usize, r: usize) {
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
    // Every denial counted recorded itself as the latest (ruling T12-later).
    let last = round.last_denial.clone().unwrap_or_default();
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
    denials: Vec<String>,
    streak: bool,
    now: u64,
    fx: &mut Vec<Effect>,
) {
    let round = &mut run.tasks[i].rounds[r];
    let worker = round.role == AgentRole::Worker;
    round.turn_open = false;
    round.closed_pid = round.pid;
    if let Some(usage) = usage {
        add_usage(&mut round.usage, usage);
        // Ruling T12-m5: the task's total counts its worker rounds only.
        if worker {
            let task = &mut run.tasks[i];
            task.spent_total.tokens = task.spent_total.tokens.saturating_add(usage.billable());
        }
    }
    let round = &mut run.tasks[i].rounds[r];
    // Decision 27: only the denials the turn's events did not already report, matched
    // by tool (review m-4). The last one listed only here names its tool.
    let mut seen = std::mem::take(&mut round.turn_denied);
    let mut new = Vec::new();
    for tool in denials {
        match seen.iter().position(|t| *t == tool) {
            Some(k) => {
                seen.remove(k);
            }
            None => new.push(tool),
        }
    }
    round.denials += new.len() as u32;
    if let Some(tool) = new.last() {
        round.last_denial = Some(format!("{tool}: {DENIAL_LISTED_REASON}"));
    }
    let had_done = std::mem::take(&mut round.turn_had_task_done);
    // The interrupted turn ended: its queued `stall_nudge` goes next. This comes before
    // any early return, so a task blocked meanwhile is not left interrupted (ruling
    // T12-I4b).
    // Ruling T12-R4 (N3-1): still interrupted after activity ended the grace.
    let interrupted = std::mem::take(&mut round.interrupted)
        || matches!(round.stall, StallState::Interrupted { .. });
    if matches!(round.stall, StallState::Interrupted { .. }) {
        round.stall = StallState::Nudged;
    }
    if !worker {
        return review::turn_ended(run, i, r, outcome, streak, now, fx);
    }
    if run.tasks[i].state != TaskState::Working {
        return;
    }
    if check_denials(run, i, r, now, fx) || check_budget(run, i, now, fx) {
        return;
    }
    let claimed = run.tasks[i].claim.is_some();
    let round = &mut run.tasks[i].rounds[r];
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
            fallback::fallback(run, i, fx);
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
                at: not_before(now, wait),
                rate_limit: true,
            };
            round.rate_limited_until = Some(not_before(now, wait));
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
                at: not_before(now, wait),
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
/// `(pid, killed)`: the exited process and whether the engine killed it.
fn exited(
    run: &mut Run,
    i: usize,
    r: usize,
    (pid, killed): (u32, bool),
    now: u64,
    fx: &mut Vec<Effect>,
) {
    let working = run.tasks[i].state == TaskState::Working;
    let round = &mut run.tasks[i].rounds[r];
    let worker = round.role == AgentRole::Worker;
    // Codex runs one process per turn: its exit between turns is the normal end of one,
    // and the round has no process until the next starts. A retiring round's exit ends
    // it (rulings T13-I2, T13-R2).
    if !killed && !round.turn_open && round.route.runtime == Runtime::Codex && !round.retiring {
        round.pid = None;
        return;
    }
    round.pid = None;
    round.exited_pid = Some(pid);
    // Decision 35's resume rule for reviewers (M8a.13).
    if !worker {
        return review::exited(run, i, r, killed, now, fx);
    }
    // Ruling T12-I2: an exit while an interrupt is pending is the interrupted turn's end
    // (a Codex interrupt is a `ProcessExited` with no `TurnEnded`, M8a.1), not a death:
    // the queued `stall_nudge` goes next, as a delivery (Codex spawns `exec resume`) or,
    // for Claude, a resume of the ended round. The grace is over. A turn interrupted
    // before its session had an id (only Codex's can be: Claude's id is set at launch)
    // has nothing to resume: rung 2, with the nudge at the end of the fresh session's
    // prompt (ruling T12-later).
    // Ruling T12-R4 (N3-1): still interrupted after activity ended the grace.
    let interrupted = round.interrupted || matches!(round.stall, StallState::Interrupted { .. });
    if !killed && working && round.turn_open && interrupted {
        round.interrupted = false;
        if round.session_id.is_none() {
            end_round(round, now);
            let reason = "its turn was interrupted before its session had an id".to_string();
            ladder::rung2(run, i, reason, now, fx);
            let nudge = stall_nudge(run.limits.stall_after_secs / 60);
            if let Some(fresh) = run.tasks[i].fresh_session.as_mut() {
                fresh.append = Some(nudge);
            }
            return;
        }
        round.stall = StallState::Nudged;
        round.turn_open = false;
        if round.route.runtime != Runtime::Codex {
            end_round(round, now);
        }
        return;
    }
    if killed || !working || !round.turn_open {
        let between_turns = !killed && working && !round.turn_open;
        end_round(round, now);
        // M8a.25: a hand-back that came before this kill's exit resumes it now.
        if killed && worker {
            ladder::reopen_stopped(&mut run.tasks[i]);
        }
        let round = &mut run.tasks[i].rounds[r];
        // Ruling T12-I4a: a fallback waiting for sub-agents runs now; they died with
        // the process.
        if between_turns && round.fallback_waiting {
            round.fallback_waiting = false;
            round.open_subagents.clear();
            fallback::fallback(run, i, fx);
        }
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
    run.tasks[i].rounds[r].resume_op = Some(op);
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
    round.interrupted = false;
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
        let Some(r) = worker_round(task) else {
            continue;
        };
        // Ruling T12-I4a: a round that ended between turns keeps its failed turn's
        // timer; the continue then goes out as a resume.
        let round = &task.rounds[r];
        let resumable = round.ended && !round.retiring && round.session_id.is_some();
        if !live(round) && !resumable {
            continue;
        }
        if let FailedTurn::WaitingContinue { at, rate_limit } = round.failed_turn
            && now >= at
        {
            let round = &mut run.tasks[i].rounds[r];
            round.failed_turn = FailedTurn::ContinueSent { rate_limit };
            round.rate_limited_until = None;
            let reason = round.failed_error.clone().unwrap_or_default();
            let id = run.tasks[i].id().to_string();
            outbox::queue(run, &id, rate_limit_continue(&reason), now);
        }
        fallback::retry_count(run, i, now, fx);
        if !live(&run.tasks[i].rounds[r]) || check_budget(run, i, now, fx) {
            continue;
        }
        let round = &run.tasks[i].rounds[r];
        // Ruling T12-later: no stall while the worker's `task_done` is being checked.
        if !round.turn_open || run.tasks[i].claim.is_some() {
            continue;
        }
        let stall_after = run.limits.stall_after_secs;
        let silent = now >= stall_due(round, stall_after);
        match round.stall {
            StallState::Interrupted { deadline } if now >= deadline => {
                let reason = "the interrupt did not end its turn".to_string();
                ladder::stall(run, i, reason, now, fx);
            }
            StallState::Watching if silent => {
                let window_id = round.window_id.unwrap_or_default();
                fx.push(Effect::Interrupt { window_id });
                let round = &mut run.tasks[i].rounds[r];
                round.stall = StallState::Interrupted {
                    deadline: not_before(now, INTERRUPT_GRACE_SECS),
                };
                round.interrupted = true;
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
