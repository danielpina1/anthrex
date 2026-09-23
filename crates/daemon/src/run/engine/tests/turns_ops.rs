//! M8a.12 fix round 2: the re-review's probes as regression tests. Rulings T12-N (every
//! op result is matched to the op the engine still awaits), T12-N2 (a late verdict
//! re-engages a worker whose turn is over), T12-N4 (a rejected fallback claim sends
//! only its rejection) and T12-later (the stall clock waits for a check; a Codex
//! interrupt before its session has an id is rung 2).

use serde_json::json;

use super::dispatch::replies;
use super::fixture::*;
use super::holds::delivers;
use super::turns::{exited, killed_exit, queue, working_on};
use super::turns_fixes::assert_alive;
use crate::run::contract::{
    DONE_ACCEPTED, DONE_NUDGE, NO_COMMIT_NUDGE, protected_file_message, stall_nudge,
};
use crate::run::engine::{Effect, EventKind, OpKind, OpResult};
use crate::run::model::{FallbackState, StallState};

const ROOMY: &str = "[task.budget]\ntool_calls = 1000\nminutes = 1000";
const FIVE_MIN: &str = "[task.budget]\ntool_calls = 1000\nminutes = 5";
const CODEX_ROOMY: &str = "[task.route]\nruntime = \"codex\"\nmodel = \"\"\n[task.budget]\ntool_calls = 1000\nminutes = 1000";

fn args() -> serde_json::Value {
    json!({"summary": "did it", "test": "a::works", "red": "abcdef1"})
}

fn with_head(mut r: OpResult, h: &str) -> OpResult {
    if let OpResult::DoneChecked { head, .. } = &mut r {
        *head = h.into();
    }
    r
}

fn dirty(fx: &Fixture) -> OpResult {
    let mut r = fx.clean_check("t1");
    if let OpResult::DoneChecked { dirty_tracked, .. } = &mut r {
        *dirty_tracked = 1;
    }
    r
}

fn protected(fx: &Fixture) -> OpResult {
    let mut r = fx.clean_check("t1");
    if let OpResult::DoneChecked {
        protected_changed, ..
    } = &mut r
    {
        *protected_changed = vec!["AGENTS.md".into()];
    }
    r
}

fn commits(count: u32) -> OpResult {
    OpResult::Commits {
        count,
        head: HEAD.into(),
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

/// Rung 2 through the minutes budget, the killed exit and the diff: the fresh window.
fn rung2_fresh(fx: &mut Fixture, window: u32) -> u32 {
    let started = fx.task("t1").rounds[0].started_at;
    let effects = fx.send(started + 450, EventKind::Tick);
    assert!(effects.contains(&Effect::KillWindow { window_id: window }));
    let effects = killed_exit(fx, window);
    let (op, _) = ops_in(&effects, "DiffSoFar")[0].clone();
    fx.done(
        op,
        OpResult::Diff {
            stat: String::new(),
            patch: String::new(),
        },
    );
    fx.complete_windows()[0].1
}

/// Probe P1 (T12-N): the replaced session's `VerifyDone` result settles nothing; the
/// fresh session's own claim waits for its own result.
#[test]
fn a_stale_verify_result_leaves_the_fresh_sessions_claim_pending() {
    let (mut fx, window) = working_on(FIVE_MIN);
    let effects = fx.tool(window, "task_done", args());
    let (old, _) = ops_in(&effects, "VerifyDone")[0].clone();
    let new = rung2_fresh(&mut fx, window);
    let effects = fx.tool(new, "task_done", args());
    let (fresh, _) = ops_in(&effects, "VerifyDone")[0].clone();

    let result = with_head(fx.clean_check("t1"), "OLD_HEAD");
    let effects = fx.done(old, result);
    assert!(replies(&effects).is_empty(), "{effects:#?}");
    assert!(fx.task("t1").claim.is_some());
    assert_eq!(fx.task("t1").head, None);

    let result = with_head(fx.clean_check("t1"), "NEW_HEAD");
    let effects = fx.done(fresh, result);
    assert_eq!(replies(&effects), vec![Ok(DONE_ACCEPTED.to_string())]);
    assert_eq!(fx.task("t1").head.as_deref(), Some("NEW_HEAD"));
}

/// Probe P1b (T12-N): nor does the replaced session's rejection reach the fresh one.
#[test]
fn a_stale_rejection_is_not_the_fresh_sessions() {
    let (mut fx, window) = working_on(FIVE_MIN);
    let effects = fx.tool(window, "task_done", args());
    let (old, _) = ops_in(&effects, "VerifyDone")[0].clone();
    let new = rung2_fresh(&mut fx, window);
    fx.tool(new, "task_done", args());
    let result = dirty(&fx);
    let effects = fx.done(old, result);
    assert!(replies(&effects).is_empty(), "{effects:#?}");
    assert!(delivers(&effects).is_empty());
    assert!(fx.task("t1").claim.is_some());
    assert!(fx.run().outbox.is_empty(), "{:#?}", fx.run().outbox);
}

/// A worker whose first session ended between turns and was resumed with the
/// fallback's nudge; the `ResumeSession` op of that resume.
fn resumed_once(fx: &mut Fixture, window: u32) -> u64 {
    fx.turn_completed(window);
    exited(fx, window);
    let (op, _) = fx.op("CountCommits");
    let effects = fx.done(op, commits(2));
    ops_in(&effects, "ResumeSession")[0].0
}

/// Probe P2 (T12-N): the replaced session's failed resume leaves the fresh session
/// live: no second fresh session, no kill, and its tools still answered.
#[test]
fn a_stale_resume_failure_leaves_the_fresh_session_live() {
    let (mut fx, window) = working_on(FIVE_MIN);
    let resume = resumed_once(&mut fx, window);
    let new = rung2_fresh(&mut fx, window);
    let effects = fx.done(
        resume,
        OpResult::ResumeFailed {
            error: "late".into(),
        },
    );
    let t1 = fx.task("t1");
    let last = t1.rounds.last().unwrap();
    assert_eq!(last.window_id, Some(new));
    assert!(!last.ended && !last.retiring, "{last:#?}");
    assert!(t1.fresh_session.is_none());
    assert!(!effects.contains(&Effect::KillWindow { window_id: new }));
    let effects = fx.tick();
    assert!(ops_in(&effects, "DiffSoFar").is_empty());
    let live = fx
        .task("t1")
        .rounds
        .iter()
        .filter(|r| !r.ended && !r.retiring);
    assert_eq!(live.count(), 1, "one live session");
    let effects = fx.tool(new, "task_done", args());
    assert_eq!(ops_in(&effects, "VerifyDone").len(), 1, "{effects:#?}");
}

/// Probe P2b (T12-N): the replaced session's successful resume does not take the
/// messages the fresh session's own resume carries.
#[test]
fn a_stale_resumed_leaves_the_fresh_resumes_messages() {
    let (mut fx, window) = working_on(FIVE_MIN);
    let old = resumed_once(&mut fx, window);
    let new = rung2_fresh(&mut fx, window);
    let effects = fx.turn_completed(new);
    let (count, _) = ops_in(&effects, "CountCommits")[0].clone();
    exited(&mut fx, new);
    let effects = fx.done(count, commits(2));
    assert_eq!(resume_messages(&effects), vec![DONE_NUDGE.to_string()]);
    let carried = fx.task("t1").rounds.last().unwrap().carried.clone();
    assert!(!carried.is_empty());

    fx.done(old, OpResult::Resumed);
    assert_eq!(fx.task("t1").rounds.last().unwrap().carried, carried);
    assert!(fx.run().outbox.iter().any(|m| m.text == DONE_NUDGE));
}

/// T12-N: the replaced session's commit count neither holds up the fresh session's own
/// fallback nor nudges it.
#[test]
fn a_stale_count_neither_blocks_nor_nudges_the_fresh_session() {
    let (mut fx, window) = working_on(FIVE_MIN);
    fx.turn_completed(window);
    let (old, _) = fx.op("CountCommits");
    let new = rung2_fresh(&mut fx, window);
    let effects = fx.turn_completed(new);
    let fresh = ops_in(&effects, "CountCommits");
    assert_eq!(
        fresh.len(),
        1,
        "the fresh session's fallback runs: {effects:#?}"
    );

    let effects = fx.done(old, commits(0));
    assert!(delivers(&effects).is_empty(), "{effects:#?}");
    assert!(!fx.run().outbox.iter().any(|m| m.text == NO_COMMIT_NUDGE));
    let last = fx.task("t1").rounds.last().unwrap();
    assert_eq!(last.fallback, FallbackState::Counting);

    let effects = fx.done(fresh[0].0, commits(2));
    assert_eq!(delivers(&effects), vec![DONE_NUDGE.to_string()]);
}

/// T12-N: a delivery to the replaced session that failed does not close the fresh
/// session's turn, count against it, or queue its text again.
#[test]
fn a_stale_delivery_failure_leaves_the_fresh_session_alone() {
    let (mut fx, window) = working_on(FIVE_MIN);
    fx.turn_completed(window);
    let (op, _) = fx.op("CountCommits");
    let effects = fx.done(op, commits(2));
    let ids = effects
        .iter()
        .find_map(|e| match e {
            Effect::Deliver { message_ids, .. } => Some(message_ids.clone()),
            _ => None,
        })
        .expect("the nudge is delivered");
    let new = rung2_fresh(&mut fx, window);
    assert!(fx.task("t1").rounds.last().unwrap().turn_open);

    fx.next(EventKind::Delivered {
        run_id: RUN_ID.into(),
        message_ids: ids,
        ok: false,
        error: Some("gone".into()),
    });
    let last = fx.task("t1").rounds.last().unwrap();
    assert_eq!(last.window_id, Some(new));
    assert!(last.turn_open, "{last:#?}");
    assert_eq!((last.delivery_failures, last.delivery_retry_at), (0, None));
    assert!(fx.run().outbox.is_empty(), "{:#?}", fx.run().outbox);
    fx.turn_completed(new);
    let effects = fx.tick();
    assert!(
        !delivers(&effects).contains(&DONE_NUDGE.to_string()),
        "{effects:#?}"
    );
}

/// Case (a) of T12-N2: the turn ended and the Claude process exited before the claim
/// was rejected. The rejection resumes the session as its next turn.
#[test]
fn a_rejection_after_the_process_exited_resumes_with_it() {
    let (mut fx, window) = working_on(ROOMY);
    let effects = fx.tool(window, "task_done", args());
    let (verify, _) = ops_in(&effects, "VerifyDone")[0].clone();
    fx.turn_completed(window);
    exited(&mut fx, window);
    let result = dirty(&fx);
    let effects = fx.done(verify, result);
    let resumes = resume_messages(&effects);
    assert_eq!(resumes.len(), 1, "{effects:#?}");
    assert!(resumes[0].contains("task_done rejected: the tracked tree"));
    assert_alive(&fx);
}

/// Case (b) of T12-N2: a rung-1 bounce after the turn ended, the process alive. The
/// bounce text is the next turn.
#[test]
fn a_bounce_after_the_turn_is_the_next_turn() {
    let (mut fx, window) = working_on(ROOMY);
    let effects = fx.tool(window, "task_done", args());
    let (verify, _) = ops_in(&effects, "VerifyDone")[0].clone();
    fx.turn_completed(window);
    let result = protected(&fx);
    let effects = fx.done(verify, result);
    assert_eq!(fx.task("t1").rung, 1);
    let bounce = protected_file_message(&["AGENTS.md".to_string()]);
    assert_eq!(delivers(&effects), vec![bounce], "{effects:#?}");
    assert_alive(&fx);
    let effects = fx.send(fx.now + 10, EventKind::Tick);
    assert!(
        delivers(&effects).is_empty(),
        "delivered once: {effects:#?}"
    );
}

/// Case (c) of T12-N2: a rung-1 bounce after the process exited resumes the session.
#[test]
fn a_bounce_after_the_process_exited_resumes_with_it() {
    let (mut fx, window) = working_on(ROOMY);
    let effects = fx.tool(window, "task_done", args());
    let (verify, _) = ops_in(&effects, "VerifyDone")[0].clone();
    fx.turn_completed(window);
    exited(&mut fx, window);
    let result = protected(&fx);
    let effects = fx.done(verify, result);
    let bounce = protected_file_message(&["AGENTS.md".to_string()]);
    assert_eq!(resume_messages(&effects), vec![bounce], "{effects:#?}");
    assert_alive(&fx);
}

/// Case (d) of T12-N2: a check that could not run, after the process exited, resumes
/// the session with the failure.
#[test]
fn a_failed_check_after_the_process_exited_resumes_with_it() {
    let (mut fx, window) = working_on(ROOMY);
    let effects = fx.tool(window, "task_done", args());
    let (verify, _) = ops_in(&effects, "VerifyDone")[0].clone();
    fx.turn_completed(window);
    exited(&mut fx, window);
    let effects = fx.done(
        verify,
        OpResult::Failed {
            message: "git".into(),
        },
    );
    let resumes = resume_messages(&effects);
    assert_eq!(resumes.len(), 1, "{effects:#?}");
    assert!(resumes[0].contains("could not be checked: git"));
    assert_alive(&fx);
}

/// A rejection while the claiming turn is still open is the tool reply alone.
#[test]
fn a_rejection_in_the_open_turn_is_only_the_reply() {
    let (mut fx, window) = working_on(ROOMY);
    let effects = fx.tool(window, "task_done", args());
    let (verify, _) = ops_in(&effects, "VerifyDone")[0].clone();
    let result = dirty(&fx);
    let effects = fx.done(verify, result);
    assert!(replies(&effects)[0].is_err());
    assert!(fx.run().outbox.is_empty(), "{:#?}", fx.run().outbox);
}

/// Probe P3 (T12-N4): the fallback's claim, rejected, sends its rejection and nothing
/// else: no second count, no `DONE_NUDGE` on top.
#[test]
fn a_rejected_fallback_claim_sends_only_the_rejection() {
    let (mut fx, window) = working_on(ROOMY);
    fx.turn_completed(window);
    let (op, _) = fx.op("CountCommits");
    fx.done(op, commits(2));
    let effects = fx.turn_completed(window);
    let (verify, _) = ops_in(&effects, "VerifyDone")[0].clone();
    let result = dirty(&fx);
    let effects = fx.done(verify, result);
    assert!(ops_in(&effects, "CountCommits").is_empty(), "{effects:#?}");
    let rejection = "[anthrex] task_done rejected: the tracked tree has uncommitted changes (1 files); commit or revert them first";
    assert_eq!(
        delivers(&effects),
        vec![rejection.to_string()],
        "{effects:#?}"
    );
    assert_alive(&fx);
}

/// T12-later: the stall clock does not run while a `task_done` check is in flight, and
/// starts again from the verdict.
#[test]
fn the_stall_clock_waits_for_a_task_done_check() {
    let (mut fx, window) = working_on(ROOMY);
    let stall_after = fx.run().limits.stall_after_secs;
    let effects = fx.tool(window, "task_done", args());
    let (verify, _) = ops_in(&effects, "VerifyDone")[0].clone();
    let quiet = fx.task("t1").rounds[0].last_event;
    let effects = fx.send(quiet + stall_after + 5, EventKind::Tick);
    assert!(
        !effects.contains(&Effect::Interrupt { window_id: window }),
        "{effects:#?}"
    );
    assert_eq!(fx.task("t1").rounds[0].stall, StallState::Watching);
    let result = dirty(&fx);
    fx.done(verify, result);
    let verdict = fx.now;
    let effects = fx.send(verdict + stall_after - 1, EventKind::Tick);
    assert!(!effects.contains(&Effect::Interrupt { window_id: window }));
    let effects = fx.send(verdict + stall_after, EventKind::Tick);
    assert!(effects.contains(&Effect::Interrupt { window_id: window }));
}

/// Probe P4 (T12-later): a Codex turn interrupted before its session had an id has
/// nothing to resume: rung 2, with the nudge at the end of the fresh session's prompt.
#[test]
fn a_codex_interrupt_before_its_session_id_is_rung_two() {
    let (mut fx, window) = working_on(CODEX_ROOMY);
    let stall_after = fx.run().limits.stall_after_secs;
    let quiet = fx.task("t1").rounds[0].last_event;
    let effects = fx.send(quiet + stall_after, EventKind::Tick);
    assert!(effects.contains(&Effect::Interrupt { window_id: window }));
    let effects = exited(&mut fx, window);
    assert!(delivers(&effects).is_empty(), "{effects:#?}");
    assert!(ops_in(&effects, "ResumeSession").is_empty());
    assert_eq!(fx.task("t1").rung, 2);
    let (diff, _) = ops_in(&effects, "DiffSoFar")[0].clone();
    let effects = fx.done(
        diff,
        OpResult::Diff {
            stat: String::new(),
            patch: String::new(),
        },
    );
    let creates = ops_in(&effects, "CreateWindow");
    let OpKind::CreateWindow { first_turn, .. } = &creates[0].1 else {
        unreachable!()
    };
    assert!(
        first_turn.ends_with(&stall_nudge(stall_after / 60)),
        "{first_turn}"
    );
    fx.complete_windows();
    assert_alive(&fx);
}

/// T12-N: results that come back after rung 2 killed the session but before the fresh
/// one starts are dropped too: the replaced session's resume and count put nothing in
/// the fresh session's prompt.
#[test]
fn results_between_the_kill_and_the_fresh_session_are_dropped() {
    let (mut fx, window) = working_on(FIVE_MIN);
    let resume = resumed_once(&mut fx, window);
    let started = fx.task("t1").rounds[0].started_at;
    fx.send(started + 450, EventKind::Tick);
    fx.done(
        resume,
        OpResult::ResumeFailed {
            error: "late".into(),
        },
    );
    assert_eq!(fx.task("t1").fresh_session.as_ref().unwrap().append, None);

    let (mut fx, window) = working_on(FIVE_MIN);
    fx.turn_completed(window);
    let (count, _) = fx.op("CountCommits");
    let started = fx.task("t1").rounds[0].started_at;
    fx.send(started + 450, EventKind::Tick);
    fx.done(count, commits(2));
    assert!(fx.run().outbox.is_empty(), "{:#?}", fx.run().outbox);
    assert_eq!(fx.task("t1").fresh_session.as_ref().unwrap().append, None);
}

/// T12-N: a session that died with nothing to resume is superseded by the fresh one:
/// the delivery that was in flight to it, failing late, leaves the fresh session alone.
#[test]
fn a_dead_sessions_late_delivery_failure_leaves_the_fresh_session_alone() {
    let (mut fx, window) = working_on(CODEX_ROOMY);
    fx.turn_completed(window);
    let (op, _) = fx.op("CountCommits");
    let effects = fx.done(op, commits(2));
    let ids = effects
        .iter()
        .find_map(|e| match e {
            Effect::Deliver { message_ids, .. } => Some(message_ids.clone()),
            _ => None,
        })
        .expect("the nudge is delivered");
    let effects = exited(&mut fx, window);
    let (diff, _) = ops_in(&effects, "DiffSoFar")[0].clone();
    fx.done(
        diff,
        OpResult::Diff {
            stat: String::new(),
            patch: String::new(),
        },
    );
    let new = fx.complete_windows()[0].1;
    fx.next(EventKind::Delivered {
        run_id: RUN_ID.into(),
        message_ids: ids,
        ok: false,
        error: Some("gone".into()),
    });
    let last = fx.task("t1").rounds.last().unwrap();
    assert_eq!(last.window_id, Some(new));
    assert!(last.turn_open, "{last:#?}");
    assert_eq!(last.delivery_failures, 0);
}

/// T12-N: while the fallback's count is awaited, another turn end does not start a
/// second one. The awaited count, now an earlier turn's, is dropped and the fallback
/// counts again for the later turn (ruling T12-R4); `NO_COMMIT_NUDGE` was read, so
/// that count's 0 is the stall.
#[test]
fn one_count_at_a_time() {
    let (mut fx, window) = working_on(ROOMY);
    fx.turn_completed(window);
    let (op, _) = fx.op("CountCommits");
    fx.done(op, commits(0));
    let effects = fx.turn_completed(window);
    let (second, _) = ops_in(&effects, "CountCommits")[0].clone();
    queue(&mut fx, "[anthrex] meanwhile");
    fx.tick();
    let effects = fx.turn_completed(window);
    assert!(ops_in(&effects, "CountCommits").is_empty(), "{effects:#?}");
    let effects = fx.done(second, commits(0));
    assert_eq!(
        fx.task("t1").stalls,
        0,
        "an earlier turn's count is dropped"
    );
    let (third, _) = ops_in(&effects, "CountCommits")[0].clone();
    fx.done(third, commits(0));
    assert_eq!(fx.task("t1").rung, 2, "the second empty count is a stall");
}

/// T12-N: the resume after a mid-turn exit is awaited like any other: its failure
/// makes the session final and starts a fresh one.
#[test]
fn a_failed_resume_after_a_mid_turn_exit_starts_a_fresh_session() {
    let (mut fx, window) = working_on(ROOMY);
    let effects = exited(&mut fx, window);
    let (resume, _) = ops_in(&effects, "ResumeSession")[0].clone();
    let effects = fx.done(
        resume,
        OpResult::ResumeFailed {
            error: "gone".into(),
        },
    );
    assert!(fx.task("t1").rounds[0].retiring);
    assert_eq!(ops_in(&effects, "DiffSoFar").len(), 1, "{effects:#?}");
}

/// T12-N: a restart awaits no count; the next turn end counts again.
#[test]
fn a_restore_awaits_no_count() {
    let (mut fx, window) = working_on(ROOMY);
    fx.turn_completed(window);
    let run = fx.run().clone();
    let mut restored = Fixture::new(&fx.plan);
    restored.next(EventKind::Restore {
        runs: vec![run],
        replay: vec![],
    });
    restored.run_mut().state = proto::RunState::Running;
    let effects = restored.turn_completed(window);
    assert_eq!(ops_in(&effects, "CountCommits").len(), 1, "{effects:#?}");
}
