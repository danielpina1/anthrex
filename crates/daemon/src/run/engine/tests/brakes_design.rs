//! Milestone 9.16 task M9.16.5's design half (decision 11, ruling C6; task 5's review
//! I1), added in M9.16.6: a design run's brainstormers (`design_agents::start_next`)
//! and its document reviewer (`design_reviewer::start_next`) wait on a braked runtime
//! and under the hold; a refused design agent never spins the reader loop, and the next
//! candidate (the other brainstormer, a run scout) is still tried.

use std::sync::mpsc;
use std::time::Duration;

use proto::Runtime;
use proto::brakes::{DispatchHold, QuotaWindow};
use proto::models::{ModelRef, Role, RoleChoice};
use serde_json::Value;

use super::brakes::{brake_on, reading, trip_claude};
use super::design_agents::launches;
use super::design_fixture::*;
use super::design_review_fixture::*;
use super::fixture::*;
use super::orch::answer;
use super::run_scouts::{scout, scout_ended};
use crate::run::design::state::DesignAgentState;
use crate::run::engine::{EventKind, ScoutEnd};
use crate::run::orch::RunScoutState;

/// The hang guard: one pure reducer step takes milliseconds; a loop that spins never
/// returns (docs/timing-budgets.md, as `brakes_readers.rs`'s `STEP_WAIT`).
const STEP_WAIT: Duration = Duration::from_secs(10);

/// The labels of every design agent launched so far, oldest first.
fn launched(fx: &Fixture) -> Vec<String> {
    (launches(fx).into_iter())
        .map(|(_, s)| s.kind.label())
        .collect()
}

fn brainstormer_states(fx: &Fixture) -> Vec<(String, DesignAgentState)> {
    let design = fx.run().orch.design.as_ref().expect("a design run");
    (design.brainstormers.iter())
        .map(|a| (a.label.clone(), a.state.clone()))
        .collect()
}

fn hold(fx: &mut Fixture, on: bool) {
    let reply = fx.reply();
    let hold = on.then(|| DispatchHold {
        since: 1_900,
        by: "user".into(),
    });
    fx.next(EventKind::Hold { reply, hold });
}

/// Codex's 7-day window at 96 %, resetting in ten minutes (inside the specifying
/// phase's budget): a trip.
fn trip_codex(fx: &mut Fixture) -> u64 {
    let until = fx.now + 600;
    fx.next(brake_on());
    fx.next(reading(Runtime::Codex, QuotaWindow::SevenDay, 960, until));
    until
}

/// A design run in specifying whose document reviewer is the `reviewer` row's Codex
/// default with no fallback, so a refused reviewer can only wait.
fn specifying_codex_reviewer() -> Fixture {
    let mut fx = specifying();
    let models = (fx.run_mut().limits.models.as_mut()).expect("frozen at start");
    let row = RoleChoice {
        model: ModelRef::parse("codex:default").expect("a model"),
        effort: Some("high".to_string()),
        fallback: None,
    };
    models.rows.insert(Role::Reviewer, row);
    fx
}

/// `steps` on `fx` on a thread of its own, within [`STEP_WAIT`] (the hang guard): a
/// reader loop that spins never returns, and the test fails instead of hanging.
fn guarded(fx: Fixture, steps: impl FnOnce(&mut Fixture) + Send + 'static) -> Fixture {
    let (send, recv) = mpsc::channel();
    let mut fx = fx;
    std::thread::spawn(move || {
        steps(&mut fx);
        let _ = send.send(fx);
    });
    recv.recv_timeout(STEP_WAIT)
        .expect("the step returned: the reader loop does not spin")
}

fn reviewer_state(fx: &Fixture) -> Option<DesignAgentState> {
    let design = fx.run().orch.design.as_ref()?;
    design.reviewer.as_ref().map(|a| a.state.clone())
}

#[test]
fn a_braked_brainstormer_waits_and_the_other_one_starts() {
    let mut fx = design_launched(false);
    let until = trip_claude(&mut fx);
    // Task 6's review M2: a refused brainstormer that re-picked itself would spin.
    let mut fx = guarded(fx, |fx| {
        start_brainstorm(fx);
    });
    // The Claude brainstormer is first in line; refused, the Codex one is tried.
    assert_eq!(launched(&fx), ["codex"]);
    assert_eq!(
        brainstormer_states(&fx),
        [
            ("claude".into(), DesignAgentState::Queued),
            ("codex".into(), DesignAgentState::Running),
        ]
    );
    // A lower reading, newer than the trip, releases it.
    fx.next(reading(Runtime::Claude, QuotaWindow::SevenDay, 500, until));
    assert_eq!(launched(&fx), ["codex", "claude"]);
}

#[test]
fn the_hold_keeps_both_brainstormers_queued() {
    let mut fx = design_launched(false);
    hold(&mut fx, true);
    // Task 6's review M2: both refused; a loop that re-picked one would never return.
    let mut fx = guarded(fx, |fx| {
        start_brainstorm(fx);
        fx.tick();
    });
    assert!(launched(&fx).is_empty(), "{:?}", launched(&fx));
    assert_eq!(
        brainstormer_states(&fx),
        [
            ("claude".into(), DesignAgentState::Queued),
            ("codex".into(), DesignAgentState::Queued),
        ]
    );
    hold(&mut fx, false);
    assert_eq!(launched(&fx), ["claude", "codex"]);
}

#[test]
fn a_braked_document_reviewer_waits_without_spinning_and_a_scout_still_starts() {
    let mut fx = specifying_codex_reviewer();
    let until = trip_codex(&mut fx);
    let before = launched(&fx).len();
    let (send, recv) = mpsc::channel();
    // The queued reviewer is refused inside the tool handlers' own scheduler passes;
    // a loop that re-picks it never returns.
    std::thread::spawn(move || {
        let reply = outcome(&submit_spec(&mut fx, false, Value::Null));
        let effects = scout(&mut fx, "s1");
        let _ = send.send((fx, reply, effects));
    });
    let (mut fx, reply, effects) = recv
        .recv_timeout(STEP_WAIT)
        .expect("the step returned: the reader loop does not spin");
    assert_eq!(reply.unwrap()["awaiting_review"], Value::Bool(true));
    assert_eq!(launched(&fx).len(), before, "no reviewer launch");
    assert_eq!(reviewer_state(&fx), Some(DesignAgentState::Queued));
    // The run scout (the `research` row: Claude) behind it starts.
    let (ok, value) = answer(&effects);
    assert!(ok, "{value}");
    assert_eq!(value["state"], "starting");
    assert_eq!(fx.run().orch.run_scouts[0].state, RunScoutState::Running);
    // The window resets and the scout's reader slot frees: the reviewer starts.
    fx.send(until, EventKind::Tick);
    let id = fx.run().orch.run_scouts[0].id.clone();
    scout_ended(&mut fx, &id, ScoutEnd::Reported);
    assert_eq!(launched(&fx).last().map(String::as_str), Some("spec-r1"));
    assert_eq!(reviewer_state(&fx), Some(DesignAgentState::Running));
}

#[test]
fn the_hold_keeps_the_document_reviewer_queued() {
    let mut fx = specifying_codex_reviewer();
    hold(&mut fx, true);
    let before = launched(&fx).len();
    let (send, recv) = mpsc::channel();
    std::thread::spawn(move || {
        outcome(&submit_spec(&mut fx, false, Value::Null)).unwrap();
        fx.tick();
        let _ = send.send(fx);
    });
    let mut fx = recv
        .recv_timeout(STEP_WAIT)
        .expect("the step returned: the reader loop does not spin");
    assert_eq!(launched(&fx).len(), before);
    assert_eq!(reviewer_state(&fx), Some(DesignAgentState::Queued));
    hold(&mut fx, false);
    assert_eq!(launched(&fx).last().map(String::as_str), Some("spec-r1"));
    assert_eq!(reviewer_state(&fx), Some(DesignAgentState::Running));
}
