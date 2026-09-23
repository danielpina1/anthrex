//! M8a.12 fix round 1: the review's probes A–G as regression tests (rulings T12-I1..I4
//! and T12-minors), each ending with the liveness check of ruling T12-I4: a working
//! task always has an open, watched turn, a deliverable message, a timer, a kill or an
//! op in flight — never nothing.

use proto::{AgentRole, PlanEdit, TaskState};
use serde_json::json;

use super::dispatch::{edit, replies};
use super::fixture::*;
use super::holds::delivers;
use super::turns::{exited, killed_exit, queue, working_on};
use crate::headless::FailureKind;
use crate::run::contract::{
    DONE_ACCEPTED, DONE_NUDGE, RESUME_AFTER_EXIT, answer_message, rate_limit_continue, stall_nudge,
};
use crate::run::engine::{AgentSignal, Effect, EventKind, OpKind, OpResult, TurnOutcome};
use crate::run::model::{FailedTurn, FallbackState, StallState};

const ROOMY: &str = "[task.budget]\ntool_calls = 1000\nminutes = 1000";
const CODEX_ROOMY: &str = "[task.route]\nruntime = \"codex\"\nmodel = \"\"\n[task.budget]\ntool_calls = 1000\nminutes = 1000";

fn args() -> serde_json::Value {
    json!({"summary": "did it", "test": "a::works", "red": "abcdef1"})
}

/// Ruling T12-I4's invariant, for every working task of the fixture run.
pub(super) fn assert_alive(fx: &Fixture) {
    let run = fx.run();
    for t in run.tasks.iter().filter(|t| t.state == TaskState::Working) {
        let round = t.rounds.iter().rev().find(|r| r.role == AgentRole::Worker);
        let live =
            |r: &&crate::run::model::AgentRound| r.window_id.is_some() && !r.ended && !r.retiring;
        let open = round.is_some_and(|r| live(&r) && r.turn_open);
        let deliverable = t.fresh_session.is_some()
            || round.is_some_and(|r| {
                (live(&r) && !matches!(r.stall, StallState::Interrupted { .. }))
                    || (r.ended && !r.retiring && r.session_id.is_some())
            });
        let queued = run
            .outbox
            .iter()
            .any(|m| m.task_id == t.spec.id && (m.delivered_at.is_some() || deliverable));
        let timer = round.is_some_and(|r| {
            matches!(r.failed_turn, FailedTurn::WaitingContinue { .. })
                || r.delivery_retry_at.is_some()
                || r.count_retry_at.is_some()
        });
        let op = run
            .pending_ops
            .values()
            .any(|p| p.task_id.as_deref() == Some(t.id()));
        let killing = t.rounds.iter().any(|r| r.retiring && !r.ended);
        assert!(
            open || queued || timer || op || killing,
            "{} is working with nothing pending: {:#?}",
            t.spec.id,
            t
        );
    }
}

fn resume_messages(effects: &[Effect]) -> Vec<String> {
    ops_in(effects, "ResumeSession")
        .into_iter()
        .map(|(_, k)| match k {
            OpKind::ResumeSession { message, .. } => message,
            _ => unreachable!(),
        })
        .collect()
}

/// Probe A (T12-I2): a Codex interrupt is a `ProcessExited` with no `TurnEnded` (M8a.1).
/// That exit ends the interrupted turn: `stall_nudge` goes next, no death is counted
/// and the grace does not kill the session.
#[test]
fn a_codex_interrupt_exit_ends_the_turn_and_gets_the_nudge() {
    let (mut fx, window) = working_on(CODEX_ROOMY);
    fx.signal(
        window,
        AgentSignal::Init {
            session_id: "thread-1".into(),
        },
    );
    let quiet = fx.task("t1").rounds[0].last_event;
    let effects = fx.send(quiet + 600, EventKind::Tick);
    assert!(effects.contains(&Effect::Interrupt { window_id: window }));
    let effects = exited(&mut fx, window);
    assert!(resume_messages(&effects).is_empty(), "{effects:#?}");
    // Codex: the next turn is an `exec resume` the delivery starts.
    assert_eq!(delivers(&effects), vec![stall_nudge(10)]);
    let round = &fx.task("t1").rounds[0];
    assert_eq!((round.deaths, round.stall), (0, StallState::Nudged));
    assert_alive(&fx);
    let effects = fx.send(fx.now + 30, EventKind::Tick);
    assert!(!effects.contains(&Effect::KillWindow { window_id: window }));
    assert_eq!(fx.task("t1").stalls, 0);
    assert_alive(&fx);
}

/// T12-I2 for Claude under `SIGINT`: the exit ends the turn, and the nudge resumes the
/// session instead of `RESUME_AFTER_EXIT`.
#[test]
fn a_claude_exit_on_the_interrupt_resumes_with_the_nudge() {
    let (mut fx, window) = working_on(ROOMY);
    let quiet = fx.task("t1").rounds[0].last_event;
    fx.send(quiet + 600, EventKind::Tick);
    let effects = exited(&mut fx, window);
    assert_eq!(resume_messages(&effects), vec![stall_nudge(10)]);
    assert!(!resume_messages(&effects).contains(&RESUME_AFTER_EXIT.to_string()));
    assert_eq!(fx.task("t1").rounds[0].deaths, 0);
    assert_alive(&fx);
    let effects = fx.send(fx.now + 30, EventKind::Tick);
    assert!(!effects.contains(&Effect::KillWindow { window_id: window }));
}

/// Probe B (T12-I4a): a Claude exit during a failed turn's wait keeps the timer; the
/// continue then resumes the session.
#[test]
fn an_exit_during_a_failed_turns_wait_keeps_its_continue() {
    let (mut fx, window) = working_on(ROOMY);
    fx.turn_ended(
        window,
        TurnOutcome::Failed {
            error: "rate_limit".into(),
            kind: FailureKind::RateLimit,
        },
    );
    let at = fx.now;
    exited(&mut fx, window);
    assert_alive(&fx);
    let effects = fx.send(at + 299, EventKind::Tick);
    assert!(resume_messages(&effects).is_empty());
    assert_alive(&fx);
    let effects = fx.send(at + 300, EventKind::Tick);
    assert_eq!(
        resume_messages(&effects),
        vec![rate_limit_continue("rate_limit")]
    );
    assert_alive(&fx);
}

/// T12-I4a: an exit while the fallback waits for sub-agents runs the fallback.
#[test]
fn an_exit_while_the_fallback_waits_for_subagents_runs_it() {
    let (mut fx, window) = working_on(ROOMY);
    fx.signal(
        window,
        AgentSignal::SubagentStart {
            agent_id: "a1".into(),
        },
    );
    fx.turn_completed(window);
    assert!(fx.ops("CountCommits").is_empty());
    let effects = exited(&mut fx, window);
    assert_eq!(ops_in(&effects, "CountCommits").len(), 1, "{effects:#?}");
    assert_alive(&fx);
}

/// Probe C (T12-I1): a claim in flight when rung 2 fires. Its result arrives after the
/// fresh session is live: it is dropped, and the fresh session goes on.
#[test]
fn a_claim_from_a_replaced_session_is_dropped() {
    let (mut fx, window) = working_on("[task.budget]\ntool_calls = 1000\nminutes = 5");
    let started = fx.task("t1").rounds[0].started_at;
    let effects = fx.tool(window, "task_done", args());
    let (verify, _) = ops_in(&effects, "VerifyDone")[0].clone();
    let effects = fx.send(started + 450, EventKind::Tick);
    assert!(effects.contains(&Effect::KillWindow { window_id: window }));
    assert!(fx.task("t1").claim.is_none(), "rung 2 clears the claim");
    // The waiting tool call is answered at once, not left to time out.
    let reply = replies(&effects);
    assert_eq!(reply.len(), 1, "{effects:#?}");
    assert!(
        reply[0].as_ref().unwrap_err().contains("being replaced"),
        "{reply:?}"
    );
    let effects = killed_exit(&mut fx, window);
    let (op, _) = ops_in(&effects, "DiffSoFar")[0].clone();
    fx.done(
        op,
        OpResult::Diff {
            stat: String::new(),
            patch: String::new(),
        },
    );
    let new = fx.complete_windows()[0].1;
    let result = fx.clean_check("t1");
    let effects = fx.done(verify, result);
    assert!(replies(&effects).is_empty(), "{effects:#?}");
    assert_eq!(fx.task("t1").state, TaskState::Working);
    assert!(!effects.contains(&Effect::KillWindow { window_id: new }));
    assert_alive(&fx);
    // The fresh session's own claim is accepted.
    let effects = fx.tool(new, "task_done", args());
    let (op, _) = ops_in(&effects, "VerifyDone")[0].clone();
    let result = fx.clean_check("t1");
    let effects = fx.done(op, result);
    assert_eq!(replies(&effects), vec![Ok(DONE_ACCEPTED.to_string())]);

    // The result arrives before the fresh launch: dropped, and the launch goes on.
    let (mut fx, window) = working_on("[task.budget]\ntool_calls = 1000\nminutes = 5");
    let started = fx.task("t1").rounds[0].started_at;
    let effects = fx.tool(window, "task_done", args());
    let (verify, _) = ops_in(&effects, "VerifyDone")[0].clone();
    fx.send(started + 450, EventKind::Tick);
    let result = fx.clean_check("t1");
    fx.done(verify, result);
    assert_eq!(fx.task("t1").state, TaskState::Working);
    assert!(fx.task("t1").fresh_session.is_some());
    let effects = killed_exit(&mut fx, window);
    let (op, _) = ops_in(&effects, "DiffSoFar")[0].clone();
    let effects = fx.done(
        op,
        OpResult::Diff {
            stat: String::new(),
            patch: String::new(),
        },
    );
    assert_eq!(ops_in(&effects, "CreateWindow").len(), 1);
}

/// T12-I1 (the implementer's concern 1): restore clears a claim and a count in flight.
#[test]
fn restore_clears_a_claim_and_a_count_in_flight() {
    let (mut fx, window) = working_on(ROOMY);
    fx.tool(window, "task_done", args());
    fx.task_mut("t1").rounds[0].fallback = FallbackState::Counting;
    let run = fx.run().clone();
    fx.next(EventKind::Restore {
        runs: vec![run],
        replay: Vec::new(),
    });
    let t1 = fx.task("t1");
    assert!(t1.claim.is_none());
    assert_eq!(t1.rounds[0].fallback, FallbackState::None);
}

/// Probe D (T12-I3): a failed resume ends the round for good. Later messages wait for
/// the fresh session, whose prompt ends with every one of them, in order.
#[test]
fn a_failed_resume_is_final_and_messages_accumulate_for_the_fresh_session() {
    let (mut fx, window) = working_on(ROOMY);
    fx.turn_completed(window);
    exited(&mut fx, window);
    let (op, _) = fx.op("CountCommits");
    fx.done(
        op,
        OpResult::Commits {
            count: 2,
            head: HEAD.into(),
        },
    );
    let (op, _) = fx.op("ResumeSession");
    let effects = fx.done(
        op,
        OpResult::ResumeFailed {
            error: "no such session".into(),
        },
    );
    let diffs = ops_in(&effects, "DiffSoFar");
    assert_eq!(diffs.len(), 1);
    queue(&mut fx, "[anthrex] X");
    let effects = fx.tick();
    assert!(
        resume_messages(&effects).is_empty(),
        "never again: {effects:#?}"
    );
    assert!(delivers(&effects).is_empty());
    assert_alive(&fx);
    let effects = fx.done(
        diffs[0].0,
        OpResult::Diff {
            stat: String::new(),
            patch: String::new(),
        },
    );
    let windows = ops_in(&effects, "CreateWindow");
    assert_eq!(windows.len(), 1, "{effects:#?}");
    let OpKind::CreateWindow { first_turn, .. } = &windows[0].1 else {
        unreachable!()
    };
    let tail = format!("{DONE_NUDGE}\n\n[anthrex] X");
    assert!(first_turn.ends_with(&tail), "{first_turn}");
    assert!(fx.run().outbox.is_empty());
    assert_alive(&fx);
}

/// Probe E (T12-I4b): `task_blocked` lands while an interrupt is pending. The answer is
/// then delivered, not held behind a stall that no longer applies.
#[test]
fn a_block_during_an_interrupt_clears_it_so_the_answer_delivers() {
    let (mut fx, window) = working_on(ROOMY);
    let quiet = fx.task("t1").rounds[0].last_event;
    fx.send(quiet + 600, EventKind::Tick);
    fx.tool(window, "task_blocked", json!({"reason": "which module?"}));
    assert_eq!(fx.task("t1").state, TaskState::Blocked);
    fx.turn_ended(window, TurnOutcome::Interrupted);
    let effects = edit(
        &mut fx,
        vec![PlanEdit::Answer {
            task_id: "t1".into(),
            text: "module a".into(),
        }],
    );
    assert_eq!(fx.task("t1").state, TaskState::Working);
    assert_eq!(delivers(&effects), vec![answer_message("module a")]);
    assert_alive(&fx);
    // And no kill for the stale interrupt.
    let effects = fx.send(fx.now + 31, EventKind::Tick);
    assert!(!effects.contains(&Effect::KillWindow { window_id: window }));
}

/// Probe F (review m-2): no `DONE_NUDGE` while an explicit claim is in flight.
#[test]
fn no_done_nudge_while_a_claim_is_in_flight() {
    let (mut fx, window) = working_on(ROOMY);
    fx.turn_completed(window);
    let (count, _) = fx.op("CountCommits");
    queue(&mut fx, "[anthrex] A");
    fx.tick();
    let effects = fx.tool(window, "task_done", args());
    assert_eq!(ops_in(&effects, "VerifyDone").len(), 1);
    fx.done(
        count,
        OpResult::Commits {
            count: 2,
            head: HEAD.into(),
        },
    );
    assert!(fx.run().outbox.iter().all(|m| m.text != DONE_NUDGE));
    assert_eq!(fx.task("t1").rounds[0].fallback, FallbackState::None);
    assert_alive(&fx);
}

/// A fresh session the window limit refuses keeps what it carries: the task is
/// blocked on its environment and `fresh_session` stays for a later retry.
#[test]
fn a_refused_fresh_launch_keeps_its_messages() {
    let (mut fx, window) = working_on("[task.budget]\ntool_calls = 1\nminutes = 1000");
    for _ in 0..2 {
        fx.signal(
            window,
            AgentSignal::ToolUse {
                name: "Bash".into(),
            },
        );
    }
    assert_eq!(fx.task("t1").rung, 2);
    let created = fx.run().windows_created;
    fx.run_mut().limits.max_windows = created;
    queue(&mut fx, "[anthrex] X");
    let effects = killed_exit(&mut fx, window);
    let (op, _) = ops_in(&effects, "DiffSoFar")[0].clone();
    let effects = fx.done(
        op,
        OpResult::Diff {
            stat: String::new(),
            patch: String::new(),
        },
    );
    assert!(ops_in(&effects, "CreateWindow").is_empty());
    let t1 = fx.task("t1");
    assert_eq!(t1.state, TaskState::Blocked);
    let append = t1.fresh_session.as_ref().and_then(|f| f.append.clone());
    assert_eq!(append.as_deref(), Some("[anthrex] X"));
}

/// T12-I1: a claim whose session's resume failed is dropped (the round is over), while
/// the same claim survives the session merely ending between turns.
#[test]
fn a_claim_follows_its_session_not_its_process() {
    // The process ends between turns: still the same session, and the claim stands.
    let (mut fx, window) = working_on(ROOMY);
    let effects = fx.tool(window, "task_done", args());
    let (verify, _) = ops_in(&effects, "VerifyDone")[0].clone();
    fx.turn_completed(window);
    exited(&mut fx, window);
    let result = fx.clean_check("t1");
    let effects = fx.done(verify, result);
    assert_eq!(replies(&effects), vec![Ok(DONE_ACCEPTED.to_string())]);

    // Its resume fails: the session is gone, and so is the claim.
    let (mut fx, window) = working_on(ROOMY);
    let effects = fx.tool(window, "task_done", args());
    let (verify, _) = ops_in(&effects, "VerifyDone")[0].clone();
    fx.turn_completed(window);
    exited(&mut fx, window);
    queue(&mut fx, "[anthrex] X");
    let effects = fx.tick();
    let (resume, _) = ops_in(&effects, "ResumeSession")[0].clone();
    fx.done(
        resume,
        OpResult::ResumeFailed {
            error: "no such session".into(),
        },
    );
    let result = fx.clean_check("t1");
    let effects = fx.done(verify, result);
    assert!(replies(&effects)[0].is_err(), "{effects:#?}");
    assert_eq!(fx.task("t1").state, TaskState::Working);
    assert!(fx.task("t1").fresh_session.is_some());
    assert_alive(&fx);
}

/// T12-I4, as ruling T12-N2 revised it: a claim rejected after its turn already ended
/// is the worker's next turn, so the task is not left with nothing pending.
#[test]
fn a_late_rejection_is_the_next_turn() {
    let (mut fx, window) = working_on(ROOMY);
    let effects = fx.tool(window, "task_done", args());
    let (verify, _) = ops_in(&effects, "VerifyDone")[0].clone();
    let effects = fx.turn_completed(window);
    assert!(
        ops_in(&effects, "CountCommits").is_empty(),
        "the claim is in flight"
    );
    let mut result = fx.clean_check("t1");
    if let OpResult::DoneChecked { dirty_tracked, .. } = &mut result {
        *dirty_tracked = 2;
    }
    let effects = fx.done(verify, result);
    assert!(replies(&effects)[0].is_err());
    assert!(ops_in(&effects, "CountCommits").is_empty(), "{effects:#?}");
    let turns = delivers(&effects);
    assert_eq!(turns.len(), 1, "{effects:#?}");
    assert!(turns[0].starts_with("[anthrex] task_done rejected: the tracked tree"));
    assert_alive(&fx);
}
