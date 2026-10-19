//! A wall clock stepped back (fix/clock-step-back). The driver measures a wall clock
//! that falls behind the awake clock as `EventKind::SteppedBack`; every anchor and
//! deadline `clock::slept` moves is moved back by the step, so each keeps the awake
//! time it had left. A brief forward step (which the driver cannot tell from a sleep)
//! followed by its correction leaves every wait as it was. Every sequence ends with the
//! liveness check.

use proto::{DocGateAction, DocGateKind, RaceLane, RunState};

use super::design_agents::{agents, brainstorming};
use super::design_fixture::{act, at_spec_gate};
use super::fixture::*;
use super::liveness::assert_alive;
use super::race::{lane, racing};
use super::race_lanes::{HEAD_B, passes};
use super::turns::working;
use crate::run::engine::clock;
use crate::run::engine::{Effect, EventKind};
use crate::run::model::{FailedTurn, StallState};

/// A clock set a year ahead by mistake.
const YEAR: u64 = 365 * 24 * 3_600;
/// A plain step back: longer than every bound below.
const BACK: u64 = 1_000;

fn interrupts(effects: &[Effect]) -> usize {
    effects
        .iter()
        .filter(|e| matches!(e, Effect::Interrupt { .. }))
        .count()
}

/// The wall clock set `YEAR` ahead one second after `fx.now`, then back to true time a
/// second later: the driver steps `Slept` for the first and `SteppedBack` for the
/// second (one second awake each time).
fn ahead_and_back(fx: &mut Fixture) -> Vec<Effect> {
    let t = fx.now;
    let mut effects = fx.send(t + 1 + YEAR, EventKind::Slept { secs: YEAR });
    effects.extend(fx.send(t + 2, EventKind::SteppedBack { secs: YEAR }));
    effects
}

/// Master's #87 review: an open turn with no more events stalls at its original awake
/// bound after a year's forward step and its correction, not a year later.
#[test]
fn a_forward_step_then_its_correction_stalls_at_the_original_bound() {
    let (mut fx, _window) = working();
    let stall_after = fx.run().limits.stall_after_secs;
    let quiet = fx.task("t1").rounds[0].last_event;
    assert!(fx.now + 2 < quiet + stall_after);
    let effects = ahead_and_back(&mut fx);
    assert_eq!(interrupts(&effects), 0, "{effects:#?}");
    assert_eq!(fx.task("t1").rounds[0].last_event, quiet);
    let effects = fx.send(quiet + stall_after, EventKind::Tick);
    assert_eq!(interrupts(&effects), 0, "{effects:#?}");
    let effects = fx.send(quiet + stall_after + 1, EventKind::Tick);
    assert_eq!(interrupts(&effects), 1, "{effects:#?}");
    assert_alive(&fx);
}

/// Every wait of a round that `slept` moves, still running or already come, and the
/// minutes excused, are as they were after the forward step and its correction.
#[test]
fn a_forward_step_then_its_correction_leaves_every_wait_as_it_was() {
    let (mut fx, _window) = working();
    let now = fx.now;
    let round = &mut fx.run_mut().tasks[0].rounds[0];
    round.set_rate_limited(Some(now + 10), now);
    round.delivery_retry_at = Some(now - 1);
    round.count_retry_at = Some(now + 20);
    round.stall = StallState::Interrupted { deadline: now + 30 };
    round.failed_turn = FailedTurn::WaitingContinue {
        at: now + 40,
        rate_limit: false,
    };
    round.excused_secs = 0;
    let before = fx.task("t1").rounds[0].clone();
    ahead_and_back(&mut fx);
    let round = &fx.task("t1").rounds[0];
    assert_eq!(round.rate_limited_until, before.rate_limited_until);
    assert_eq!(round.delivery_retry_at, before.delivery_retry_at);
    assert_eq!(round.count_retry_at, before.count_retry_at);
    assert_eq!(round.stall, before.stall);
    assert_eq!(round.failed_turn, before.failed_turn);
    assert_eq!(round.last_event, before.last_event);
    assert_eq!(round.excused_secs, 0, "{round:#?}");
    assert_alive(&fx);
}

/// A plain step back, no forward step first: an open turn stalls within its original
/// remaining awake time, not `BACK` seconds later.
#[test]
fn a_step_back_stalls_an_open_turn_within_its_remaining_awake_time() {
    let (mut fx, _window) = working();
    let stall_after = fx.run().limits.stall_after_secs;
    let quiet = fx.task("t1").rounds[0].last_event;
    let t = fx.now;
    let effects = fx.send(t - BACK, EventKind::SteppedBack { secs: BACK });
    assert_eq!(interrupts(&effects), 0, "{effects:#?}");
    assert_eq!(fx.task("t1").rounds[0].last_event, quiet - BACK);
    let effects = fx.send(quiet - BACK + stall_after, EventKind::Tick);
    assert_eq!(interrupts(&effects), 0, "{effects:#?}");
    let effects = fx.send(quiet - BACK + stall_after + 1, EventKind::Tick);
    assert_eq!(interrupts(&effects), 1, "{effects:#?}");
    assert_alive(&fx);
}

/// A plain step back moves each wait of a round still to come back by it, clamps one
/// that had come to `now` (it stays come), moves the stall clock's anchor back by it,
/// and takes the step off the minutes excused while the round's clock runs: its awake
/// spend is unchanged.
#[test]
fn a_step_back_moves_every_wait_back_by_it() {
    let (mut fx, _window) = working();
    fx.now += 2 * BACK;
    let now = fx.now;
    let round = &mut fx.run_mut().tasks[0].rounds[0];
    round.set_rate_limited(Some(now + 10), now);
    round.delivery_retry_at = Some(now - 1);
    round.count_retry_at = Some(now + 20);
    round.stall = StallState::Interrupted { deadline: now + 30 };
    round.failed_turn = FailedTurn::WaitingContinue {
        at: now + 40,
        rate_limit: false,
    };
    round.excused_secs = BACK + 5;
    let spend = |fx: &Fixture| {
        crate::run::engine::ladder::round_spend(&fx.task("t1").rounds[0], None, fx.now).secs
    };
    let spent = spend(&fx);
    let quiet = fx.task("t1").rounds[0].last_event;
    fx.send(now - BACK, EventKind::SteppedBack { secs: BACK });
    let round = &fx.task("t1").rounds[0];
    assert_eq!(round.rate_limited_until, Some(now + 10 - BACK));
    assert_eq!(round.rate_limited_since, Some(now), "a record: wall time");
    assert_eq!(
        round.delivery_retry_at,
        Some(now - BACK),
        "come: clamped to now"
    );
    assert_eq!(round.count_retry_at, Some(now + 20 - BACK));
    assert_eq!(
        round.stall,
        StallState::Interrupted {
            deadline: now + 30 - BACK
        }
    );
    let waiting = FailedTurn::WaitingContinue {
        at: now + 40 - BACK,
        rate_limit: false,
    };
    assert_eq!(round.failed_turn, waiting);
    assert_eq!(round.last_event, quiet - BACK);
    assert_eq!(round.excused_secs, 5);
    assert_eq!(spend(&fx), spent);
}

/// A stopped clock's span shrinks with the step, as its round's elapsed time does: the
/// minutes excused stay, as `slept` leaves them.
#[test]
fn a_step_back_leaves_a_stopped_clocks_excused_minutes() {
    let (mut fx, window) = working();
    fx.now += 200;
    super::control::blocked(&mut fx, window, "question", "which table?");
    fx.turn_completed(window);
    assert!(fx.task("t1").clock.stopped.is_some(), "the clock stopped");
    let round = &mut fx.run_mut().tasks[0].rounds[0];
    assert!(round.ended_at.is_none() && !round.ended, "{round:#?}");
    round.excused_secs = 50;
    let now = fx.now - BACK;
    clock::stepped_back(fx.run_mut(), BACK, now);
    assert_eq!(fx.task("t1").rounds[0].excused_secs, 50);
}

/// A stopped racer's exit wait (review M1 of #87) moves back by the step.
#[test]
fn a_step_back_moves_a_stopped_lanes_exit_wait() {
    let (mut fx, _, _) = racing();
    passes(&mut fx, RaceLane::B, HEAD_B);
    let sent = lane(&fx, RaceLane::A).kill_sent_at.expect("lane a stopped");
    let now = fx.now - BACK;
    clock::stepped_back(fx.run_mut(), BACK, now);
    assert_eq!(lane(&fx, RaceLane::A).kill_sent_at, Some(sent - BACK));
}

/// A design phase's budget (milestone 9.6 decision 8) keeps its awake minutes: the run
/// halts 60 awake minutes after the phase began, not `BACK` seconds later.
#[test]
fn a_step_back_keeps_a_design_phases_budget() {
    let mut fx = at_spec_gate(false);
    act(&mut fx, DocGateKind::Spec, DocGateAction::APPROVE).unwrap();
    assert_eq!(fx.run().state, RunState::Planning);
    let start = fx.now;
    fx.send(start - BACK, EventKind::SteppedBack { secs: BACK });
    fx.send(start - BACK + 60 * 60, EventKind::Tick);
    assert_eq!(
        fx.run().state,
        RunState::Planning,
        "{:?}",
        fx.run().halted_reason
    );
    fx.send(start - BACK + 60 * 60 + 1, EventKind::Tick);
    assert_eq!(fx.run().state, RunState::Halted);
}

/// A running design agent's session start (review M4 of #87) moves back by the step.
#[test]
fn a_step_back_moves_a_design_agents_session_start() {
    let mut fx = brainstorming();
    let started = agents(&fx)[0].started.expect("a started brainstormer");
    let now = fx.now - BACK;
    clock::stepped_back(fx.run_mut(), BACK, now);
    assert_eq!(agents(&fx)[0].started, Some(started - BACK));
}

/// A paused run's design phase has the step off its paused time already
/// (`Run::paused_total`), so its start stays, as `slept` leaves it.
#[test]
fn a_step_back_leaves_a_paused_design_phases_start() {
    let mut fx = at_spec_gate(false);
    act(&mut fx, DocGateKind::Spec, DocGateAction::APPROVE).unwrap();
    fx.run_mut().state = RunState::Paused;
    let started = fx.run().orch.design.as_ref().unwrap().phase_started;
    assert!(started.is_some());
    let now = fx.now - BACK;
    clock::stepped_back(fx.run_mut(), BACK, now);
    assert_eq!(
        fx.run().orch.design.as_ref().unwrap().phase_started,
        started
    );
}
