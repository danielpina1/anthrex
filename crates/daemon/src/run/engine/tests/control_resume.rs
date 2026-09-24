//! M8a.15: what a resume must not do, and what a restart must not lose. A lost resume,
//! message or override count is sent or asked again; a paused session is not stalled by
//! the pause; only a restart's resume sends the restart's messages, and never to a
//! session that was replaced or whose verdict is in; a launch the task no longer wants
//! is not re-issued. Split from `control_restore.rs` for size. Every sequence ends with
//! the liveness check.

use proto::{PlanEdit, RunState, TaskState};

use super::control::{blocked, override_task, resume, retry};
use super::control_restore::{restart, resumes};
use super::dispatch::edit;
use super::done::one_reply;
use super::fixture::*;
use super::gates::{CHECK_MODE, only_op, working_on};
use super::gates_review::{CODEX_AUTHOR, in_review, reviewed, submit, verdict};
use super::liveness::assert_alive;
use super::merge::{doc_task, start_on, window_of};
use super::turns::{exited, queue, working};
use crate::headless::FailureKind;
use crate::run::contract::{RESUME_REVIEWER, RESUME_WORKER};
use crate::run::engine::{AgentSignal, Effect, EventKind, OpResult, TurnOutcome};
use crate::run::model::BaseMoved;

fn interrupts(effects: &[Effect]) -> usize {
    effects
        .iter()
        .filter(|e| matches!(e, Effect::Interrupt { .. }))
        .count()
}

fn queued(fx: &Fixture, text: &str) -> bool {
    fx.run().outbox.iter().any(|m| m.text == text)
}

/// A message delivered but not confirmed when the daemon stopped is delivered again.
#[test]
fn a_message_in_flight_at_the_restart_is_delivered_again() {
    let (mut fx, window) = working();
    fx.turn_completed(window);
    queue(&mut fx, "[anthrex] X");
    fx.tick();
    assert!(fx.run().outbox.iter().all(|m| m.delivered_at.is_some()));
    restart(&mut fx, Vec::new());
    let effects = resume(&mut fx);
    let texts: Vec<String> = resumes(&effects).into_iter().map(|(_, _, m)| m).collect();
    assert!(
        texts.iter().any(|t| t.contains("[anthrex] X")),
        "{effects:#?}"
    );
    assert_alive(&fx);
}

/// An override whose count the restart lost can be asked again.
#[test]
fn an_override_count_lost_to_the_restart_can_be_asked_again() {
    let (mut fx, window) = working();
    blocked(&mut fx, window, "question", "which table?");
    let effects = override_task(&mut fx, "t1", "trust me");
    let (lost, _) = only_op(&effects, "CountCommits");
    restart(&mut fx, Vec::new());
    resume(&mut fx);
    let effects = override_task(&mut fx, "t1", "trust me");
    let (op, _) = only_op(&effects, "CountCommits");
    assert_ne!(op, lost);
    assert!(fx.run().pending_ops.contains_key(&op));
    assert_alive(&fx);
}

/// A resume in flight when the daemon stopped again is sent again.
#[test]
fn a_resume_lost_to_a_second_restart_is_sent_again() {
    let (mut fx, _) = working();
    restart(&mut fx, Vec::new());
    let effects = resume(&mut fx);
    let (lost, _) = only_op(&effects, "ResumeSession");
    restart(&mut fx, Vec::new());
    let effects = resume(&mut fx);
    let (op, _) = only_op(&effects, "ResumeSession");
    assert_ne!(op, lost);
    assert_alive(&fx);
}

/// The stall clock does not run while a run is paused: its first stage is re-armed by
/// the resume, so a session silent through the pause is not interrupted for it, and
/// the pause is no session time (decision 40's minutes): a long one breaches no budget.
#[test]
fn a_paused_session_is_not_stalled_by_the_pause() {
    let (mut fx, _) = working();
    edit(&mut fx, vec![PlanEdit::Pause]);
    let later = fx.now + 10 * fx.run().limits.stall_after_secs;
    assert_eq!(interrupts(&fx.send(later, EventKind::Tick)), 0);
    let effects = resume(&mut fx);
    assert_eq!(fx.run().state, RunState::Running);
    let t1 = fx.task("t1");
    assert_eq!(
        (t1.state, t1.rung),
        (TaskState::Working, 0),
        "{:?}",
        t1.history.last()
    );
    assert_eq!(interrupts(&effects), 0, "{effects:#?}");
    assert_eq!(interrupts(&fx.tick()), 0);
    assert_alive(&fx);
}

/// A pause edit's resume is no restart: a worker whose process exited between turns
/// gets no restart message.
#[test]
fn a_pause_resume_sends_no_restart_message() {
    let (mut fx, window) = working();
    fx.turn_completed(window);
    exited(&mut fx, window);
    assert!(fx.task("t1").rounds[0].ended);
    edit(&mut fx, vec![PlanEdit::Pause]);
    let effects = resume(&mut fx);
    assert!(resumes(&effects).is_empty(), "{effects:#?}");
    assert!(!queued(&fx, RESUME_WORKER));
    assert_alive(&fx);
}

/// One live session per role: a retried task's old session, ended between turns and
/// still resumable, is not resumed beside the fresh one after a restart, and the fresh
/// session's prompt carries no restart message.
#[test]
fn a_retried_task_s_old_session_is_not_resumed_after_a_restart() {
    // t2's live session makes this a restart that ended sessions.
    let (mut fx, windows) = start_on(PROFILE, &[doc_task("t1", ""), doc_task("t2", "")]);
    let window = window_of(&windows, "t1");
    blocked(&mut fx, window, "question", "which table?");
    fx.turn_completed(window);
    exited(&mut fx, window);
    let effects = retry(&mut fx, "t1");
    assert!(one_reply(&effects).is_ok(), "{effects:#?}");
    assert!(fx.task("t1").fresh_session.is_some());
    restart(&mut fx, Vec::new());
    let effects = resume(&mut fx);
    let resumed: Vec<u32> = resumes(&effects).into_iter().map(|(w, _, _)| w).collect();
    assert_eq!(resumed, vec![window_of(&windows, "t2")], "{effects:#?}");
    assert!(
        !fx.run()
            .outbox
            .iter()
            .any(|m| m.task_id == "t1" && m.text == RESUME_WORKER)
    );
    let fresh = fx.task("t1").fresh_session.clone().unwrap();
    assert_eq!(
        fresh.append, None,
        "a fresh session gets no restart message"
    );
    let (_, kind) = only_op(&effects, "DiffSoFar");
    assert!(format!("{kind:?}").contains("t1"), "{kind:?}");
    assert_alive(&fx);
}

/// A launch the restart lost is not re-issued for a task cancelled while paused.
#[test]
fn a_lost_launch_of_a_task_cancelled_while_paused_is_not_relaunched() {
    let mut fx = Fixture::new(&plan_with(PROFILE, &[task("t1", "S", "a", "")]));
    fx.ready(true);
    fx.complete_prepares();
    fx.op("CreateWindow");
    restart(&mut fx, Vec::new());
    edit(
        &mut fx,
        vec![PlanEdit::CancelTask {
            task_id: "t1".into(),
        }],
    );
    let effects = resume(&mut fx);
    assert!(ops_in(&effects, "CreateWindow").is_empty(), "{effects:#?}");
    assert_eq!(fx.task("t1").state, TaskState::Cancelled);
    assert!(fx.task("t1").rounds.iter().all(|r| r.ended));
    assert_alive(&fx);

    // A reviewer's launch, for a task cancelled in review.
    let (mut fx, window) = working_on(PROFILE, CHECK_MODE);
    let (op, _) = in_review(&mut fx, window);
    let effects = fx.done(
        op,
        OpResult::Review {
            base: BASE.into(),
            head: HEAD.into(),
            patch: "diff".into(),
        },
    );
    only_op(&effects, "CreateWindow");
    restart(&mut fx, Vec::new());
    edit(
        &mut fx,
        vec![PlanEdit::CancelTask {
            task_id: "t1".into(),
        }],
    );
    let effects = resume(&mut fx);
    assert!(ops_in(&effects, "CreateWindow").is_empty(), "{effects:#?}");
    assert!(fx.task("t1").rounds.iter().all(|r| r.ended));
    assert_alive(&fx);
}

/// `run resume --rebaseline` of a paused run records the refs read and clears an
/// advance of the base recorded before the restart (`base_moved` is in `run.json`).
#[test]
fn a_paused_rebaseline_clears_the_recorded_advance() {
    let (mut fx, _) = working();
    fx.run_mut().base_moved = Some(BaseMoved {
        from: BASE.into(),
        to: "b2".repeat(20),
        commits: 1,
        seen_at: fx.now,
    });
    restart(&mut fx, Vec::new());
    let (base, head) = ("b2".repeat(20), "c3".repeat(20));
    let reply = fx.reply();
    let effects = fx.next(EventKind::Resume {
        reply,
        run_id: RUN_ID.into(),
        rebaseline: Some((base.clone(), head.clone())),
    });
    assert!(one_reply(&effects).is_ok(), "{effects:#?}");
    let run = fx.run();
    assert_eq!(
        (&run.base_sha, &run.run_head, &run.base_moved),
        (&base, &head, &None)
    );
    assert_alive(&fx);
}

/// A reviewer whose verdict is in is not resumed after a restart.
#[test]
fn a_reviewer_whose_verdict_is_in_is_not_resumed() {
    let (mut fx, _, rwindow) = reviewed(PROFILE, "");
    fx.signal(
        rwindow,
        AgentSignal::Init {
            session_id: "s-r".into(),
        },
    );
    let effects = submit(&mut fx, rwindow, verdict("approve", vec![]));
    assert!(one_reply(&effects).is_ok(), "{effects:#?}");
    assert_ne!(fx.task("t1").state, TaskState::Review);
    restart(&mut fx, Vec::new());
    let effects = resume(&mut fx);
    assert!(
        resumes(&effects)
            .iter()
            .all(|(_, _, m)| m != RESUME_REVIEWER),
        "{effects:#?}"
    );
    assert!(!queued(&fx, RESUME_REVIEWER));
    assert_alive(&fx);
}

/// A Claude reviewer whose process exited between turns, while it waits out a rate
/// limit, still owes its verdict and holds its reader slot: no second reviewer starts
/// beside it, and a pause edit's resume sends it no restart message.
#[test]
fn a_reviewer_ended_between_turns_is_neither_replaced_nor_told_of_a_restart() {
    let (mut fx, _, rwindow) = reviewed(PROFILE, CODEX_AUTHOR);
    let outcome = TurnOutcome::Failed {
        error: "rate limit reached".into(),
        kind: FailureKind::RateLimit,
    };
    fx.turn_ended(rwindow, outcome);
    exited(&mut fx, rwindow);
    let round = fx.task("t1").rounds.last().unwrap();
    assert!(round.ended && round.session_id.is_some(), "{round:#?}");
    let effects = fx.tick();
    assert!(ops_in(&effects, "PrepareReview").is_empty(), "{effects:#?}");
    edit(&mut fx, vec![PlanEdit::Pause]);
    let effects = resume(&mut fx);
    assert!(resumes(&effects).is_empty(), "{effects:#?}");
    assert!(ops_in(&effects, "PrepareReview").is_empty(), "{effects:#?}");
    assert!(!queued(&fx, RESUME_REVIEWER));
    assert_alive(&fx);
}
