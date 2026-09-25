//! M8a.15 fix round 2 (ruling T15-R2): no round is ever excused more time than it
//! has lasted. The re-review's probes P1c (a relaunch inheriting its lost launch's
//! excused time) and P2 (a restore excusing a pause a second time) as regression
//! tests, and a property-style test that drives many pseudo-random sequences of the
//! clock's events and checks the invariant after every step.

use proto::{PlanEdit, TaskState};

use super::control::{resume, retry};
use super::control_restore::restart;
use super::dispatch::edit;
use super::fixture::*;
use super::holds::answer;
use super::liveness::{assert_alive, assert_clock_sound};
use super::turns::{exited, working};
use crate::run::engine::ladder::round_spend;
use crate::run::engine::{AgentSignal, Effect, EventKind, OpKind, OpResult};

fn interrupted(effects: &[Effect]) -> bool {
    effects
        .iter()
        .any(|e| matches!(e, Effect::Interrupt { .. }))
}

fn first_round_spend(fx: &Fixture) -> u64 {
    let t = fx.task("t1");
    round_spend(&t.rounds[0], t.clock.stopped, fx.now).secs
}

/// Minutes of activity, every two minutes, until the worker's session is stopped.
fn minutes_until_stopped(fx: &mut Fixture, window: u32) -> Option<u64> {
    for m in 0..60 {
        fx.signal(window, AgentSignal::Activity);
        fx.send(fx.now + 118, EventKind::Tick);
        let t1 = fx.task("t1");
        // Rung 2 kills the session (its round retires) before the fresh one starts.
        let killed = t1.rounds.iter().any(|r| r.retiring);
        if killed || t1.rounds.len() > 1 || t1.state != TaskState::Working {
            return Some(m * 2);
        }
    }
    None
}

/// N-1 (probe P1c): a launch lost at a restart and relaunched two hours later gives the
/// new session no credit: its 15-minute budget stops it.
#[test]
fn a_relaunched_session_gets_no_credit_from_its_lost_launch() {
    let mut fx = Fixture::new(&plan_with(PROFILE, &[task("t1", "S", "a", "")]));
    fx.ready(true);
    fx.complete_prepares();
    restart(&mut fx, Vec::new());
    fx.now += 2 * 3_600;
    resume(&mut fx);
    let window = fx.complete_windows()[0].1;
    assert_eq!(fx.task("t1").rounds[0].excused_secs, 0);
    let stopped = minutes_until_stopped(&mut fx, window);
    assert!(stopped.is_some_and(|m| m <= 24), "{stopped:?}");
}

/// N-2 (probe P2): a pause excused at its resume is not excused again by a later
/// restore; the charge never goes backwards.
#[test]
fn a_restore_does_not_excuse_a_pause_twice() {
    let (mut fx, window) = working();
    let stall_after = fx.run().limits.stall_after_secs;
    let effects = fx.send(fx.now + stall_after + 1, EventKind::Tick);
    assert!(interrupted(&effects));
    fx.turn_completed(window);
    fx.turn_completed(window);
    exited(&mut fx, window);
    fx.now += 100;
    edit(&mut fx, vec![PlanEdit::Pause]);
    fx.now += 3_600;
    resume(&mut fx);
    let at_resume = first_round_spend(&fx);
    fx.now += 10;
    restart(&mut fx, Vec::new());
    fx.now += 3_600;
    resume(&mut fx);
    assert_clock_sound(fx.run(), fx.now);
    assert!(
        first_round_spend(&fx) >= at_resume,
        "{} < {at_resume}",
        first_round_spend(&fx)
    );
    let stopped = minutes_until_stopped(&mut fx, window);
    assert!(stopped.is_some_and(|m| m <= 24), "{stopped:?}");
}

/// The window of the task's latest worker round.
fn worker_window(fx: &Fixture) -> Option<u32> {
    fx.task("t1").rounds.iter().rev().find_map(|r| r.window_id)
}

/// Answers every op a session's life waits on: launches, diffs, resumes and counts.
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

/// A `task_blocked` whose refusal (the task not working) is part of the sequence.
fn try_block(fx: &mut Fixture) {
    if let Some(w) = worker_window(fx) {
        let args = serde_json::json!({"kind": "question", "reason": "which?"});
        fx.tool(w, "task_blocked", args);
    }
}

/// One step of a sequence, chosen by `pick`.
fn step_once(fx: &mut Fixture, pick: u64) {
    let window = worker_window(fx);
    match pick % 13 {
        0 => fx.now += 37 + pick % 4_000,
        1 => {
            fx.tick();
        }
        2 => {
            if let Some(w) = window {
                fx.signal(w, AgentSignal::Activity);
            }
        }
        3 => {
            if let Some(w) = window {
                fx.turn_completed(w);
            }
        }
        4 => {
            if let Some(w) = window {
                exited(fx, w);
            }
        }
        5 => {
            edit(fx, vec![PlanEdit::Pause]);
        }
        6 => {
            resume(fx);
        }
        7 => {
            restart(fx, Vec::new());
        }
        8 => try_block(fx),
        9 => {
            edit(fx, vec![answer("a")]);
        }
        10 => {
            retry(fx, "t1");
        }
        11 => settle_ops(fx),
        _ => {
            if let Some(w) = window {
                fx.signal(w, AgentSignal::TurnStarted);
            }
        }
    }
}

/// Ruling T15-R2, property-style: over many pseudo-random sequences of time, activity,
/// turn ends, exits, pauses, resumes, restarts, blocks, answers, retries and the ops
/// they wait on, no round is ever excused more than it has lasted.
#[test]
fn no_sequence_excuses_more_than_elapsed() {
    let mut seed: u64 = 0x9e37_79b9_7f4a_7c15;
    for _ in 0..300 {
        let (mut fx, _) = working();
        for _ in 0..40 {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            step_once(&mut fx, seed);
            assert_clock_sound(fx.run(), fx.now);
        }
    }
}

/// The probes' own sequences, checked after every step.
#[test]
fn the_probe_sequences_keep_the_clock_sound() {
    // P1: a lost launch, relaunched after two hours, idle between turns.
    let mut fx = Fixture::new(&plan_with(PROFILE, &[task("t1", "S", "a", "")]));
    fx.ready(true);
    fx.complete_prepares();
    restart(&mut fx, Vec::new());
    assert_clock_sound(fx.run(), fx.now);
    fx.now += 2 * 3_600;
    resume(&mut fx);
    let window = fx.complete_windows()[0].1;
    assert_clock_sound(fx.run(), fx.now);
    fx.turn_completed(window);
    for _ in 0..9 {
        fx.send(fx.now + 600, EventKind::Tick);
        assert_clock_sound(fx.run(), fx.now);
    }
    assert!(fx.task("t1").state != TaskState::Working || first_round_spend(&fx) >= 60);

    // P2b: the double excusal, then chained blocks and answers.
    let (mut fx, _) = working();
    for _ in 0..4 {
        try_block(&mut fx);
        assert_clock_sound(fx.run(), fx.now);
        fx.now += 3 * 3_600;
        edit(&mut fx, vec![answer("a")]);
        assert_clock_sound(fx.run(), fx.now);
        edit(&mut fx, vec![PlanEdit::Pause]);
        fx.now += 600;
        restart(&mut fx, Vec::new());
        assert_clock_sound(fx.run(), fx.now);
        fx.now += 600;
        resume(&mut fx);
        settle_ops(&mut fx);
        assert_clock_sound(fx.run(), fx.now);
    }
    assert_alive(&fx);
}

/// N-2 for a live session between turns whose last event lies inside an excused
/// pause: a nudged round (a resume re-arms only a watching one) keeps its old last
/// event through the resume, so a restore right after must stop the clock at the
/// resume, not at that event, or the pause is excused twice.
#[test]
fn a_restore_right_after_a_resume_stops_the_clock_at_the_resume() {
    let (mut fx, window) = working();
    let stall_after = fx.run().limits.stall_after_secs;
    let effects = fx.send(fx.now + stall_after + 1, EventKind::Tick);
    assert!(interrupted(&effects));
    fx.turn_completed(window);
    fx.turn_completed(window);
    let round = &fx.task("t1").rounds[0];
    assert!(!round.ended && !round.turn_open, "{round:#?}");
    fx.now += 100;
    edit(&mut fx, vec![PlanEdit::Pause]);
    fx.now += 3_600;
    resume(&mut fx);
    let at_resume = first_round_spend(&fx);
    fx.now += 5;
    restart(&mut fx, Vec::new());
    fx.now += 3_600;
    resume(&mut fx);
    assert!(
        first_round_spend(&fx) >= at_resume,
        "{} < {at_resume}",
        first_round_spend(&fx)
    );
    assert_alive(&fx);
}
