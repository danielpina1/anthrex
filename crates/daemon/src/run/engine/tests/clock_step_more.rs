//! Review round 1 of the clock-step fix (M2, M3, M4). A step back also moves back a
//! task's and a racing lane's clock (`TaskClock.stopped`, `restarted`), so a restore
//! after the correction charges no downtime; it moves back the orchestrator's wait and
//! a runtime's writer-cap anchors, so neither waits a forward step's length; and the
//! edges of `excused_secs` and of unequal steps hold.

use proto::{RaceLane, Runtime};

use super::fixture::*;
use super::liveness::assert_alive;
use super::race::racing;
use super::turns::working;
use crate::run::engine::clock::{self, TaskClock};
use crate::run::engine::ladder::round_spend;
use crate::run::engine::{Effect, EventKind};
use crate::run::model::RuntimeConcurrency;

/// A clock set a year ahead by mistake.
const YEAR: u64 = 365 * 24 * 3_600;
/// A plain step back.
const BACK: u64 = 1_000;

fn interrupts(effects: &[Effect]) -> usize {
    effects
        .iter()
        .filter(|e| matches!(e, Effect::Interrupt { .. }))
        .count()
}

/// M2, the review's scenario: the task's clock restarted while the wall clock was a
/// year ahead, then the clock was corrected, then the daemon restarted. The restore
/// stops the clock at its last restart at the latest; once moved back, that is no
/// later than `now`, so the downtime that follows is excused, not charged.
#[test]
fn a_restart_stamped_ahead_is_moved_back_so_a_restore_charges_no_downtime() {
    let (mut fx, _window) = working();
    let t = fx.now;
    fx.task_mut("t1").clock = TaskClock {
        stopped: None,
        restarted: t + YEAR,
    };
    let now = t + 2;
    clock::stepped_back(fx.run_mut(), YEAR, now);
    assert_eq!(fx.task("t1").clock.restarted, t);
    clock::stop_at_restore(fx.task_mut("t1"));
    let stopped = fx.task("t1").clock.stopped.expect("stopped at the restore");
    assert!(stopped <= now, "stopped at {stopped}, after {now}");
    // An hour of downtime later, the round has spent no more than it had.
    let round = &fx.task("t1").rounds[0];
    let spent = round_spend(round, Some(stopped), now).secs;
    assert_eq!(round_spend(round, Some(stopped), now + 3_600).secs, spent);
}

/// M2: a clock stopped while the wall clock was ahead keeps the span of its stop, and
/// its round's spend is what it was before the step.
#[test]
fn a_stop_stamped_ahead_is_moved_back_and_keeps_its_span() {
    let (mut fx, _window) = working();
    fx.now += 3 * BACK;
    let t = fx.now;
    let spent = round_spend(&fx.task("t1").rounds[0], Some(t - 100), t).secs;
    fx.task_mut("t1").clock = TaskClock {
        stopped: Some(t + YEAR - 100),
        restarted: 0,
    };
    fx.task_mut("t1").rounds[0].excused_secs = YEAR;
    clock::stepped_back(fx.run_mut(), YEAR, t);
    let task = fx.task("t1");
    assert_eq!(task.clock.stopped, Some(t - 100));
    assert_eq!(task.rounds[0].excused_secs, 0);
    assert_eq!(
        round_spend(&task.rounds[0], task.clock.stopped, t).secs,
        spent
    );
}

/// M2: a stopped clock in a plain step back: its stop moves back with it and its
/// round's excused minutes lose the step, so its spend is unchanged.
#[test]
fn a_step_back_keeps_a_stopped_clocks_spend() {
    let (mut fx, window) = working();
    fx.now += 3 * BACK;
    super::control::blocked(&mut fx, window, "question", "which table?");
    fx.turn_completed(window);
    let since = fx.task("t1").clock.stopped.expect("the clock stopped");
    fx.now += 10;
    let round = &mut fx.run_mut().tasks[0].rounds[0];
    assert!(round.ended_at.is_none() && !round.ended, "{round:#?}");
    round.excused_secs = BACK + 50;
    let spend = |fx: &Fixture, now| {
        let task = fx.task("t1");
        round_spend(&task.rounds[0], task.clock.stopped, now).secs
    };
    let before = spend(&fx, fx.now);
    let now = fx.now - BACK;
    clock::stepped_back(fx.run_mut(), BACK, now);
    assert_eq!(fx.task("t1").clock.stopped, Some(since - BACK));
    assert_eq!(fx.task("t1").rounds[0].excused_secs, 50);
    assert_eq!(spend(&fx, now), before);
}

/// M2: a racing lane's clock moves back too.
#[test]
fn a_step_back_moves_a_lanes_clock() {
    let (mut fx, _, _) = racing();
    let t = fx.now;
    let race = fx.task_mut("t1").race.as_mut().expect("a race");
    let a = (race.lanes.iter_mut())
        .find(|l| l.lane == RaceLane::A)
        .unwrap();
    a.gates.clock = TaskClock {
        stopped: Some(t + YEAR - 5),
        restarted: t + YEAR - 10,
    };
    clock::stepped_back(fx.run_mut(), YEAR, t);
    let race = fx.task("t1").race.as_ref().unwrap();
    let a = race.lanes.iter().find(|l| l.lane == RaceLane::A).unwrap();
    assert_eq!(
        a.gates.clock,
        TaskClock {
            stopped: Some(t - 5),
            restarted: t - 10,
        }
    );
}

/// M3: the orchestrator's wait (`orch_stall`) and a runtime's writer-cap anchors
/// (`concurrency`) stamped while the clock was ahead move back with the step, so the
/// stall notice and the cap's recovery do not wait a year.
#[test]
fn a_step_back_moves_the_orchestrators_wait_and_the_writer_caps() {
    let (mut fx, _window) = working();
    let t = fx.now;
    let run = fx.run_mut();
    run.orch.waiting_since = Some(t + YEAR - 50);
    let mut c = RuntimeConcurrency::new(2);
    (c.cap, c.last_rate_limit_at, c.last_change_at) = (1, Some(t + YEAR - 60), Some(t + YEAR - 70));
    run.concurrency
        .insert(Runtime::Claude.label().to_string(), c);
    clock::stepped_back(fx.run_mut(), YEAR, t);
    let run = fx.run();
    assert_eq!(run.orch.waiting_since, Some(t - 50));
    let c = &run.concurrency[Runtime::Claude.label()];
    assert_eq!(c.last_rate_limit_at, Some(t - 60));
    assert_eq!(c.last_change_at, Some(t - 70));
}

/// M4: excused minutes smaller than the step floor at none: the round's spend drops by
/// the rest (an undercount, so a budget is breached late, never early).
#[test]
fn excused_minutes_smaller_than_the_step_floor_at_none() {
    let (mut fx, _window) = working();
    fx.now += 3 * BACK;
    let t = fx.now;
    fx.task_mut("t1").rounds[0].excused_secs = 5;
    let before = round_spend(&fx.task("t1").rounds[0], None, t).secs;
    clock::stepped_back(fx.run_mut(), BACK, t - BACK);
    let round = &fx.task("t1").rounds[0];
    assert_eq!(round.excused_secs, 0);
    assert_eq!(round_spend(round, None, t - BACK).secs, before - (BACK - 5));
}

/// M4: a correction short of the forward step (the clock left 100 s ahead) leaves the
/// open turn's stall 100 s later than before; one past it, 100 s earlier.
#[test]
fn unequal_steps_net_their_difference() {
    for (back, net) in [(YEAR - 100, 100_i64), (YEAR + 100, -100)] {
        let (mut fx, _window) = working();
        let stall_after = fx.run().limits.stall_after_secs;
        let quiet = fx.task("t1").rounds[0].last_event;
        let t = fx.now;
        fx.send(t + 1 + YEAR, EventKind::Slept { secs: YEAR });
        let wall = (t + 2 + YEAR).checked_sub(back).unwrap();
        fx.send(wall, EventKind::SteppedBack { secs: back });
        let moved = quiet.checked_add_signed(net).unwrap();
        assert_eq!(fx.task("t1").rounds[0].last_event, moved, "back {back}");
        let effects = fx.send(moved + stall_after, EventKind::Tick);
        assert_eq!(interrupts(&effects), 0, "{effects:#?}");
        let effects = fx.send(moved + stall_after + 1, EventKind::Tick);
        assert_eq!(interrupts(&effects), 1, "{effects:#?}");
        assert_alive(&fx);
    }
}
