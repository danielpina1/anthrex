//! Milestone 9.1 task M9.1.14, controller ruling C-18: an executor failure of tier 3
//! is retried with backoff and then held for `run resume`, never marked red; the
//! `finish` attention names only tests that ran on the red commit; a setup failure's
//! line; a cancel with a red stage; one tier-3 job per run; a stale red wakes nobody.

use proto::{PlanEdit, RunState};

use super::control::resume;
use super::dispatch::{edit, replies};
use super::fixture::*;
use super::full::{
    attention, block, full_job, full_jobs, later, merge_tiered, outcome, profile,
    red_at_completion, tier, verify_ok,
};
use super::merge::{commit, doc_task, pending, start_on, window_of};
use super::wake_notes::notes;
use crate::run::engine::stages::set_stage_head;
use crate::run::engine::{Effect, EventKind, OpResult};

const LOCK: &str = "git: could not lock the index";

fn failed() -> OpResult {
    OpResult::Failed {
        message: format!("{LOCK}\nsecond line"),
    }
}

/// A tiered run whose one task merged at `commit(1)`, its completion's tier 3 in
/// flight.
fn completing() -> Fixture {
    let (mut fx, windows) = start_on(&profile(), &[doc_task("t1", "")]);
    merge_tiered(&mut fx, "t1", window_of(&windows, "t1"), &commit(1));
    verify_ok(&mut fx);
    full_job(&fx);
    fx
}

/// Answers the pending `VerifyRefs`, if any, and returns the pending tier-3 jobs.
fn verify_then_jobs(fx: &mut Fixture) -> usize {
    if !pending(fx, "VerifyRefs", None).is_empty() {
        verify_ok(fx);
    }
    pending(fx, "Tier", None).len()
}

/// Gives the run an orchestrator (so wake notes are kept), its notes empty.
fn with_orchestrator(fx: &mut Fixture) {
    let orch = super::orch::launched(false).run().orch.orchestrator.clone();
    assert!(orch.is_some());
    fx.run_mut().orch.orchestrator = orch;
    super::wake_notes::clear(fx);
}

fn held_line() -> String {
    format!("stage 1: could not run tier 3 ({LOCK}); anthrex run resume retries")
}

#[test]
fn a_failed_tier3_is_retried_after_a_backoff_and_the_run_completes() {
    let mut fx = completing();
    let (op, _) = full_job(&fx);
    fx.done(op, failed());
    let at = fx.now;
    let full = fx.run().stage(1).unwrap().full.clone();
    assert_eq!(full.red_at, None, "an executor failure is not red");
    assert_eq!(full.infra.as_ref().map(|i| i.count), Some(1));
    assert!(fx.run().pending_ops.is_empty());
    // Within the backoff: nothing starts, not even the guard.
    let effects = fx.send(at + 29, EventKind::Tick);
    assert!(
        effects.iter().all(|e| !matches!(e, Effect::Op { .. })),
        "{effects:#?}"
    );
    // After 30 s the next pass starts it again.
    fx.send(at + 30, EventKind::Tick);
    assert_eq!(verify_then_jobs(&mut fx), 1);
    let (op, spec) = full_job(&fx);
    assert_eq!(spec.head, commit(1));
    fx.done(op, tier(outcome(3, &[])));
    assert_eq!(fx.run().stage(1).unwrap().full.infra, None);
    verify_ok(&mut fx);
    assert_eq!(fx.run().state, RunState::Complete);
    assert!(!fx.run().final_check_failed);
}

#[test]
fn three_failures_hold_the_run_and_resume_retries() {
    let mut fx = completing();
    for (k, wait) in [(1, 30), (2, 120)] {
        let (op, _) = full_job(&fx);
        fx.done(op, failed());
        let at = fx.now;
        fx.send(at + wait - 1, EventKind::Tick);
        assert_eq!(pending(&fx, "Tier", None).len(), 0, "failure {k}");
        assert!(pending(&fx, "VerifyRefs", None).is_empty(), "failure {k}");
        fx.send(at + wait, EventKind::Tick);
        assert_eq!(verify_then_jobs(&mut fx), 1, "retry after failure {k}");
    }
    let (op, _) = full_job(&fx);
    fx.done(op, failed());
    // Held: nothing retries, however long, and the stage is not red.
    let effects = later(&mut fx, 100_000);
    assert!(
        effects.iter().all(|e| !matches!(e, Effect::Op { .. })),
        "{effects:#?}"
    );
    assert_eq!(fx.run().state, RunState::Running);
    let full = fx.run().stage(1).unwrap().full.clone();
    assert_eq!(full.red_at, None);
    assert_eq!(full.infra.as_ref().map(|i| i.count), Some(3));
    assert!(
        attention(&fx).contains(&held_line()),
        "{:?}",
        attention(&fx)
    );
    super::turns_fixes::assert_alive(&fx);

    // `run resume` resets the count and retries.
    let effects = resume(&mut fx);
    assert!(matches!(&replies(&effects)[..], [Ok(_)]), "{effects:#?}");
    assert_eq!(fx.run().stage(1).unwrap().full.infra, None);
    assert_eq!(verify_then_jobs(&mut fx), 1);
    assert!(!attention(&fx).contains(&held_line()));
}

#[test]
fn a_failed_tier3_sends_the_orchestrator_no_note() {
    let (mut fx, windows) = start_on(&profile(), &[doc_task("t1", ""), doc_task("t2", "")]);
    block(&mut fx, "t2", window_of(&windows, "t2"));
    merge_tiered(&mut fx, "t1", window_of(&windows, "t1"), &commit(1));
    let since = fx.run().queue_idle_since.unwrap();
    let (op, _) = full_jobs(&fx.send(since + 120, EventKind::Tick))[0].clone();
    with_orchestrator(&mut fx);
    fx.done(op, failed());
    assert_eq!(notes(&fx), Vec::<String>::new());
    // Nor when it is held after the third.
    for wait in [30, 120] {
        fx.send(fx.now + wait, EventKind::Tick);
        let (op, _) = full_job(&fx);
        fx.done(op, failed());
    }
    assert_eq!(
        fx.run()
            .stage(1)
            .unwrap()
            .full
            .infra
            .as_ref()
            .map(|i| i.count),
        Some(3)
    );
    assert_eq!(notes(&fx), Vec::<String>::new());
}

#[test]
fn finish_attention_names_only_tests_that_ran_on_the_red_commit() {
    // Red at C1 on `a::works`; the head moves to C2, whose tier 3 cannot set up.
    let mut fx = red_at_completion();
    set_stage_head(fx.run_mut(), 1, &commit(2));
    fx.tick();
    verify_then_jobs(&mut fx);
    let (op, spec) = full_job(&fx);
    assert_eq!(spec.head, commit(2));
    fx.done(
        op,
        OpResult::SetupFailed {
            output: "npm ERR! missing script".into(),
        },
    );
    let full = fx.run().stage(1).unwrap().full.clone();
    assert_eq!(full.red_at, Some(commit(2)));
    assert_eq!(full.last, None, "a job that did not run leaves no record");
    edit(&mut fx, vec![PlanEdit::Finish]);
    verify_ok(&mut fx);
    assert_eq!(fx.run().state, RunState::Complete);
    let lines = attention(&fx);
    assert!(!lines.iter().any(|l| l.contains("a::works")), "{lines:?}");
    assert!(
        lines.contains(
            &"stage 1: could not run tier 3: setup failed: npm ERR! missing script".to_string()
        ),
        "{lines:?}"
    );

    // The record of the red commit itself is named.
    let mut fx = red_at_completion();
    assert_eq!(
        fx.run()
            .stage(1)
            .unwrap()
            .full
            .last
            .as_ref()
            .map(|t| t.commit.clone()),
        Some(commit(1))
    );
    edit(&mut fx, vec![PlanEdit::Finish]);
    verify_ok(&mut fx);
    assert!(attention(&fx).contains(&"tier 3 red on stage 1: a::works".to_string()));
}

#[test]
fn a_setup_failure_line_is_its_first_non_empty_line_capped() {
    let mut fx = completing();
    let (op, _) = full_job(&fx);
    let long = "x".repeat(300);
    fx.done(
        op,
        OpResult::SetupFailed {
            output: format!("\n   \n{long}\nmore"),
        },
    );
    let note = fx.run().stage(1).unwrap().full.note.clone().unwrap();
    assert_eq!(
        note,
        format!(
            "stage 1: could not run tier 3: setup failed: {}",
            "x".repeat(200)
        )
    );
    assert!(attention(&fx).contains(&note));
}

#[test]
fn a_cancel_with_a_red_stage_completes_the_run() {
    let mut fx = red_at_completion();
    let reply = fx.reply();
    fx.next(EventKind::Cancel {
        reply,
        run_id: RUN_ID.into(),
    });
    verify_ok(&mut fx);
    assert_eq!(fx.run().state, RunState::Complete);
    assert!(fx.run().final_check_failed);
}

#[test]
fn an_idle_job_during_the_guard_is_the_only_tier3_job() {
    let (mut fx, windows) = start_on(&profile(), &[doc_task("t1", "")]);
    merge_tiered(&mut fx, "t1", window_of(&windows, "t1"), &commit(1));
    assert_eq!(pending(&fx, "VerifyRefs", None).len(), 1);
    let since = fx.run().queue_idle_since.unwrap();
    // The idle job starts while the completion guard is pending.
    let jobs = full_jobs(&fx.send(since + 120, EventKind::Tick));
    assert_eq!(jobs.len(), 1);
    let effects = verify_ok(&mut fx);
    assert!(full_jobs(&effects).is_empty(), "{effects:#?}");
    assert_eq!(pending(&fx, "Tier", None).len(), 1);
    assert_eq!(fx.run().full_op, Some(jobs[0].0));
    // Its green result completes the run through the next guard.
    fx.done(jobs[0].0, tier(outcome(3, &[])));
    verify_ok(&mut fx);
    assert_eq!(fx.run().state, RunState::Complete);
}

#[test]
fn a_stale_red_result_is_recorded_without_a_wake_note() {
    let (mut fx, windows) = start_on(&profile(), &[doc_task("t1", ""), doc_task("t2", "")]);
    block(&mut fx, "t2", window_of(&windows, "t2"));
    merge_tiered(&mut fx, "t1", window_of(&windows, "t1"), &commit(1));
    let since = fx.run().queue_idle_since.unwrap();
    let (op, _) = full_jobs(&fx.send(since + 120, EventKind::Tick))[0].clone();
    with_orchestrator(&mut fx);
    // The head moves while the job runs on commit(1).
    set_stage_head(fx.run_mut(), 1, &commit(2));
    fx.done(op, tier(outcome(3, &["a::works"])));
    let full = fx.run().stage(1).unwrap().full.clone();
    assert_eq!(full.red_at, Some(commit(1)));
    assert_eq!(full.last.map(|t| t.commit), Some(commit(1)));
    assert_eq!(notes(&fx), Vec::<String>::new());

    // A red on the current head does wake it.
    let (op, _) = full_job(&fx);
    fx.done(op, tier(outcome(3, &["a::works"])));
    assert_eq!(notes(&fx).len(), 1, "{:?}", notes(&fx));
}
