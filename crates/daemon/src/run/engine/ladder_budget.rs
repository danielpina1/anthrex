//! Decision 40's budgets for decision 38's ladder: a session's and a task's spend, the
//! soft and hard limits, rung 4's ceiling, and the budget check of a live worker
//! session. Pure (design decision 2).

use proto::{AgentRole, Budget, Size, Spend, TaskState};

use super::ladder::{breach, live, rung4, worker_round};
use super::{Effect, outbox};
use crate::run::contract::budget_wrap_up;
use crate::run::model::{AgentRound, Run, Task};
use crate::run::refit::{self, SizeClass};

/// A worker round's session spend (decision 40): its tool calls, its wall-clock
/// seconds since it started less those its task's clock was stopped (`stopped` is the
/// open stop, rulings T15-I2 and T15-I3), and its billable tokens.
pub(crate) fn round_spend(round: &AgentRound, stopped: Option<u64>, now: u64) -> Spend {
    let end = round.ended_at.unwrap_or(now);
    let open = stopped.map_or(0, |since| end.saturating_sub(since.max(round.started_at)));
    Spend {
        tool_calls: round.tool_calls,
        secs: end
            .saturating_sub(round.started_at)
            .saturating_sub(round.excused_secs + open),
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
        .map(|r| round_spend(r, task.clock.stopped, now).secs)
        .sum();
    Spend {
        secs,
        ..task.spent_total
    }
}

/// Some axis reached: `tool_calls`, minutes or (when set) tokens at or past `budget`.
pub(super) fn reached(spend: Spend, budget: Budget) -> bool {
    spend.tool_calls >= budget.tool_calls
        || spend.secs >= u64::from(budget.minutes) * 60
        || budget.tokens.is_some_and(|t| spend.tokens >= t)
}

/// Decision 40's hard limit: spend at 1.5 × the budget on any axis is a breach,
/// compared in integers (`2 × spend >= 3 × budget`, so 7 of 5 tool calls is not and 8
/// is; 450 seconds of 5 minutes is); what was breached.
pub(super) fn breached(spend: Spend, budget: Budget) -> Option<String> {
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

/// Rung 4's ceiling (milestone 9.5 rulings RH-4 and T9-1), from the run's frozen
/// budgets: `refit::ceiling`, the one rule.
pub(super) fn ceiling(run: &Run, task: &Task) -> Budget {
    let class = match (task.hub, task.size) {
        (true, _) => SizeClass::Hub,
        (false, Size::S) => SizeClass::S,
        _ => SizeClass::M,
    };
    let l = &run.limits;
    let own = match class {
        SizeClass::S => l.budget_s,
        SizeClass::M => l.budget_m,
        SizeClass::Hub => l.budget_hub.unwrap_or(l.budget_m),
    };
    refit::ceiling(class, own, l.budget_m, l.budget_l)
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
    // Ruling T15-C1: the spend since the last retry.
    let total = super::clock::epoch_spend(task, now);
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
    let spend = round_spend(&task.rounds[r], task.clock.stopped, now);
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
