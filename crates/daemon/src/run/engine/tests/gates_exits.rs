//! M8a.13 fix round 2 (ruling T13-R2): a reviewer given up while its process lives
//! holds its reader slot until that process exits, and an exit is matched against the
//! round's process id, so a late exit of an earlier process is not a death of the
//! current one. Each test ends with the liveness check.

use proto::{Runtime, TaskState};

use super::fixture::*;
use super::gates::{CHECK_MODE, only_op, working_on};
use super::gates_fixes::ack;
use super::gates_review::reviewed;
use super::turns_fixes::assert_alive;
use crate::headless::FailureKind;
use crate::run::engine::{AgentSignal, Effect, EventKind, TurnOutcome};

fn started(fx: &mut Fixture, window: u32, pid: u32) {
    fx.signal(window, AgentSignal::ProcessStarted { pid });
}

fn exit(fx: &mut Fixture, window: u32, pid: u32) -> Vec<Effect> {
    fx.signal(
        window,
        AgentSignal::ProcessExited {
            code: Some(0),
            killed_by_engine: false,
            pid,
        },
    )
}

/// Probe r1 inverted (N1): Codex emits `turn.completed` before its process exits. A
/// reviewer given up in that window is killed, keeps its slot, and the next round's
/// `PrepareReview` waits for the exit, which ends the round (natural or killed).
#[test]
fn a_codex_reviewer_given_up_before_its_exit_holds_its_slot() {
    let (mut fx, _, rwindow) = reviewed(PROFILE, "");
    assert_eq!(
        fx.task("t1").rounds.last().unwrap().route.runtime,
        Runtime::Codex
    );
    started(&mut fx, rwindow, 41);
    fx.signal(rwindow, AgentSignal::TurnStarted);
    fx.turn_completed(rwindow); // the nudge, delivered: the next process starts
    ack(&mut fx);
    started(&mut fx, rwindow, 42);
    exit(&mut fx, rwindow, 41); // the previous process's late exit
    fx.signal(rwindow, AgentSignal::TurnStarted);
    let effects = fx.turn_completed(rwindow); // verdict-less: given up
    assert!(effects.contains(&Effect::KillWindow { window_id: rwindow }));
    assert!(ops_in(&effects, "PrepareReview").is_empty(), "{effects:#?}");
    let round = fx.task("t1").rounds.last().unwrap().clone();
    assert!(round.retiring && !round.ended, "{round:#?}");
    assert!(ops_in(&fx.tick(), "PrepareReview").is_empty());
    assert_alive(&fx);
    let effects = exit(&mut fx, rwindow, 42);
    assert!(fx.task("t1").rounds.last().unwrap().ended);
    only_op(&effects, "PrepareReview");
    assert_eq!(fx.task("t1").state, TaskState::Review);
    assert_alive(&fx);
}

/// N1: a Codex reviewer between processes (its last one exited) has nothing to kill,
/// so stopping it ends its round at once.
#[test]
fn a_codex_reviewer_between_processes_ends_at_once_when_stopped() {
    let (mut fx, _, rwindow) = reviewed(PROFILE, "");
    started(&mut fx, rwindow, 41);
    fx.signal(rwindow, AgentSignal::TurnStarted);
    // A rate-limited turn: nothing is delivered until the wait is over.
    fx.turn_ended(
        rwindow,
        TurnOutcome::Failed {
            error: "rate limit reached".into(),
            kind: FailureKind::RateLimit,
        },
    );
    exit(&mut fx, rwindow, 41);
    assert_eq!(fx.task("t1").rounds.last().unwrap().pid, None);
    let reply = fx.reply();
    let effects = fx.next(EventKind::Override {
        reply,
        run_id: RUN_ID.into(),
        task_id: "t1".into(),
        reason: "r".into(),
    });
    assert!(
        !effects
            .iter()
            .any(|e| matches!(e, Effect::KillWindow { .. })),
        "{effects:#?}"
    );
    let round = fx.task("t1").rounds.last().unwrap().clone();
    assert!(round.retiring && round.ended, "{round:#?}");
    assert_eq!(fx.task("t1").state, TaskState::MergeQueue);
    assert_alive(&fx);
}

/// Ruling T13-R2: a worker's exit from another process than the round's is dropped;
/// its own process's exit mid-turn is still a death.
#[test]
fn a_workers_exit_from_another_process_is_dropped() {
    let (mut fx, window) = working_on(PROFILE, CHECK_MODE);
    started(&mut fx, window, 41);
    fx.signal(window, AgentSignal::TurnStarted);
    let effects = exit(&mut fx, window, 40);
    assert!(ops_in(&effects, "ResumeSession").is_empty(), "{effects:#?}");
    let round = fx.task("t1").rounds.last().unwrap().clone();
    assert_eq!((round.deaths, round.ended, round.pid), (0, false, Some(41)));
    assert_alive(&fx);
    let effects = exit(&mut fx, window, 41);
    assert_eq!(ops_in(&effects, "ResumeSession").len(), 1, "{effects:#?}");
    assert_eq!(fx.task("t1").rounds.last().unwrap().deaths, 1);
    assert_alive(&fx);
}

/// Ruling T13-R2: the same for a reviewer — a late exit of its previous Codex process,
/// arriving while the next one runs its turn, is not a death.
#[test]
fn a_reviewers_exit_from_another_process_is_dropped() {
    let (mut fx, _, rwindow) = reviewed(PROFILE, "");
    started(&mut fx, rwindow, 41);
    fx.signal(rwindow, AgentSignal::TurnStarted);
    fx.turn_completed(rwindow); // the nudge
    ack(&mut fx);
    started(&mut fx, rwindow, 42);
    fx.signal(rwindow, AgentSignal::TurnStarted);
    let effects = exit(&mut fx, rwindow, 41);
    assert!(ops_in(&effects, "ResumeSession").is_empty(), "{effects:#?}");
    let round = fx.task("t1").rounds.last().unwrap().clone();
    assert_eq!((round.deaths, round.ended, round.pid), (0, false, Some(42)));
    assert_eq!(fx.task("t1").state, TaskState::Review);
    assert_alive(&fx);
}
