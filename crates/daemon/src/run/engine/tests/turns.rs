//! M8a.12: turn-based delivery (decision 29), the turn-end fallback, the stall watchdog,
//! rate limits and failed turns, denials and process exits (decision 32).

use proto::{BlockReason, DoneSignal, TaskState};

use super::fixture::*;
use super::holds::{delivers, joined};
use crate::headless::FailureKind;
use crate::run::contract::{
    DONE_NUDGE, NO_COMMIT_NUDGE, RESUME_AFTER_EXIT, rate_limit_continue, stall_nudge,
};
use crate::run::engine::{AgentSignal, Effect, EventKind, OpKind, OpResult, TurnOutcome};
use crate::run::messages::DELIVERY_RETRY_SECS;
use crate::run::model::StallState;
use crate::run::snapshot::snapshot;

/// A working `t1` (S) whose first turn is open; its window.
pub(super) fn working_on(extra: &str) -> (Fixture, u32) {
    let mut fx = Fixture::new(&plan_with(PROFILE, &[task("t1", "S", "a", extra)]));
    fx.ready(true);
    let window = fx.launch_all()[0].1;
    assert!(fx.task("t1").rounds[0].turn_open);
    (fx, window)
}

pub(super) fn working() -> (Fixture, u32) {
    working_on("")
}

pub(super) fn queue(fx: &mut Fixture, text: &str) {
    let now = fx.now;
    crate::run::engine::outbox::queue(fx.run_mut(), "t1", text.to_string(), now);
}

/// The exit of the window's process, pid 7. Every process the helpers end has started
/// first (the ordering contract, `AgentSignal`'s doc), so a later one of the same
/// window is a new process, not a repeat of the last exit (final review B-10 drops a
/// repeat): the helpers reuse pid 7 for each.
pub(super) fn exited(fx: &mut Fixture, window: u32) -> Vec<Effect> {
    fx.signal(window, AgentSignal::ProcessStarted { pid: 7 });
    fx.signal(
        window,
        AgentSignal::ProcessExited {
            code: None,
            killed_by_engine: false,
            pid: 7,
        },
    )
}

pub(super) fn killed_exit(fx: &mut Fixture, window: u32) -> Vec<Effect> {
    fx.signal(window, AgentSignal::ProcessStarted { pid: 7 });
    fx.signal(
        window,
        AgentSignal::ProcessExited {
            code: None,
            killed_by_engine: true,
            pid: 7,
        },
    )
}

fn failed(kind: FailureKind, error: &str) -> TurnOutcome {
    TurnOutcome::Failed {
        error: error.into(),
        kind,
    }
}

fn delivered(fx: &mut Fixture, effects: &[Effect], ok: bool) -> Vec<Effect> {
    let ids = effects
        .iter()
        .find_map(|e| match e {
            Effect::Deliver { message_ids, .. } => Some(message_ids.clone()),
            _ => None,
        })
        .expect("a delivery");
    fx.next(EventKind::Delivered {
        run_id: RUN_ID.into(),
        message_ids: ids,
        ok,
        error: (!ok).then(|| "pipe closed".to_string()),
    })
}

fn commits(fx: &mut Fixture, count: u32) -> Vec<Effect> {
    let (op, _) = fx.op("CountCommits");
    fx.done(
        op,
        OpResult::Commits {
            count,
            head: HEAD.into(),
        },
    )
}

fn block_of(fx: &Fixture) -> (BlockReason, String) {
    let block = fx.task("t1").block.clone().expect("t1 is blocked");
    (block.reason, block.text)
}

#[test]
fn deliveries_wait_for_the_turn_to_end() {
    let (mut fx, window) = working();
    queue(&mut fx, "[anthrex] A");
    queue(&mut fx, "[anthrex] B");
    let effects = fx.tick();
    assert!(delivers(&effects).is_empty(), "the turn is open");
    let effects = fx.turn_completed(window);
    let texts = delivers(&effects);
    let ab = joined(&["[anthrex] A".into(), "[anthrex] B".into()]);
    assert_eq!(texts, vec![ab.clone()], "one turn, both in queue order");
    assert!(
        fx.task("t1").rounds[0].turn_open,
        "the delivery opens the turn"
    );

    // A failed delivery puts both back and retries after DELIVERY_RETRY_SECS.
    let mut last = effects;
    for failure in 1..=3u8 {
        let effects = delivered(&mut fx, &last, false);
        assert!(delivers(&effects).is_empty());
        let queued: Vec<&str> = fx.run().outbox.iter().map(|m| m.text.as_str()).collect();
        assert_eq!(
            queued,
            vec!["[anthrex] A", "[anthrex] B"],
            "back at the head"
        );
        if failure == 3 {
            break;
        }
        let at = fx.now;
        // Ruling T24-clock: at least DELIVERY_RETRY_SECS real seconds.
        let early = fx.send(at + DELIVERY_RETRY_SECS, EventKind::Tick);
        assert!(delivers(&early).is_empty());
        last = fx.send(at + DELIVERY_RETRY_SECS + 1, EventKind::Tick);
        assert_eq!(delivers(&last), vec![ab.clone()]);
    }
    assert_eq!(
        block_of(&fx),
        (
            BlockReason::Environment,
            "could not deliver to the agent: pipe closed".to_string()
        )
    );
    let effects = fx.send(fx.now + 60, EventKind::Tick);
    assert!(
        delivers(&effects).is_empty(),
        "a blocked task holds its messages"
    );
}

#[test]
fn turn_end_fallback_nudges_once_then_proceeds() {
    let (mut fx, window) = working();
    let effects = fx.turn_completed(window);
    let counts = ops_in(&effects, "CountCommits");
    assert_eq!(counts.len(), 1, "{effects:#?}");
    let OpKind::CountCommits {
        start, run_head, ..
    } = &counts[0].1
    else {
        unreachable!()
    };
    assert_eq!((start.as_str(), run_head.as_str()), (BASE, BASE));
    let effects = commits(&mut fx, 2);
    assert_eq!(delivers(&effects), vec![DONE_NUDGE.to_string()]);

    // The nudge turn ends without task_done: the task proceeds as if it had called it.
    let effects = fx.turn_completed(window);
    let verifies = ops_in(&effects, "VerifyDone");
    assert_eq!(verifies.len(), 1, "{effects:#?}");
    let OpKind::VerifyDone { red, .. } = &verifies[0].1 else {
        unreachable!()
    };
    assert_eq!(red, &None);
    assert!(ops_in(&effects, "CountCommits").is_empty());
    let mut result = fx.clean_check("t1");
    if let OpResult::DoneChecked { red_ok, .. } = &mut result {
        *red_ok = None;
    }
    let effects = fx.done(verifies[0].0, result);
    assert!(
        !effects.iter().any(|e| matches!(e, Effect::Reply { .. })),
        "nobody waits for a reply"
    );
    let t1 = fx.task("t1");
    // A tdd task goes on to its proof, which fails at once: no test or red was named
    // (M8a.13; `gates::turn_end_fallback_on_a_tdd_task_fails_the_proof_…` pins the text).
    assert_eq!(
        (t1.state, t1.bounces.proof, t1.proofs.len()),
        (TaskState::Working, 1, 1),
        "a tdd task goes on to its proof"
    );
    assert_eq!(
        t1.done.as_ref().unwrap().signal,
        DoneSignal::TurnEndFallback
    );
    assert_eq!(t1.done.as_ref().unwrap().test, None);

    // A task_done accepted inside the nudge turn ends the fallback instead.
    let (mut fx, window) = working();
    fx.turn_completed(window);
    commits(&mut fx, 2);
    let args = serde_json::json!({"summary": "s", "test": "a::works", "red": "abcdef1"});
    let effects = fx.tool(window, "task_done", args);
    let (op, _) = ops_in(&effects, "VerifyDone")[0].clone();
    let result = fx.clean_check("t1");
    fx.done(op, result);
    assert_eq!(fx.task("t1").state, TaskState::Proof);
    let effects = fx.turn_completed(window);
    assert!(ops_in(&effects, "VerifyDone").is_empty(), "{effects:#?}");
    assert!(ops_in(&effects, "CountCommits").is_empty());
    let done = fx.task("t1").done.clone().unwrap();
    assert_eq!(done.signal, DoneSignal::TaskDone);
}

#[test]
fn turn_end_without_commits_nudges_then_counts_as_a_stall() {
    let (mut fx, window) = working();
    fx.turn_completed(window);
    let effects = commits(&mut fx, 0);
    assert_eq!(delivers(&effects), vec![NO_COMMIT_NUDGE.to_string()]);
    let effects = fx.turn_completed(window);
    assert_eq!(ops_in(&effects, "CountCommits").len(), 1, "counted again");
    assert_eq!(fx.task("t1").stalls, 0);
    let effects = commits(&mut fx, 0);
    let t1 = fx.task("t1");
    assert_eq!((t1.stalls, t1.failures, t1.rung), (1, 1, 2));
    assert!(effects.contains(&Effect::KillWindow { window_id: window }));

    // A commit in the no-commit nudge's turn gets the done nudge instead.
    let (mut fx, window) = working();
    fx.turn_completed(window);
    commits(&mut fx, 0);
    fx.turn_completed(window);
    let effects = commits(&mut fx, 1);
    assert_eq!(delivers(&effects), vec![DONE_NUDGE.to_string()]);
    assert_eq!(fx.task("t1").stalls, 0);
}

#[test]
fn open_subagents_defer_the_fallback() {
    let (mut fx, window) = working();
    let start = |id: &str| AgentSignal::SubagentStart {
        agent_id: id.into(),
    };
    fx.signal(window, start("a1"));
    let effects = fx.turn_completed(window);
    assert!(ops_in(&effects, "CountCommits").is_empty(), "{effects:#?}");
    let effects = fx.tick();
    assert!(ops_in(&effects, "CountCommits").is_empty());
    let effects = fx.signal(
        window,
        AgentSignal::SubagentStop {
            agent_id: "a1".into(),
        },
    );
    assert_eq!(ops_in(&effects, "CountCommits").len(), 1);

    // Or until the next TurnEnded, sub-agents still open.
    let (mut fx, window) = working();
    fx.signal(window, start("a1"));
    fx.turn_completed(window);
    fx.signal(window, AgentSignal::TurnStarted);
    let effects = fx.turn_completed(window);
    assert_eq!(ops_in(&effects, "CountCommits").len(), 1, "{effects:#?}");
}

/// Budgets far from the stall tests' clocks (S's 15 minutes would breach at 22.5).
const ROOMY: &str = "[task.budget]\ntool_calls = 1000\nminutes = 1000";

#[test]
fn stall_interrupts_then_nudges_then_goes_to_rung_2() {
    let (mut fx, window) = working_on(ROOMY);
    let effort = fx.task("t1").route.effort;
    let quiet = fx.task("t1").rounds[0].last_event;
    // Ruling T24-clock: 600 whole engine seconds may be 599.x real ones; the stall
    // comes one second later.
    let effects = fx.send(quiet + 600, EventKind::Tick);
    assert!(!effects.contains(&Effect::Interrupt { window_id: window }));
    let effects = fx.send(quiet + 601, EventKind::Tick);
    assert!(effects.contains(&Effect::Interrupt { window_id: window }));
    assert!(matches!(
        fx.task("t1").rounds[0].stall,
        StallState::Interrupted { .. }
    ));
    let effects = fx.tick();
    assert!(
        delivers(&effects).is_empty(),
        "the nudge waits for the turn end"
    );
    assert!(!effects.contains(&Effect::Interrupt { window_id: window }));

    let effects = fx.turn_ended(window, TurnOutcome::Interrupted);
    assert_eq!(delivers(&effects), vec![stall_nudge(10)]);
    assert!(ops_in(&effects, "CountCommits").is_empty());
    let quiet = fx.now;
    let effects = fx.send(quiet + 600, EventKind::Tick);
    assert!(!effects.contains(&Effect::KillWindow { window_id: window }));
    let effects = fx.send(quiet + 601, EventKind::Tick);
    assert!(effects.contains(&Effect::KillWindow { window_id: window }));
    assert!(!effects.contains(&Effect::Interrupt { window_id: window }));
    let t1 = fx.task("t1");
    assert_eq!((t1.stalls, t1.failures, t1.rung), (1, 1, 2));

    // Rung 2: once the old session is gone, a fresh one with the hand-over prompt.
    let effects = killed_exit(&mut fx, window);
    let diffs = ops_in(&effects, "DiffSoFar");
    assert_eq!(diffs.len(), 1, "{effects:#?}");
    let effects = fx.done(
        diffs[0].0,
        OpResult::Diff {
            stat: " a.rs | 2 +-".into(),
            patch: "diff --git a/a.rs b/a.rs".into(),
        },
    );
    let windows = ops_in(&effects, "CreateWindow");
    assert_eq!(windows.len(), 1, "{effects:#?}");
    let OpKind::CreateWindow {
        first_turn, spec, ..
    } = &windows[0].1
    else {
        unreachable!()
    };
    assert!(first_turn.contains("This is session 2 of this task."));
    assert!(first_turn.contains("diff --git a/a.rs b/a.rs"));
    assert_eq!(
        Some(spec.effort),
        effort.raised(),
        "rung 2 escalates the route"
    );
    assert_eq!(fx.task("t1").session, 2);

    // An interrupt that does not end the turn within INTERRUPT_GRACE: rung 2 directly.
    let (mut fx, window) = working_on(ROOMY);
    let quiet = fx.task("t1").rounds[0].last_event;
    fx.send(quiet + 601, EventKind::Tick);
    let effects = fx.send(quiet + 631, EventKind::Tick);
    assert!(!effects.contains(&Effect::KillWindow { window_id: window }));
    let effects = fx.send(quiet + 632, EventKind::Tick);
    assert!(effects.contains(&Effect::KillWindow { window_id: window }));
    let t1 = fx.task("t1");
    assert_eq!((t1.stalls, t1.rung), (1, 2));
}

#[test]
fn rate_limit_retry_suspends_the_stall_clock() {
    let (mut fx, window) = working_on(ROOMY);
    let retry = AgentSignal::ApiRetry {
        error: "rate_limit".into(),
        delay_ms: 900_000,
    };
    fx.signal(window, retry.clone());
    // Ruling T24-clock: at least the retry's 900 s; the stall 600 s after that.
    let until = fx.now + 901;
    assert_eq!(fx.task("t1").rounds[0].rate_limited_until, Some(until));
    let effects = fx.send(until - 1, EventKind::Tick);
    assert!(!effects.contains(&Effect::Interrupt { window_id: window }));
    let effects = fx.send(until + 599, EventKind::Tick);
    assert!(!effects.contains(&Effect::Interrupt { window_id: window }));
    let effects = fx.send(until + 600, EventKind::Tick);
    assert!(effects.contains(&Effect::Interrupt { window_id: window }));

    // One rate-limit event per streak.
    let (mut fx, window) = working();
    fx.signal(window, retry.clone());
    fx.signal(window, retry.clone());
    assert_eq!(fx.run().rate_limits.get("claude"), Some(&1));
    fx.signal(window, AgentSignal::Activity);
    fx.signal(window, retry);
    assert_eq!(fx.run().rate_limits.get("claude"), Some(&2));
}

#[test]
fn failed_turns() {
    // A rate limit: rate_limit_continue once rate_limit_retry_secs have surely passed:
    // one engine second more than the wait, since `now` is truncated (M8a.24).
    let (mut fx, window) = working();
    fx.turn_ended(window, failed(FailureKind::RateLimit, "rate_limit"));
    let at = fx.now;
    let t1 = fx.task("t1");
    assert_eq!((t1.failures, t1.state), (0, TaskState::Working));
    assert_eq!(t1.rounds[0].rate_limited_until, Some(at + 301));
    assert!(snapshot(&fx.state, at + 1).runs[0].tasks[0].rounds[0].rate_limited);
    assert!(fx.ops("CountCommits").is_empty(), "not a completed turn");
    let effects = fx.send(at + 300, EventKind::Tick);
    assert!(delivers(&effects).is_empty());
    let effects = fx.send(at + 301, EventKind::Tick);
    assert_eq!(delivers(&effects), vec![rate_limit_continue("rate_limit")]);
    assert!(!snapshot(&fx.state, at + 301).runs[0].tasks[0].rounds[0].rate_limited);

    for (kind, error) in [
        (FailureKind::Authentication, "authentication_failed: log in"),
        (FailureKind::Billing, "billing_error: add credits"),
    ] {
        let (mut fx, window) = working();
        fx.turn_ended(window, failed(kind, error));
        assert_eq!(block_of(&fx), (BlockReason::Environment, error.to_string()));
    }

    // Two other failures in a row.
    let (mut fx, window) = working();
    fx.turn_ended(window, failed(FailureKind::Other, "overloaded"));
    let at = fx.now;
    assert_eq!(fx.task("t1").rounds[0].rate_limited_until, None);
    // A message queued meanwhile waits out the same continue (decision 29).
    queue(&mut fx, "[anthrex] X");
    let effects = fx.send(at + 300, EventKind::Tick);
    assert!(delivers(&effects).is_empty(), "{effects:#?}");
    let effects = fx.send(at + 301, EventKind::Tick);
    let both = joined(&["[anthrex] X".into(), rate_limit_continue("overloaded")]);
    assert_eq!(delivers(&effects), vec![both]);
    assert_eq!(fx.task("t1").state, TaskState::Working);
    fx.turn_ended(window, failed(FailureKind::Other, "overloaded again"));
    assert_eq!(
        block_of(&fx),
        (BlockReason::Environment, "overloaded again".to_string())
    );
}

#[test]
fn a_failed_rate_limit_turn_is_a_rate_limit_event() {
    let retry = AgentSignal::ApiRetry {
        error: "rate_limit".into(),
        delay_ms: 1_000,
    };
    let limited = failed(FailureKind::RateLimit, "rate_limit");

    let (mut fx, window) = working();
    fx.turn_ended(window, limited.clone());
    assert_eq!(fx.run().rate_limits.get("claude"), Some(&1));

    let (mut fx, window) = working();
    fx.signal(window, retry.clone());
    fx.turn_ended(window, limited.clone());
    assert_eq!(
        fx.run().rate_limits.get("claude"),
        Some(&1),
        "the streak ran into it"
    );

    let (mut fx, window) = working();
    fx.signal(window, retry);
    fx.signal(
        window,
        AgentSignal::ToolUse {
            name: "Bash".into(),
        },
    );
    fx.turn_ended(window, limited);
    assert_eq!(fx.run().rate_limits.get("claude"), Some(&2));
}

#[test]
fn a_process_that_dies_mid_turn_is_resumed_once() {
    let (mut fx, window) = working();
    let session_id = fx.task("t1").rounds[0].session_id.clone().unwrap();
    let effects = exited(&mut fx, window);
    let resumes = ops_in(&effects, "ResumeSession");
    assert_eq!(resumes.len(), 1, "{effects:#?}");
    assert_eq!(
        resumes[0].1,
        OpKind::ResumeSession {
            window_id: window,
            session_id,
            message: RESUME_AFTER_EXIT.into(),
            jitter_ms: match &resumes[0].1 {
                OpKind::ResumeSession { jitter_ms, .. } => *jitter_ms,
                _ => unreachable!(),
            },
        }
    );
    fx.done(resumes[0].0, OpResult::Resumed);
    assert_eq!(fx.task("t1").stalls, 0);
    let effects = exited(&mut fx, window);
    assert!(ops_in(&effects, "ResumeSession").is_empty());
    let t1 = fx.task("t1");
    assert_eq!((t1.stalls, t1.failures, t1.rung), (1, 1, 2));
    // The dead session is gone already: the fresh one starts from its diff.
    assert_eq!(ops_in(&effects, "DiffSoFar").len(), 1, "{effects:#?}");
}

#[test]
fn a_claude_process_that_exits_between_turns_is_resumed_on_the_next_delivery() {
    let (mut fx, window) = working();
    fx.turn_completed(window);
    let effects = exited(&mut fx, window);
    assert!(
        ops_in(&effects, "ResumeSession").is_empty(),
        "between turns"
    );
    assert!(fx.task("t1").rounds[0].ended, "the round is marked ended");
    // The fallback's nudge is the next message: it resumes the session.
    let effects = commits(&mut fx, 2);
    assert!(delivers(&effects).is_empty());
    let resumes = ops_in(&effects, "ResumeSession");
    assert_eq!(resumes.len(), 1, "{effects:#?}");
    let OpKind::ResumeSession { message, .. } = &resumes[0].1 else {
        unreachable!()
    };
    assert_eq!(message, DONE_NUDGE);
    assert!(fx.task("t1").rounds[0].turn_open);
    fx.done(resumes[0].0, OpResult::Resumed);
    assert!(fx.run().outbox.is_empty(), "delivered");
    assert!(!fx.task("t1").rounds[0].ended);

    // A resume that fails gives a fresh session whose prompt ends with the message.
    let (mut fx, window) = working();
    fx.turn_completed(window);
    exited(&mut fx, window);
    let effects = commits(&mut fx, 2);
    let (op, _) = ops_in(&effects, "ResumeSession")[0].clone();
    let effects = fx.done(
        op,
        OpResult::ResumeFailed {
            error: "no such session".into(),
        },
    );
    assert_eq!(fx.task("t1").failures, 0, "not a failure");
    let (op, _) = ops_in(&effects, "DiffSoFar")[0].clone();
    let effects = fx.done(
        op,
        OpResult::Diff {
            stat: String::new(),
            patch: String::new(),
        },
    );
    let windows = ops_in(&effects, "CreateWindow");
    let OpKind::CreateWindow {
        first_turn, spec, ..
    } = &windows[0].1
    else {
        unreachable!()
    };
    assert!(first_turn.ends_with(DONE_NUDGE), "{first_turn}");
    assert_eq!(
        spec.effort,
        fx.task("t1").rounds[0].route.effort,
        "same rung"
    );
    assert!(fx.run().outbox.is_empty());
}

#[test]
fn a_codex_exit_after_turn_completed_is_normal() {
    let (mut fx, window) = working_on("[task.route]\nruntime = \"codex\"\nmodel = \"\"");
    fx.turn_completed(window);
    let before = fx.run().clone();
    let effects = fx.signal(
        window,
        AgentSignal::ProcessExited {
            code: Some(0),
            killed_by_engine: false,
            pid: 7,
        },
    );
    assert_eq!(fx.run(), &before, "nothing changes");
    assert!(effects.is_empty(), "{effects:#?}");
}

#[test]
fn a_session_the_engine_killed_is_not_treated_as_an_exit() {
    let (mut fx, window) = working();
    let effects = killed_exit(&mut fx, window);
    assert!(ops_in(&effects, "ResumeSession").is_empty());
    assert!(ops_in(&effects, "DiffSoFar").is_empty());
    let t1 = fx.task("t1");
    assert_eq!((t1.stalls, t1.failures, t1.rung), (0, 0, 0));
    assert_eq!(t1.state, TaskState::Working);
}
