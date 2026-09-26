//! M8b.9: the scout lifecycle machine (decision 14).

use std::time::Duration;

use proto::{ScoutState, TokenUsage};

use super::contract::{SCOUT_NUDGE, scout_wrap_up};
use super::machine::{ScoutEffect, ScoutEvent, ScoutLimits, ScoutMachine, step};

const GRACE: Duration = Duration::from_secs(30);
const RETIRE: Duration = Duration::from_secs(30);

fn limits(max_tool_calls: u32) -> ScoutLimits {
    ScoutLimits {
        timeout_secs: 900,
        max_tool_calls,
    }
}

fn started(limits: &ScoutLimits) -> ScoutMachine {
    let (machine, effects) = step(
        ScoutMachine::default(),
        ScoutEvent::Start { now: 1000 },
        limits,
    );
    assert!(effects.is_empty());
    assert_eq!(machine.state, ScoutState::Working);
    assert_eq!(machine.started_at, 1000);
    machine
}

fn failed(reason: &str, kill: bool) -> Vec<ScoutEffect> {
    let mut effects = Vec::new();
    if kill {
        effects.push(ScoutEffect::Kill);
    }
    effects.push(ScoutEffect::Finished(Err(reason.to_string())));
    effects.push(ScoutEffect::RemoveAfter(RETIRE));
    effects
}

#[test]
fn the_machine_uses_the_drivers_grace_and_retirement() {
    assert_eq!(GRACE, crate::run::driver::INTERRUPT_GRACE);
    assert_eq!(RETIRE, crate::run::driver::RETIRE_AFTER);
}

#[test]
fn machine_nudges_once_then_fails() {
    let l = limits(120);
    let machine = started(&l);
    let (machine, effects) = step(machine, ScoutEvent::TurnEnded { usage: None }, &l);
    assert_eq!(effects, [ScoutEffect::Send(SCOUT_NUDGE.to_string())]);
    assert_eq!(machine.state, ScoutState::Working);
    let (machine, effects) = step(machine, ScoutEvent::TurnEnded { usage: None }, &l);
    let reason = "the scout ended two turns without a report";
    assert_eq!(effects, failed(reason, true));
    assert_eq!(machine.state, ScoutState::Failed);
    assert_eq!(machine.failure.as_deref(), Some(reason));
    // A failed scout does nothing more.
    let (_, effects) = step(machine, ScoutEvent::TurnEnded { usage: None }, &l);
    assert!(effects.is_empty());
}

#[test]
fn machine_wraps_up_then_kills_at_1_5x_the_tool_budget() {
    let l = limits(11);
    let mut machine = started(&l);
    for n in 1..=16u32 {
        let (next, effects) = step(machine, ScoutEvent::ToolUse, &l);
        machine = next;
        assert_eq!(machine.tool_calls, n);
        if n == 11 {
            assert_eq!(effects, [ScoutEffect::Send(scout_wrap_up(11))], "at {n}");
        } else {
            assert!(effects.is_empty(), "at {n}: {effects:?}");
        }
    }
    assert!(machine.wrap_up_sent);
    let (machine, effects) = step(machine, ScoutEvent::ToolUse, &l);
    let reason = "the scout used 17 tool calls without a report";
    assert_eq!(effects, failed(reason, true));
    assert_eq!(machine.state, ScoutState::Failed);
}

#[test]
fn machine_times_out() {
    let l = limits(120);
    let machine = started(&l);
    let (machine, effects) = step(machine, ScoutEvent::Tick { now: 1899 }, &l);
    assert!(effects.is_empty());
    let (machine, effects) = step(machine, ScoutEvent::Tick { now: 1900 }, &l);
    assert_eq!(effects, failed("the scout ran longer than 900 s", true));
    assert_eq!(machine.state, ScoutState::Failed);
}

#[test]
fn machine_exit_without_report_fails() {
    let l = limits(120);
    let (machine, effects) = step(started(&l), ScoutEvent::Exited { code: Some(3) }, &l);
    assert_eq!(
        effects,
        failed(
            "the scout's process exited without a report (code 3)",
            false
        )
    );
    assert_eq!(machine.state, ScoutState::Failed);
    let (_, effects) = step(started(&l), ScoutEvent::Exited { code: None }, &l);
    assert_eq!(
        effects,
        failed(
            "the scout's process exited without a report (code unknown)",
            false
        )
    );
}

#[test]
fn machine_report_closes_stdin_then_kills_and_removes() {
    let l = limits(120);
    let (machine, effects) = step(started(&l), ScoutEvent::ReportAccepted, &l);
    assert_eq!(
        effects,
        [
            ScoutEffect::Finished(Ok(())),
            ScoutEffect::CloseStdin,
            ScoutEffect::KillAfter(GRACE),
            ScoutEffect::RemoveAfter(RETIRE),
        ]
    );
    assert_eq!(machine.state, ScoutState::Reported);
    // After the report, its turn's end and its exit are the expected ones.
    for event in [
        ScoutEvent::TurnEnded { usage: None },
        ScoutEvent::Exited { code: Some(0) },
        ScoutEvent::Tick { now: 99_999 },
        ScoutEvent::ToolUse,
        ScoutEvent::ReportAccepted,
    ] {
        let (next, effects) = step(machine.clone(), event, &l);
        assert!(effects.is_empty());
        assert_eq!(next.state, ScoutState::Reported);
    }
}

#[test]
fn machine_stop_kills_and_fails() {
    let l = limits(120);
    let (machine, effects) = step(started(&l), ScoutEvent::Stop, &l);
    assert_eq!(effects, failed("stopped by the user", true));
    assert_eq!(machine.state, ScoutState::Failed);
}

#[test]
fn machine_sums_usage() {
    let l = limits(120);
    let usage = |n: u64| TokenUsage {
        input: n,
        output: 2 * n,
        cache_read: 3 * n,
        cache_write: 4 * n,
    };
    let machine = started(&l);
    let (machine, _) = step(
        machine,
        ScoutEvent::TurnEnded {
            usage: Some(usage(1)),
        },
        &l,
    );
    let (machine, _) = step(machine, ScoutEvent::ReportAccepted, &l);
    // The turn that carried the report still counts.
    let (machine, _) = step(
        machine,
        ScoutEvent::TurnEnded {
            usage: Some(usage(10)),
        },
        &l,
    );
    let (machine, _) = step(machine, ScoutEvent::TurnEnded { usage: None }, &l);
    assert_eq!(machine.usage, usage(11));
}
