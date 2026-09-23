//! M8a.12 fix round 3: the second re-review's probes as regression tests. Rulings
//! T12-A1 (a failed resume supersedes its round's awaited count), T12-A2 (a failed
//! `CountCommits` is retried, and blocks at the third failure), T12-O1 (worker activity
//! during the interrupt grace cancels the grace kill) and T12-O2 (a verdict during a
//! later turn waits for that turn's end).

use proto::{BlockReason, TaskState};
use serde_json::json;

use super::done::block_of;
use super::fixture::*;
use super::holds::delivers;
use super::turns::{exited, queue, working_on};
use super::turns_fixes::assert_alive;
use crate::run::contract::DONE_NUDGE;
use crate::run::engine::TurnOutcome;
use crate::run::engine::{AgentSignal, Effect, EventKind, OpKind, OpResult};
use crate::run::messages::DELIVERY_RETRY_SECS;
use crate::run::model::StallState;

const ROOMY: &str = "[task.budget]\ntool_calls = 1000\nminutes = 1000";
const FOUR_CALLS: &str = "[task.budget]\ntool_calls = 4\nminutes = 1000";

fn args() -> serde_json::Value {
    json!({"summary": "did it", "test": "a::works", "red": "abcdef1"})
}

fn commits(count: u32) -> OpResult {
    OpResult::Commits {
        count,
        head: HEAD.into(),
    }
}

fn dirty(fx: &Fixture) -> OpResult {
    let mut r = fx.clean_check("t1");
    if let OpResult::DoneChecked { dirty_tracked, .. } = &mut r {
        *dirty_tracked = 2;
    }
    r
}

fn failed() -> OpResult {
    OpResult::Failed {
        message: "git broke".into(),
    }
}

fn delivered_ok(fx: &mut Fixture, effects: &[Effect]) {
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
        ok: true,
        error: None,
    });
}

/// Probe PA (T12-A1): the count of a session whose resume failed comes back after the
/// failure. It is dropped: no stall, and the pending fresh session is unchanged.
#[test]
fn a_count_after_a_failed_resume_is_dropped() {
    for count in [0, 2] {
        let (mut fx, w) = working_on(FOUR_CALLS);
        let effects = fx.turn_completed(w);
        let (c1, _) = ops_in(&effects, "CountCommits")[0].clone();
        let effects = fx.done(c1, commits(0));
        delivered_ok(&mut fx, &effects);
        for _ in 0..4 {
            fx.signal(
                w,
                AgentSignal::ToolUse {
                    name: "Bash".into(),
                },
            );
        }
        let effects = fx.turn_completed(w);
        let (c2, _) = ops_in(&effects, "CountCommits")[0].clone();
        let effects = exited(&mut fx, w);
        let (resume, _) = ops_in(&effects, "ResumeSession")[0].clone();
        fx.done(
            resume,
            OpResult::ResumeFailed {
                error: "gone".into(),
            },
        );
        let fresh = fx.task("t1").fresh_session.clone();
        let effects = fx.done(c2, commits(count));
        let t1 = fx.task("t1");
        assert_eq!(
            (t1.failures, t1.stalls, t1.rung),
            (0, 0, 0),
            "count {count}"
        );
        assert_eq!(t1.fresh_session, fresh, "count {count}");
        assert!(delivers(&effects).is_empty());
        assert!(!fx.run().outbox.iter().any(|m| m.text == DONE_NUDGE));
    }
}

/// A working `t1` whose completed turn's `CountCommits` failed once; the fixture, its
/// window and whether its process exited first.
fn failed_count(exit: bool) -> (Fixture, u32) {
    let (mut fx, w) = working_on(ROOMY);
    let effects = fx.turn_completed(w);
    let (count, _) = ops_in(&effects, "CountCommits")[0].clone();
    if exit {
        exited(&mut fx, w);
    }
    fx.done(count, failed());
    (fx, w)
}

/// Probe PB (T12-A2): a failed count is retried `DELIVERY_RETRY_SECS` later, live or
/// after the process exited; a count that then succeeds nudges as usual.
#[test]
fn a_failed_count_is_retried() {
    for exit in [false, true] {
        let (mut fx, _) = failed_count(exit);
        assert_alive(&fx);
        let failed_at = fx.now;
        let effects = fx.send(failed_at + DELIVERY_RETRY_SECS - 1, EventKind::Tick);
        assert!(ops_in(&effects, "CountCommits").is_empty(), "exit {exit}");
        let effects = fx.send(failed_at + DELIVERY_RETRY_SECS, EventKind::Tick);
        let retry = ops_in(&effects, "CountCommits");
        assert_eq!(retry.len(), 1, "exit {exit}: {effects:#?}");
        let effects = fx.done(retry[0].0, commits(2));
        let resumes: Vec<String> = ops_in(&effects, "ResumeSession")
            .into_iter()
            .map(|(_, k)| match k {
                OpKind::ResumeSession { message, .. } => message,
                _ => unreachable!(),
            })
            .collect();
        let sent = if exit { resumes } else { delivers(&effects) };
        assert_eq!(sent, vec![DONE_NUDGE.to_string()], "exit {exit}");
        assert!(ops_in(&effects, "CountCommits").is_empty(), "retried once");
        assert_alive(&fx);
    }
}

/// T12-A2: the third failed count in a row blocks the task as `environment`, with the
/// error.
#[test]
fn the_third_failed_count_blocks_the_task() {
    for exit in [false, true] {
        let (mut fx, _) = failed_count(exit);
        for _ in 0..2 {
            let at = fx.now + DELIVERY_RETRY_SECS;
            let effects = fx.send(at, EventKind::Tick);
            let (count, _) = ops_in(&effects, "CountCommits")[0].clone();
            fx.done(count, failed());
        }
        assert_eq!(fx.task("t1").state, TaskState::Blocked, "exit {exit}");
        let (reason, text) = block_of(&fx);
        assert_eq!(reason, BlockReason::Environment);
        assert!(text.contains("git broke"), "{text}");
    }
}

/// A working `t1` whose silent turn was interrupted; the fixture, its window and the
/// grace deadline.
fn interrupted() -> (Fixture, u32, u64) {
    let (mut fx, w) = working_on(ROOMY);
    let stall = fx.run().limits.stall_after_secs;
    let quiet = fx.task("t1").rounds[0].last_event;
    let effects = fx.send(quiet + stall, EventKind::Tick);
    assert!(effects.contains(&Effect::Interrupt { window_id: w }));
    let StallState::Interrupted { deadline } = fx.task("t1").rounds[0].stall else {
        panic!("interrupted")
    };
    (fx, w, deadline)
}

/// Probe PE (T12-O1): a `task_done` inside the interrupt grace cancels the grace kill;
/// the verdict then leaves the session alive.
#[test]
fn a_claim_in_the_grace_cancels_the_kill() {
    let (mut fx, w, deadline) = interrupted();
    let effects = fx.tool(w, "task_done", args());
    let (verify, _) = ops_in(&effects, "VerifyDone")[0].clone();
    fx.send(deadline + 100, EventKind::Tick);
    let result = dirty(&fx);
    let effects = fx.done(verify, result);
    assert!(
        !effects.contains(&Effect::KillWindow { window_id: w }),
        "{effects:#?}"
    );
    let effects = fx.send(deadline + 101, EventKind::Tick);
    assert!(!effects.contains(&Effect::KillWindow { window_id: w }));
    assert_eq!(fx.task("t1").rounds[0].stall, StallState::Nudged);
    assert_eq!(fx.task("t1").stalls, 0);
}

/// T12-O1: any stream event from the worker inside the grace cancels the kill too.
#[test]
fn activity_in_the_grace_cancels_the_kill() {
    let (mut fx, w, deadline) = interrupted();
    fx.signal(
        w,
        AgentSignal::ToolUse {
            name: "Bash".into(),
        },
    );
    let effects = fx.send(deadline, EventKind::Tick);
    assert!(
        !effects.contains(&Effect::KillWindow { window_id: w }),
        "{effects:#?}"
    );
    assert_eq!(fx.task("t1").rounds[0].stall, StallState::Nudged);
    assert_alive(&fx);
}

/// Probe PD (T12-O2): the verdict comes while a later turn is open (a queued message
/// opened it after the claiming turn ended). The rejection waits for that turn's end
/// and is delivered then.
#[test]
fn a_verdict_during_a_later_turn_is_its_next_turn() {
    let (mut fx, w) = working_on(ROOMY);
    let effects = fx.tool(w, "task_done", args());
    let (verify, _) = ops_in(&effects, "VerifyDone")[0].clone();
    queue(&mut fx, "[anthrex] meanwhile");
    let effects = fx.turn_completed(w);
    assert_eq!(delivers(&effects), vec!["[anthrex] meanwhile".to_string()]);
    let result = dirty(&fx);
    let effects = fx.done(verify, result);
    assert!(delivers(&effects).is_empty());
    let rejection = "[anthrex] task_done rejected: the tracked tree has uncommitted changes (2 files); commit or revert them first";
    assert!(
        fx.run()
            .outbox
            .iter()
            .any(|m| m.text == rejection && m.delivered_at.is_none()),
        "{:#?}",
        fx.run().outbox
    );
    let effects = fx.turn_completed(w);
    assert_eq!(
        delivers(&effects),
        vec![rejection.to_string()],
        "{effects:#?}"
    );
}

/// T12-A2: a count that succeeds resets the failures; two more failures do not block.
#[test]
fn a_successful_count_resets_the_failures() {
    let (mut fx, w) = failed_count(false);
    let effects = fx.send(fx.now + DELIVERY_RETRY_SECS, EventKind::Tick);
    let (count, _) = ops_in(&effects, "CountCommits")[0].clone();
    let effects = fx.done(count, commits(0));
    delivered_ok(&mut fx, &effects);
    let effects = fx.turn_completed(w);
    let (count, _) = ops_in(&effects, "CountCommits")[0].clone();
    fx.done(count, failed());
    let effects = fx.send(fx.now + DELIVERY_RETRY_SECS, EventKind::Tick);
    let (count, _) = ops_in(&effects, "CountCommits")[0].clone();
    fx.done(count, failed());
    assert_eq!(fx.task("t1").state, TaskState::Working);
    assert_alive(&fx);
}

/// T12-A2: the worker's own claim, made while a failed count waits, takes over: the
/// retry is not sent.
#[test]
fn a_claim_takes_over_from_a_count_retry() {
    let (mut fx, w) = failed_count(false);
    queue(&mut fx, "[anthrex] go on");
    fx.tick();
    fx.tool(w, "task_done", args());
    let effects = fx.send(fx.now + DELIVERY_RETRY_SECS, EventKind::Tick);
    assert!(ops_in(&effects, "CountCommits").is_empty(), "{effects:#?}");
}

/// T12-A2: while a failed count waits for its retry, another turn end does not count
/// too; the retry does, once. (After `NO_COMMIT_NUDGE`, whose count may be sent from
/// any turn end.)
#[test]
fn a_turn_end_waits_for_the_count_retry() {
    let (mut fx, w) = working_on(ROOMY);
    let effects = fx.turn_completed(w);
    let (count, _) = ops_in(&effects, "CountCommits")[0].clone();
    let effects = fx.done(count, commits(0));
    delivered_ok(&mut fx, &effects);
    let effects = fx.turn_completed(w);
    let (count, _) = ops_in(&effects, "CountCommits")[0].clone();
    fx.done(count, failed());
    queue(&mut fx, "[anthrex] go on");
    fx.tick();
    let effects = fx.turn_completed(w);
    assert!(ops_in(&effects, "CountCommits").is_empty(), "{effects:#?}");
    let effects = fx.send(fx.now + DELIVERY_RETRY_SECS, EventKind::Tick);
    assert_eq!(ops_in(&effects, "CountCommits").len(), 1);
}

/// T12-A2: a restart waits for no count retry.
#[test]
fn a_restore_drops_a_count_retry() {
    let (fx, _) = failed_count(false);
    let run = fx.run().clone();
    let mut restored = Fixture::new(&fx.plan);
    restored.next(EventKind::Restore {
        runs: vec![run],
        replay: vec![],
    });
    restored.run_mut().state = proto::RunState::Running;
    let effects = restored.send(fx.now + DELIVERY_RETRY_SECS, EventKind::Tick);
    assert!(ops_in(&effects, "CountCommits").is_empty(), "{effects:#?}");
}

/// T12-O1: the interrupted turn's own end is not activity inside the grace: a completed
/// interrupted turn gets the stall nudge, not the fallback.
#[test]
fn the_interrupted_turns_end_gets_the_nudge_not_the_fallback() {
    let (mut fx, w, _) = interrupted();
    let effects = fx.turn_ended(w, TurnOutcome::Completed);
    assert!(ops_in(&effects, "CountCommits").is_empty(), "{effects:#?}");
    assert_eq!(delivers(&effects).len(), 1, "the stall nudge");
}
