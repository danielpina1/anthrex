//! M8a.15 fix round 3 (ruling T15-R3, refining T15-I3): the task clock runs while a
//! worker turn is open, whatever the task's or the run's state, and the watchdog stays
//! armed on that turn. Excused time covers only spans with no open turn. The
//! re-review's oracle (M-5): over pseudo-random sequences, the charge covers the time
//! the worker actually worked. Every sequence ends with the liveness check.

use proto::{AgentRole, PlanEdit, RunState, TaskState};

use super::control::{blocked, resume, retry};
use super::control_restore::restart;
use super::dispatch::edit;
use super::fixture::*;
use super::holds::answer;
use super::liveness::{assert_alive, assert_clock_sound};
use super::turns::{exited, working};
use crate::run::engine::ladder::total_spend;
use crate::run::engine::{AgentSignal, Effect, EventKind, OpKind, OpResult};

fn total(fx: &Fixture) -> u64 {
    total_spend(fx.task("t1"), fx.now).secs
}

fn interrupts(effects: &[Effect]) -> usize {
    effects
        .iter()
        .filter(|e| matches!(e, Effect::Interrupt { .. }))
        .count()
}

/// Streams the open turn: activity every two minutes for `minutes`; the interrupts
/// the watchdog sent meanwhile.
fn stream(fx: &mut Fixture, window: u32, minutes: u64) -> usize {
    let mut sent = 0;
    for _ in 0..minutes / 2 {
        fx.now += 120;
        sent += interrupts(&fx.signal(window, AgentSignal::Activity));
        sent += interrupts(&fx.tick());
    }
    sent
}

/// A worker that blocks itself and keeps streaming its open turn for an hour is
/// charged for the hour, and its hard budget interrupts the turn; the block stands.
#[test]
fn a_turn_streaming_after_task_blocked_is_charged_and_breaches() {
    let (mut fx, window) = working();
    fx.now += 60;
    blocked(&mut fx, window, "question", "which table?");
    assert!(fx.task("t1").rounds[0].turn_open);
    let sent = stream(&mut fx, window, 60);
    assert!(total(&fx) >= 60 * 60, "charged {}", total(&fx));
    assert!(sent >= 1, "the hard budget interrupts the open turn");
    assert_eq!(fx.task("t1").state, TaskState::Blocked);
    assert_alive(&fx);
}

/// A silent open turn of a blocked task stalls: the watchdog interrupts it, once.
#[test]
fn a_silent_open_turn_of_a_blocked_task_is_interrupted() {
    let (mut fx, window) = working();
    blocked(&mut fx, window, "question", "which table?");
    let stall_after = fx.run().limits.stall_after_secs;
    let effects = fx.send(fx.now + stall_after + 1, EventKind::Tick);
    assert_eq!(interrupts(&effects), 1, "{effects:#?}");
    // Once: the turn has its interrupt until it ends.
    let effects = fx.send(fx.now + stall_after, EventKind::Tick);
    assert_eq!(interrupts(&effects), 0, "{effects:#?}");
    assert_eq!(fx.task("t1").state, TaskState::Blocked);
    assert_alive(&fx);
}

/// A turn open at a pause runs to its end (decision 45) and is charged for it.
#[test]
fn a_pause_with_an_open_streaming_turn_is_charged() {
    let (mut fx, window) = working();
    fx.signal(window, AgentSignal::Activity);
    fx.now += 60;
    edit(&mut fx, vec![PlanEdit::Pause]);
    let at_pause = total(&fx);
    stream(&mut fx, window, 10);
    fx.turn_completed(window);
    let at_turn_end = total(&fx);
    assert!(at_turn_end >= at_pause + 600, "{at_pause} -> {at_turn_end}");
    // The rest of the pause, with no turn open, is excused.
    fx.now += 3_600;
    resume(&mut fx);
    assert!(total(&fx) < at_turn_end + 60, "{}", total(&fx));
    assert_alive(&fx);
}

/// The latest worker round, if it is live.
fn live_worker(fx: &Fixture) -> Option<u32> {
    let t = fx.task("t1");
    let r = t
        .rounds
        .iter()
        .rev()
        .find(|r| r.role == AgentRole::Worker)?;
    if r.ended || r.retiring {
        return None;
    }
    r.window_id
}

fn settle_ops(fx: &mut Fixture) {
    fx.complete_windows();
    let pending: Vec<(u64, OpKind)> = fx
        .run()
        .pending_ops
        .values()
        .map(|p| (p.op, p.kind.clone()))
        .collect();
    for (op, kind) in pending {
        let result = match kind {
            OpKind::DiffSoFar { .. } => OpResult::Diff {
                stat: String::new(),
                patch: String::new(),
            },
            OpKind::ResumeSession { .. } => OpResult::Resumed,
            OpKind::CountCommits { .. } => OpResult::Commits {
                count: 0,
                head: BASE.into(),
            },
            _ => continue,
        };
        if fx.run().pending_ops.contains_key(&op) {
            fx.done(op, result);
        }
    }
}

/// M-5, the re-review's oracle: over 400 pseudo-random sequences of 60 steps, the
/// worker's total charge is at least the seconds it worked, counted independently: a
/// work step's time counts when the worker's session is live and either its task
/// works in a running run or its turn is open (ruling T15-R3). The clock stays sound
/// after every step.
#[test]
fn the_charge_covers_the_time_worked() {
    let mut seed: u64 = 0x1234_5678_9abc_def1;
    for s in 0..400 {
        let (mut fx, _) = working();
        let mut worked: u64 = 0;
        for _ in 0..60 {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            match seed % 14 {
                0..=2 => {
                    let dt = 30 + (seed >> 20) % 900;
                    let open = fx.task("t1").rounds.last().is_some_and(|r| r.turn_open);
                    let working = fx.run().state == RunState::Running
                        && fx.task("t1").state == TaskState::Working;
                    let counts = live_worker(&fx).is_some() && (working || open);
                    fx.now += dt;
                    if counts {
                        worked += dt;
                    }
                    if let Some(w) = live_worker(&fx) {
                        fx.signal(w, AgentSignal::Activity);
                    }
                    fx.tick();
                }
                3 => fx.now += (seed >> 20) % 5_000,
                4 => {
                    if let Some(w) = live_worker(&fx) {
                        fx.turn_completed(w);
                    }
                }
                5 => {
                    if let Some(w) = live_worker(&fx) {
                        exited(&mut fx, w);
                    }
                }
                6 => {
                    edit(&mut fx, vec![PlanEdit::Pause]);
                }
                7 | 8 => {
                    resume(&mut fx);
                }
                9 => {
                    restart(&mut fx, Vec::new());
                }
                10 => {
                    if let Some(w) = live_worker(&fx) {
                        let args = serde_json::json!({"kind": "question", "reason": "q"});
                        fx.tool(w, "task_blocked", args);
                    }
                }
                11 => {
                    edit(&mut fx, vec![answer("a")]);
                }
                12 => {
                    retry(&mut fx, "t1");
                }
                _ => settle_ops(&mut fx),
            }
            assert_clock_sound(fx.run(), fx.now);
            if fx.task("t1").state.is_finished() {
                break;
            }
            let charged = total(&fx);
            assert!(
                charged >= worked,
                "sequence {s}: charged {charged} < worked {worked}"
            );
        }
    }
}

/// Ruling T24-clock: a paused run's silent open turn is interrupted once at least
/// `stall_after_secs` real seconds have passed (one engine second past the whole
/// seconds, since `now` is truncated), and its grace is at least `INTERRUPT_GRACE`.
#[test]
fn a_paused_runs_silent_turn_waits_whole_seconds_to_stall_and_for_its_grace() {
    let (mut fx, _window) = working();
    edit(&mut fx, vec![PlanEdit::Pause]);
    assert_eq!(fx.run().state, RunState::Paused);
    let stall_after = fx.run().limits.stall_after_secs;
    let quiet = fx.task("t1").rounds[0].last_event;
    let effects = fx.send(quiet + stall_after, EventKind::Tick);
    assert_eq!(interrupts(&effects), 0, "{effects:#?}");
    let effects = fx.send(quiet + stall_after + 1, EventKind::Tick);
    assert_eq!(interrupts(&effects), 1, "{effects:#?}");
    assert_eq!(
        fx.task("t1").rounds[0].stall,
        crate::run::model::StallState::Interrupted {
            deadline: fx.now + crate::run::engine::INTERRUPT_GRACE_SECS + 1
        }
    );
    assert_alive(&fx);
}
