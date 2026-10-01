//! M9.9 review fixes, I3: a research task's session follows M8a's worker rules
//! (decision 35, final): a rate-limited turn waits and is continued, a stall gets the
//! interrupt-and-nudge ladder, a second death in a round is a stall, and budgets apply;
//! where a worker gets rung 2's fresh session, a research task gets a fresh research
//! session.

use proto::{AgentRole, BlockReason, TaskState};

use super::fixture::*;
use super::kinds::{research, research_window, running, window_task};
use super::merge::pending_one;
use crate::headless::FailureKind;
use crate::run::contract::rate_limit_continue;
use crate::run::engine::{
    AgentSignal, Effect, EventKind, INTERRUPT_GRACE_SECS, OpResult, TurnOutcome,
};
use crate::run::model::StallState;
use crate::scout::contract::SCOUT_NUDGE;

/// A running run of research task `r1`, its session launched with its turn open;
/// returns the fixture and the session's window.
pub(super) fn researching() -> (Fixture, u32) {
    let mut fx = running("", &[research("r1", "")]);
    fx.tick();
    let window = research_window(&mut fx, "r1");
    fx.signal(window, AgentSignal::TurnStarted);
    (fx, window)
}

fn delivered(effects: &[Effect], window: u32, needle: &str) -> bool {
    effects.iter().any(|e| {
        matches!(e, Effect::Deliver { window_id, text, .. } if *window_id == window && text.contains(needle))
    })
}

pub(super) fn sessions(fx: &Fixture) -> usize {
    fx.ops("CreateWindow")
        .iter()
        .filter(|(_, k)| window_task(k) == "r1")
        .count()
}

pub(super) fn round(fx: &mut Fixture) -> &mut crate::run::model::AgentRound {
    let task = fx.task_mut("r1");
    let r = task
        .rounds
        .iter()
        .rposition(|r| r.role == AgentRole::Scout)
        .unwrap();
    &mut task.rounds[r]
}

#[test]
fn a_rate_limited_research_turn_waits_then_continues() {
    let (mut fx, window) = researching();
    let error = "429: rate limited".to_string();
    let effects = fx.turn_ended(
        window,
        TurnOutcome::Failed {
            error: error.clone(),
            kind: FailureKind::RateLimit,
        },
    );
    // Not a turn without a report: no nudge, and the task works on.
    assert!(!delivered(&effects, window, SCOUT_NUDGE), "{effects:#?}");
    assert_eq!(fx.task("r1").state, TaskState::Working);
    // Nothing before the wait is over.
    let wait = fx.run().limits.rate_limit_retry_secs;
    let effects = fx.send(fx.now + wait / 2, EventKind::Tick);
    assert!(!delivered(&effects, window, "API error"), "{effects:#?}");
    let effects = fx.send(fx.now + wait + 2, EventKind::Tick);
    assert!(
        delivered(&effects, window, &rate_limit_continue(&error)),
        "{effects:#?}"
    );
    // The next turn without a report is the first one: nudged, not blocked.
    let effects = fx.turn_completed(window);
    assert!(delivered(&effects, window, SCOUT_NUDGE), "{effects:#?}");
    assert_eq!(fx.task("r1").state, TaskState::Working);
}

#[test]
fn a_research_stall_is_interrupted_and_nudged_before_it_counts() {
    let (mut fx, window) = researching();
    let stall = fx.run().limits.stall_after_secs;
    let effects = fx.send(fx.now + stall + 2, EventKind::Tick);
    assert!(
        effects.contains(&Effect::Interrupt { window_id: window }),
        "{effects:#?}"
    );
    assert_eq!(fx.task("r1").state, TaskState::Working);
    let nudge = crate::run::engine::research::research_stall_nudge(stall / 60);
    assert!(
        fx.run().outbox.iter().any(|m| m.text == nudge),
        "{:#?}",
        fx.run().outbox
    );
    // The interrupt did not end the turn within its grace: a stall, and a fresh
    // research session.
    let effects = fx.send(fx.now + INTERRUPT_GRACE_SECS + 2, EventKind::Tick);
    assert!(
        effects.contains(&Effect::KillWindow { window_id: window }),
        "{effects:#?}"
    );
    let task = fx.task("r1");
    assert_eq!(task.state, TaskState::Working);
    assert_eq!((task.stalls, task.failures, task.rung), (1, 1, 2));
    assert_eq!(sessions(&fx), 2);
}

#[test]
fn a_third_research_failure_blocks_the_task() {
    let (mut fx, _) = researching();
    fx.task_mut("r1").failures = 2;
    round(&mut fx).stall = StallState::Nudged;
    let stall = fx.run().limits.stall_after_secs;
    fx.send(fx.now + stall + 2, EventKind::Tick);
    let task = fx.task("r1");
    assert_eq!(task.state, TaskState::Blocked);
    let block = task.block.clone().unwrap();
    assert_eq!(block.reason, BlockReason::Environment);
    assert!(
        block
            .text
            .starts_with("stalled 1 times (3 failures in all); last: no stream event for"),
        "{}",
        block.text
    );
    assert_eq!(sessions(&fx), 1);
}

/// Ruling F-1: a client error ends a research task's session at once, as an
/// authentication failure does, with no continue.
#[test]
fn a_research_client_error_blocks_at_once() {
    let (mut fx, window) = researching();
    let error = "The model is not supported".to_string();
    let effects = fx.turn_ended(
        window,
        TurnOutcome::Failed {
            error: error.clone(),
            kind: FailureKind::ClientError,
        },
    );
    assert!(!delivered(&effects, window, "API error"), "{effects:#?}");
    let task = fx.task("r1");
    assert_eq!(task.state, TaskState::Blocked);
    let block = task.block.clone().unwrap();
    assert_eq!(
        (block.reason, block.text),
        (BlockReason::Environment, error)
    );
}

fn exit(fx: &mut Fixture, window: u32, pid: u32) -> Vec<Effect> {
    fx.signal(
        window,
        AgentSignal::ProcessExited {
            code: Some(1),
            killed_by_engine: false,
            pid,
        },
    )
}

#[test]
fn a_second_research_death_in_a_round_is_a_stall() {
    let (mut fx, window) = researching();
    fx.signal(window, AgentSignal::ProcessStarted { pid: 4_101 });
    exit(&mut fx, window, 4_101);
    let (op, _) = pending_one(&fx, "ResumeSession", Some("r1"));
    fx.done(op, OpResult::Resumed);
    fx.signal(window, AgentSignal::ProcessStarted { pid: 4_102 });
    fx.signal(window, AgentSignal::TurnStarted);
    exit(&mut fx, window, 4_102);
    let task = fx.task("r1");
    assert_eq!(task.state, TaskState::Working, "{:#?}", task.history);
    assert_eq!((task.stalls, task.failures), (1, 1));
    assert_eq!(sessions(&fx), 2);
}

#[test]
fn research_budgets_apply() {
    let (mut fx, window) = researching();
    let budget = fx.task("r1").budget;
    // The soft limit: one wrap-up, to the session.
    round(&mut fx).tool_calls = budget.tool_calls;
    fx.tick();
    fx.tick();
    let wrap_ups = fx
        .run()
        .outbox
        .iter()
        .filter(|m| {
            m.text
                .starts_with("[anthrex] This research task has used its budget")
        })
        .count();
    assert_eq!(wrap_ups, 1, "{:#?}", fx.run().outbox);
    assert_eq!(fx.task("r1").state, TaskState::Working);
    // A hard breach (1.5 times the budget): a fresh session.
    let hard = budget.tool_calls * 3 / 2;
    round(&mut fx).tool_calls = hard;
    let effects = fx.tick();
    assert!(
        effects.contains(&Effect::KillWindow { window_id: window }),
        "{effects:#?}"
    );
    assert_eq!(fx.task("r1").budget_exceeded, 1);
    assert_eq!(fx.task("r1").state, TaskState::Working);
    assert_eq!(sessions(&fx), 2);
    // The second breach blocks it.
    let second = research_window(&mut fx, "r1");
    fx.signal(second, AgentSignal::TurnStarted);
    round(&mut fx).tool_calls = hard;
    fx.tick();
    let task = fx.task("r1");
    assert_eq!(task.state, TaskState::Blocked);
    let text = task.block.clone().unwrap().text;
    assert!(
        text.starts_with("exceeded its budget twice; last: "),
        "{text}"
    );
    // The task's total over its sessions at the next size's budget is rung 4's
    // ceiling: `blocked(human)`.
    let (mut fx, _) = researching();
    let ceiling = fx.run().limits.budget_m;
    round(&mut fx).tool_calls = ceiling.tool_calls;
    fx.tick();
    let block = fx.task("r1").block.clone().unwrap();
    assert_eq!(block.reason, proto::BlockReason::Human, "{}", block.text);
}
