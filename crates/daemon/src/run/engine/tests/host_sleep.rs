//! A host sleep (the wall clock jumps, no awake time passes) is no session time: it
//! neither stalls an open turn, nor lets the outbox deliver into it (decision 42), nor
//! charges the minutes budget, nor ends a wait early. The driver measures the sleep
//! (`driver::awake`) and steps `EventKind::Slept`; the stall clock, the waits and the
//! budget then count awake time only. Every sequence ends with the liveness check.

use proto::TaskState;

use super::control::blocked;
use super::fixture::*;
use super::liveness::assert_alive;
use super::turns::{queue, working, working_on};
use crate::run::engine::ladder::round_spend;
use crate::run::engine::{Effect, EventKind, INTERRUPT_GRACE_SECS};
use crate::run::model::StallState;

/// A long host sleep, longer than every bound below.
const SLEPT: u64 = 1_000;

fn interrupts(effects: &[Effect]) -> usize {
    effects
        .iter()
        .filter(|e| matches!(e, Effect::Interrupt { .. }))
        .count()
}

fn delivers(effects: &[Effect]) -> usize {
    effects
        .iter()
        .filter(|e| matches!(e, Effect::Deliver { .. }))
        .count()
}

/// The host sleeps `secs`: the wall clock jumps and the driver steps `Slept` at the new
/// wall time, before the event that noticed it.
fn sleep(fx: &mut Fixture, secs: u64) -> Vec<Effect> {
    let now = fx.now + secs;
    fx.send(now, EventKind::Slept { secs })
}

/// The plan's test: a blocked task's open turn with a message waiting for its end. A
/// sleep longer than the stall bound interrupts nothing, the turn stays open and the
/// message stays queued; once `stall_after_secs` of awake time pass, the stall fires as
/// before.
#[test]
fn a_sleep_neither_stalls_a_blocked_tasks_open_turn_nor_delivers_into_it() {
    let (mut fx, window) = working();
    blocked(&mut fx, window, "question", "which table?");
    queue(&mut fx, "the orchestrator's answer");
    let stall_after = fx.run().limits.stall_after_secs;
    assert!(SLEPT > stall_after + INTERRUPT_GRACE_SECS);
    let quiet = fx.task("t1").rounds[0].last_event;
    let mut effects = sleep(&mut fx, SLEPT);
    effects.extend(fx.tick());
    assert_eq!(interrupts(&effects), 0, "{effects:#?}");
    assert_eq!(delivers(&effects), 0, "{effects:#?}");
    let round = &fx.task("t1").rounds[0];
    assert!(round.turn_open && !round.interrupted, "{round:#?}");
    assert!(fx.run().outbox.iter().all(|m| m.delivered_at.is_none()));
    assert_eq!(fx.task("t1").state, TaskState::Blocked);
    // Awake time: the bound, measured from the last event, less the sleep.
    let effects = fx.send(quiet + SLEPT + stall_after, EventKind::Tick);
    assert_eq!(interrupts(&effects), 0, "{effects:#?}");
    let effects = fx.send(quiet + SLEPT + stall_after + 1, EventKind::Tick);
    assert_eq!(interrupts(&effects), 1, "{effects:#?}");
    assert_alive(&fx);
}

/// A working task's open turn (`signals::watch`): the sleep is no silence, and the
/// first stage fires after `stall_after_secs` awake; a sleep inside the interrupt's
/// grace does not end the grace either.
#[test]
fn a_sleep_neither_stalls_a_working_turn_nor_ends_its_grace() {
    let (mut fx, _window) = working();
    let stall_after = fx.run().limits.stall_after_secs;
    let quiet = fx.task("t1").rounds[0].last_event;
    let mut effects = sleep(&mut fx, SLEPT);
    effects.extend(fx.tick());
    assert_eq!(interrupts(&effects), 0, "{effects:#?}");
    assert_eq!(fx.task("t1").rounds[0].stall, StallState::Watching);
    let effects = fx.send(quiet + SLEPT + stall_after + 1, EventKind::Tick);
    assert_eq!(interrupts(&effects), 1, "{effects:#?}");
    let StallState::Interrupted { deadline } = fx.task("t1").rounds[0].stall else {
        panic!("{:#?}", fx.task("t1").rounds[0]);
    };
    // A sleep inside the grace: the grace still has its awake seconds to run.
    let before = fx.task("t1").rung;
    sleep(&mut fx, SLEPT);
    fx.tick();
    assert_eq!(fx.task("t1").rung, before, "{:?}", fx.task("t1").history);
    assert!(matches!(
        fx.task("t1").rounds[0].stall,
        StallState::Interrupted { deadline: d } if d == deadline + SLEPT
    ));
    assert_eq!(fx.task("t1").state, TaskState::Working);
    assert_alive(&fx);
}

/// Decision 40's minutes budget counts awake time: a sleep three times the budget
/// charges nothing, so neither the wrap-up nor the hard limit fires.
#[test]
fn a_sleep_is_not_charged_to_the_minutes_budget() {
    let (mut fx, window) = working_on("[task.budget]\ntool_calls = 1000\nminutes = 5");
    let before = round_spend(&fx.task("t1").rounds[0], None, fx.now).secs;
    let mut effects = sleep(&mut fx, 3 * 5 * 60);
    effects.extend(fx.signal(window, crate::run::engine::AgentSignal::Activity));
    effects.extend(fx.tick());
    let spent = round_spend(&fx.task("t1").rounds[0], None, fx.now).secs;
    assert!(spent <= before + 2, "charged {spent} (was {before})");
    assert_eq!(interrupts(&effects), 0, "{effects:#?}");
    assert!(fx.run().outbox.is_empty(), "{:?}", fx.run().outbox);
    assert!(!fx.task("t1").rounds[0].wrap_up_sent);
    assert_alive(&fx);
}

/// A wait counts awake time: a deadline that had not come at the sleep's start (a rate
/// limit's end) moves by it; one that had come (a failed delivery's retry) stays.
#[test]
fn a_sleep_moves_only_the_waits_still_running() {
    let (mut fx, _window) = working();
    let now = fx.now;
    let round = &mut fx.run_mut().tasks[0].rounds[0];
    round.set_rate_limited(Some(now + 10), now);
    round.delivery_retry_at = Some(now - 1);
    sleep(&mut fx, SLEPT);
    let round = &fx.task("t1").rounds[0];
    assert_eq!(round.rate_limited_until, Some(now + 10 + SLEPT));
    assert_eq!(round.rate_limited_since, Some(now), "a record: wall time");
    assert_eq!(round.delivery_retry_at, Some(now - 1));
    assert!(round.last_event <= fx.now);
    assert_alive(&fx);
}
