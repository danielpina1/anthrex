//! 2026-10-06: a session process that died at startup says why. The driver's
//! `StartupFailed` (its last stderr lines) goes into the reason the exit's rung records,
//! so `run status` shows Claude's own words instead of only "its process exited".

use super::fixture::*;
use super::turns::working_on;
use crate::run::engine::AgentSignal;

const WHY: &str = "claude exited at startup (code 1): no config here";

fn dies_at_startup(fx: &mut Fixture, window: u32, pid: u32) {
    fx.signal(window, AgentSignal::ProcessStarted { pid });
    fx.signal(
        window,
        AgentSignal::StartupFailed {
            reason: WHY.to_string(),
        },
    );
    fx.signal(
        window,
        AgentSignal::ProcessExited {
            code: Some(1),
            killed_by_engine: false,
            pid,
        },
    );
}

#[test]
fn a_worker_that_dies_twice_at_startup_stalls_with_its_stderr() {
    let (mut fx, window) = working_on("");
    dies_at_startup(&mut fx, window, 41);
    let resumed = fx.task("t1").history.last().unwrap().text.clone();
    assert!(resumed.contains(WHY), "{resumed}");
    dies_at_startup(&mut fx, window, 42);
    let task = fx.task("t1");
    assert_eq!(task.stalls, 1, "{:?}", task.history);
    assert!(
        task.history
            .iter()
            .any(|e| e.text == format!("stalled: its process exited twice in one round: {WHY}")),
        "{:?}",
        task.history
    );
}

/// A process that said nothing on stderr keeps the plain reason; a later process's
/// exit does not inherit an earlier one's startup failure.
#[test]
fn a_startup_failure_belongs_to_its_own_process() {
    let (mut fx, window) = working_on("");
    dies_at_startup(&mut fx, window, 41);
    fx.signal(window, AgentSignal::ProcessStarted { pid: 42 });
    fx.signal(
        window,
        AgentSignal::ProcessExited {
            code: Some(1),
            killed_by_engine: false,
            pid: 42,
        },
    );
    let task = fx.task("t1");
    assert!(
        task.history
            .iter()
            .any(|e| e.text == "stalled: its process exited twice in one round"),
        "{:?}",
        task.history
    );
}
