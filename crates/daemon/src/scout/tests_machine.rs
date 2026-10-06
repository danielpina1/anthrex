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
        send_mid_turn: true,
        nudges: true,
        texts: super::machine::SCOUT_TEXTS,
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

/// Ruling I1: a runtime that cannot take a message mid-turn (Codex) gets the wrap-up at
/// the next turn end, in the nudge's place, exactly once.
#[test]
fn machine_owes_the_wrap_up_to_the_turn_end_without_mid_turn_sends() {
    let l = ScoutLimits {
        send_mid_turn: false,
        nudges: true,
        ..limits(11)
    };
    let mut machine = started(&l);
    for n in 1..=12u32 {
        let (next, effects) = step(machine, ScoutEvent::ToolUse, &l);
        machine = next;
        assert!(effects.is_empty(), "at {n}: {effects:?}");
    }
    assert!(machine.wrap_up_sent && machine.wrap_up_pending);
    let (machine, effects) = step(machine, ScoutEvent::TurnEnded { usage: None }, &l);
    assert_eq!(effects, [ScoutEffect::Send(scout_wrap_up(12))]);
    assert!(!machine.wrap_up_pending);
    // Never again: the next turn end without a report fails the scout as usual.
    let (machine, effects) = step(machine, ScoutEvent::ToolUse, &l);
    assert!(effects.is_empty());
    let (machine, effects) = step(machine, ScoutEvent::TurnEnded { usage: None }, &l);
    assert_eq!(
        effects,
        failed("the scout ended two turns without a report", true)
    );
    assert_eq!(machine.state, ScoutState::Failed);
}

/// Ruling M3: with an even budget the kill comes at exactly 1.5 times it.
#[test]
fn machine_kills_at_1_5x_an_even_budget() {
    let l = limits(10);
    let mut machine = started(&l);
    for n in 1..=14u32 {
        let (next, effects) = step(machine, ScoutEvent::ToolUse, &l);
        machine = next;
        if n == 10 {
            assert_eq!(effects, [ScoutEffect::Send(scout_wrap_up(10))]);
        } else {
            assert!(effects.is_empty(), "at {n}: {effects:?}");
        }
    }
    let (_, effects) = step(machine, ScoutEvent::ToolUse, &l);
    assert_eq!(
        effects,
        failed("the scout used 15 tool calls without a report", true)
    );
}

#[test]
fn limits_send_mid_turn_only_on_claude() {
    let scouts = config::Scouts::default();
    let claude = ScoutLimits::new(&scouts, proto::Runtime::Claude);
    assert_eq!(
        claude,
        ScoutLimits {
            timeout_secs: 900,
            max_tool_calls: 120,
            send_mid_turn: true,
            nudges: true,
            texts: super::machine::SCOUT_TEXTS,
        }
    );
    assert!(!ScoutLimits::new(&scouts, proto::Runtime::Codex).send_mid_turn);
}

/// Ruling I2 and M2: the translation of session events.
#[test]
fn session_events_become_machine_events() {
    use super::machine::scout_event;
    use crate::headless::{SessionEvent, TurnOutcome};
    use proto::Runtime;
    use std::collections::HashSet;
    let tool = |name: &str| SessionEvent::ToolUse {
        id: "u1".into(),
        name: name.into(),
        input: serde_json::Value::Null,
        parent: None,
    };
    let turn_end = SessionEvent::TurnEnded {
        outcome: TurnOutcome::Completed,
        usage: None,
        denials: Vec::new(),
    };
    let exit = SessionEvent::ProcessExited {
        code: Some(0),
        signal: None,
    };
    for runtime in [Runtime::Claude, Runtime::Codex] {
        let mut ended = HashSet::new();
        let mut event = |e: &SessionEvent, pid| {
            scout_event(e, pid, runtime, &mut ended, &super::machine::SCOUT_TEXTS)
        };
        // The report call itself does not count toward the budget.
        assert_eq!(
            event(&tool("mcp__anthrex__submit_scout_report"), Some(1)),
            None
        );
        assert_eq!(event(&tool("Read"), Some(1)), Some(ScoutEvent::ToolUse));
        assert_eq!(event(&tool("Bash"), Some(1)), Some(ScoutEvent::ToolUse));
        assert_eq!(
            event(&SessionEvent::TurnStarted, Some(1)),
            None,
            "{runtime:?}"
        );
        assert_eq!(
            event(&turn_end, Some(1)),
            Some(ScoutEvent::TurnEnded { usage: None })
        );
        // An exit of a process whose turn ended: Codex's normal end of a turn.
        let after_turn = event(&exit, Some(1));
        match runtime {
            Runtime::Codex => assert_eq!(after_turn, None),
            _ => assert_eq!(after_turn, Some(ScoutEvent::Exited { code: Some(0) })),
        }
        // A process that exits before its turn ends is the session's end on both.
        assert_eq!(
            event(&exit, Some(2)),
            Some(ScoutEvent::Exited { code: Some(0) }),
            "{runtime:?}"
        );
    }
}

/// 2026-10-06: a session that died at startup fails the scout at once with why, instead
/// of being nudged into a second dead process.
#[test]
fn a_startup_failure_fails_the_scout_at_once_with_its_reason() {
    let l = limits(120);
    let reason = "claude exited at startup (code 1): no config";
    let (machine, effects) = step(
        started(&l),
        ScoutEvent::StartupFailed {
            reason: reason.into(),
        },
        &l,
    );
    let text = format!("the scout could not start: {reason}");
    assert_eq!(effects, failed(&text, true));
    assert_eq!(machine.failure.as_deref(), Some(text.as_str()));
    // The exit that follows changes nothing.
    let (_, effects) = step(machine, ScoutEvent::Exited { code: Some(1) }, &l);
    assert!(effects.is_empty());
}

/// The driver's `StartupFailed`, and a turn failed on Claude's missing sandbox (from
/// stderr or from a `result`), are both a startup failure, the latter with what to
/// install.
#[test]
fn startup_failures_become_machine_events() {
    use super::machine::scout_event;
    use crate::headless::failure::SANDBOX_HINT;
    use crate::headless::{FailureKind, SessionEvent, TurnOutcome};
    use std::collections::HashSet;
    let mut ended = HashSet::new();
    let mut event = |e: &SessionEvent| {
        scout_event(
            e,
            Some(1),
            proto::Runtime::Claude,
            &mut ended,
            &super::machine::SCOUT_TEXTS,
        )
    };
    assert_eq!(
        event(&SessionEvent::StartupFailed { reason: "r".into() }),
        Some(ScoutEvent::StartupFailed { reason: "r".into() })
    );
    let sandbox = SessionEvent::TurnEnded {
        outcome: TurnOutcome::Failed {
            error: "sandbox required but unavailable: socat not installed".into(),
            kind: FailureKind::SandboxUnavailable,
        },
        usage: None,
        denials: Vec::new(),
    };
    assert_eq!(
        event(&sandbox),
        Some(ScoutEvent::StartupFailed {
            reason: format!("sandbox required but unavailable: socat not installed{SANDBOX_HINT}")
        })
    );
}

/// 2026-10-06 (Ubuntu 24.04 and later): commands fail inside a sandbox that cannot
/// start. The scout runs on, but its failure says so, with the first such error.
#[test]
fn a_broken_sandbox_is_named_in_the_scouts_failure() {
    let l = limits(120);
    let note = "Claude's sandbox could not run a command: apply-seccomp: denied";
    let machine = started(&l);
    let (machine, effects) = step(machine, ScoutEvent::SandboxBroken { note: note.into() }, &l);
    assert!(effects.is_empty());
    let (machine, _) = step(
        machine,
        ScoutEvent::SandboxBroken {
            note: "a later one".into(),
        },
        &l,
    );
    let (machine, _) = step(machine, ScoutEvent::TurnEnded { usage: None }, &l);
    let (_, effects) = step(machine, ScoutEvent::TurnEnded { usage: None }, &l);
    let reason = format!("the scout ended two turns without a report; {note}");
    assert_eq!(effects, failed(&reason, true));
    // The user's own stop says only that.
    let (machine, _) = step(
        started(&l),
        ScoutEvent::SandboxBroken { note: note.into() },
        &l,
    );
    let (_, effects) = step(machine, ScoutEvent::Stop, &l);
    assert_eq!(effects, failed("stopped by the user", true));
}

#[test]
fn a_tool_result_from_a_broken_sandbox_becomes_a_machine_event() {
    use super::machine::scout_event;
    use crate::headless::SessionEvent;
    use std::collections::HashSet;
    let mut ended = HashSet::new();
    let result = |text: &str, ok: bool| SessionEvent::ToolResult {
        id: "u1".into(),
        text: text.into(),
        ok,
        parent: None,
    };
    let mut event = |e: &SessionEvent| {
        scout_event(
            e,
            Some(1),
            proto::Runtime::Claude,
            &mut ended,
            &super::machine::SCOUT_TEXTS,
        )
    };
    let text = "apply-seccomp: write /proc/self/setgroups: Permission denied";
    assert_eq!(
        event(&result(text, false)),
        Some(ScoutEvent::SandboxBroken {
            note: crate::headless::failure::sandbox_command_failure(text).unwrap()
        })
    );
    assert_eq!(event(&result("all fine", false)), None);
    assert_eq!(event(&result(text, true)), None);
}
