//! M8a.22's reducer carries: the exit of the process whose turn already closed is
//! normal (ruling T13-P1), an unprompted turn's usage is spend and not a turn end, a
//! resume failure for a round the engine already stopped is ignored, and a run's
//! session nonce reaches its session uuids. Each test ends with the liveness check.

use proto::{Runtime, TaskState, TokenUsage};

use super::fixture::*;
use super::gates::{CHECK_MODE, working_on};
use super::gates_fixes::ack;
use super::gates_review::reviewed;
use super::turns_fixes::assert_alive;
use crate::run::engine::{AgentSignal, Effect, EventKind, OpKind, OpResult};
use crate::run::role_launch::{session_uuid, session_uuid_of};

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

/// Ruling T13-P1: a Codex send waits for the previous process's exit, so that exit
/// reaches the engine after it emitted the next `Deliver` (the turn is open again) and
/// before the next `ProcessStarted`. It is the normal end of the turn that closed in
/// that process, never a death.
#[test]
fn the_exit_of_the_process_whose_turn_closed_is_normal() {
    let (mut fx, _, rwindow) = reviewed(PROFILE, "");
    assert_eq!(
        fx.task("t1").rounds.last().unwrap().route.runtime,
        Runtime::Codex
    );
    fx.signal(rwindow, AgentSignal::ProcessStarted { pid: 41 });
    fx.signal(rwindow, AgentSignal::TurnStarted);
    fx.turn_completed(rwindow); // verdict-less: the nudge is delivered at once
    ack(&mut fx);
    assert!(fx.task("t1").rounds.last().unwrap().turn_open);
    let effects = exit(&mut fx, rwindow, 41);
    assert!(ops_in(&effects, "ResumeSession").is_empty(), "{effects:#?}");
    let round = fx.task("t1").rounds.last().unwrap().clone();
    assert_eq!((round.deaths, round.ended), (0, false), "{round:#?}");
    assert!(round.turn_open);
    fx.signal(rwindow, AgentSignal::ProcessStarted { pid: 42 });
    assert_eq!(fx.task("t1").rounds.last().unwrap().pid, Some(42));
    assert_eq!(fx.task("t1").state, TaskState::Review);
    assert_alive(&fx);
    // The new process's own exit mid-turn is still a death.
    let effects = exit(&mut fx, rwindow, 42);
    assert_eq!(
        fx.task("t1").rounds.last().unwrap().deaths,
        1,
        "{effects:#?}"
    );
    assert_alive(&fx);
}

/// Ruling T7-N1 (M8a.22's translation): a background turn's `TurnEnded` arrives as
/// `Spend`: its usage counts, and the delivered turn stays open.
#[test]
fn an_unprompted_turns_usage_is_spend_not_a_turn_end() {
    let (mut fx, window) = working_on(PROFILE, CHECK_MODE);
    fx.signal(window, AgentSignal::TurnStarted);
    let usage = TokenUsage {
        input: 11,
        output: 23,
        cache_read: 37,
        cache_write: 41,
    };
    let effects = fx.signal(window, AgentSignal::Spend { usage });
    assert!(ops_in(&effects, "CountCommits").is_empty(), "{effects:#?}");
    let round = fx.task("t1").rounds.last().unwrap().clone();
    assert!(round.turn_open);
    assert_eq!(round.usage, usage);
    assert_eq!(fx.task("t1").spent_total.tokens, usage.billable());
    assert_alive(&fx);
}

/// Decision 28 must not start a fresh session for a round that is gone: a resume that
/// fails after the engine cancelled the task is ignored (a pinning test; the reducer's
/// ruling T12-N correlation already drops it).
#[test]
fn a_resume_failure_for_a_killed_round_is_ignored() {
    let (mut fx, window) = working_on(PROFILE, CHECK_MODE);
    fx.signal(window, AgentSignal::ProcessStarted { pid: 41 });
    fx.signal(window, AgentSignal::TurnStarted);
    let effects = exit(&mut fx, window, 41);
    let (op, _) = ops_in(&effects, "ResumeSession")[0].clone();
    let reply = fx.reply();
    fx.next(EventKind::Cancel {
        reply,
        run_id: RUN_ID.into(),
    });
    let sessions = fx.task("t1").session;
    let effects = fx.done(
        op,
        OpResult::ResumeFailed {
            error: "could not resume session s: it was killed".into(),
        },
    );
    assert!(ops_in(&effects, "CreateWindow").is_empty(), "{effects:#?}");
    assert!(fx.task("t1").fresh_session.is_none());
    assert_eq!(fx.task("t1").session, sessions);
    assert_alive(&fx);
}

/// A run's random session nonce (M8a.21's carry) makes its Claude session uuids its
/// own: two daemons that drew the same run id never share one.
#[test]
fn the_session_nonce_reaches_the_session_uuid() {
    let plan = plan_with(PROFILE, &[task("t1", "S", "a", CHECK_MODE)]);
    let mut fx = Fixture::new(&plan);
    fx.start(true);
    fx.run_mut().session_nonce = 0x5eed;
    let (op, _) = fx.op("CreateRunBranch");
    fx.done(op, OpResult::Worktree { head: BASE.into() });
    fx.complete_prepares();
    let (op, kind) = fx.op("CreateWindow");
    let OpKind::CreateWindow {
        session_uuid: uuid, ..
    } = kind
    else {
        unreachable!()
    };
    let uuid = uuid.expect("a Claude worker has a session uuid");
    assert_ne!(uuid, session_uuid(RUN_ID, op));
    assert_eq!(uuid, session_uuid_of(fx.run(), op));
    fx.run_mut().session_nonce = 0;
    assert_eq!(session_uuid_of(fx.run(), op), session_uuid(RUN_ID, op));
    assert_alive(&fx);
}
