//! Milestone 9 task M9.8: a sub-planner on M8b's scout machine (decision 31). The
//! machine's rules are the scouts'; only its texts are the sub-planner's.

use std::time::Duration;

use proto::ScoutState;

use super::contract::{SCOUT_NUDGE, scout_wrap_up};
use super::machine::{
    PLANNER_TEXTS, SCOUT_TEXTS, ScoutEffect, ScoutEvent, ScoutLimits, ScoutMachine, step,
};
use crate::run::orch::contract::{PLANNER_NUDGE, planner_wrap_up};

const RETIRE: Duration = Duration::from_secs(30);

fn planner(max_tool_calls: u32) -> ScoutLimits {
    ScoutLimits {
        timeout_secs: 2400,
        max_tool_calls,
        send_mid_turn: true,
        texts: PLANNER_TEXTS,
    }
}

fn started(limits: &ScoutLimits) -> ScoutMachine {
    let (machine, _) = step(
        ScoutMachine::default(),
        ScoutEvent::Start { now: 1000 },
        limits,
    );
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

/// Decision 31: a planner's nudge, wrap-up and submit tool are its own; a scout keeps
/// M8b's.
#[test]
fn planner_machine_uses_its_own_texts() {
    assert_eq!(PLANNER_TEXTS.nudge, PLANNER_NUDGE);
    assert_eq!((PLANNER_TEXTS.wrap_up)(7), planner_wrap_up(7));
    assert_eq!(PLANNER_TEXTS.submit_tool, "mcp__anthrex__submit_epic");
    assert_eq!(SCOUT_TEXTS.nudge, SCOUT_NUDGE);
    assert_eq!((SCOUT_TEXTS.wrap_up)(7), scout_wrap_up(7));
    assert_eq!(SCOUT_TEXTS.submit_tool, "mcp__anthrex__submit_scout_report");
    // The submit tool does not count toward the budget; the scout's does for a planner.
    use super::machine::scout_event;
    use crate::headless::SessionEvent;
    let tool = |name: &str| SessionEvent::ToolUse {
        id: "u1".into(),
        name: name.into(),
        input: serde_json::Value::Null,
        parent: None,
    };
    let mut ended = std::collections::HashSet::new();
    let runtime = proto::Runtime::Claude;
    let submit = tool("mcp__anthrex__submit_epic");
    assert_eq!(
        scout_event(&submit, Some(1), runtime, &mut ended, &PLANNER_TEXTS),
        None
    );
    assert_eq!(
        scout_event(&submit, Some(1), runtime, &mut ended, &SCOUT_TEXTS),
        Some(ScoutEvent::ToolUse)
    );
    let report = tool("mcp__anthrex__submit_scout_report");
    assert_eq!(
        scout_event(&report, Some(1), runtime, &mut ended, &PLANNER_TEXTS),
        Some(ScoutEvent::ToolUse)
    );
}

#[test]
fn turn_without_submit_is_nudged_once_then_fails() {
    let l = planner(200);
    let machine = started(&l);
    let (machine, effects) = step(machine, ScoutEvent::TurnEnded { usage: None }, &l);
    assert_eq!(effects, [ScoutEffect::Send(PLANNER_NUDGE.to_string())]);
    let (machine, effects) = step(machine, ScoutEvent::TurnEnded { usage: None }, &l);
    let reason = "the sub-planner ended two turns without an accepted epic";
    assert_eq!(effects, failed(reason, true));
    assert_eq!(machine.state, ScoutState::Failed);
    assert_eq!(machine.failure.as_deref(), Some(reason));
}

#[test]
fn tool_call_wrap_up_then_kill() {
    let l = planner(20);
    let mut machine = started(&l);
    for n in 1..=29u32 {
        let (next, effects) = step(machine, ScoutEvent::ToolUse, &l);
        machine = next;
        if n == 20 {
            assert_eq!(effects, [ScoutEffect::Send(planner_wrap_up(20))], "at {n}");
        } else {
            assert!(effects.is_empty(), "at {n}: {effects:?}");
        }
    }
    let (machine, effects) = step(machine, ScoutEvent::ToolUse, &l);
    let reason = "the sub-planner used 30 tool calls without an accepted epic";
    assert_eq!(effects, failed(reason, true));
    assert_eq!(machine.state, ScoutState::Failed);
}

#[test]
fn timeout_fails() {
    let l = planner(200);
    let machine = started(&l);
    let (machine, effects) = step(machine, ScoutEvent::Tick { now: 3399 }, &l);
    assert!(effects.is_empty());
    let (machine, effects) = step(machine, ScoutEvent::Tick { now: 3400 }, &l);
    assert_eq!(
        effects,
        failed("the sub-planner ran longer than 2400 s", true)
    );
    assert_eq!(machine.state, ScoutState::Failed);
    // An exit names the sub-planner too; the engine's stop gives its own reason.
    let (_, effects) = step(started(&l), ScoutEvent::Exited { code: Some(3) }, &l);
    let reason = "the sub-planner's process exited without an accepted epic (code 3)";
    assert_eq!(effects, failed(reason, false));
    let halt = ScoutEvent::Halt {
        reason: "the sub-planner's epic was rejected 5 times".into(),
    };
    let (machine, effects) = step(started(&l), halt, &l);
    assert_eq!(
        effects,
        failed("the sub-planner's epic was rejected 5 times", true)
    );
    assert_eq!(machine.state, ScoutState::Failed);
}
