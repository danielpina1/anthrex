//! Milestone 9.0.5 task 3, ruling D-1: a task's detail shows a `task_done` summary only
//! while the session that claimed it is the task's latest worker session. `Task.done`
//! is never cleared (the gates read it), so after rung 2's fresh session the old claim
//! would otherwise stand in for the new session's work.

use proto::{SummarySource, TaskState};
use serde_json::json;

use super::fixture::*;
use super::gates::{CHECK_MODE, accepted, check_result, only_op, working_on};
use super::gates_review::{blocking, reviewer, submit, verdict};
use super::turns::killed_exit;
use crate::run::engine::{AgentSignal, Effect, OpResult};
use crate::run::snapshot_detail::task_detail;

fn summary(fx: &Fixture) -> (Option<String>, Option<SummarySource>) {
    let d = task_detail(fx.run(), "t1").expect("t1 exists");
    (d.worker_summary, d.summary_source)
}

fn said(fx: &mut Fixture, window: u32, text: &str) {
    fx.signal(window, AgentSignal::Said { text: text.into() });
}

/// Rung 2 killed `old`: its exit, the diff so far, and the fresh session's window.
fn fresh_session(fx: &mut Fixture, old: u32) -> u32 {
    let effects = killed_exit(fx, old);
    let (op, _) = only_op(&effects, "DiffSoFar");
    fx.done(
        op,
        OpResult::Diff {
            stat: String::new(),
            patch: String::new(),
        },
    );
    let windows = fx.complete_windows();
    assert_eq!(windows.len(), 1, "{windows:?}");
    windows[0].1
}

fn kills(effects: &[Effect], window: u32) -> bool {
    effects.contains(&Effect::KillWindow { window_id: window })
}

/// The reviewer's scenario: session 1 claims `S1`; two failed checks reach rung 2,
/// whose fresh session closes a turn with text and no claim yet.
#[test]
fn a_fresh_sessions_detail_drops_the_old_claim() {
    let (mut fx, window) = working_on(PROFILE, CHECK_MODE);
    for _ in 0..2 {
        let effects = accepted(&mut fx, window, json!({"summary": "S1"}));
        let (op, _) = only_op(&effects, "Check");
        let effects = fx.done(op, check_result(false));
        if kills(&effects, window) {
            break;
        }
        assert_eq!(
            summary(&fx),
            (Some("S1".into()), Some(SummarySource::TaskDone))
        );
    }
    assert_eq!(fx.task("t1").rung, 2);
    assert_eq!(fx.task("t1").done.as_ref().unwrap().session, Some(1));

    let second = fresh_session(&mut fx, window);
    // The fresh session is mid-turn: nothing closed is its, and the old claim is not.
    assert_eq!(summary(&fx), (None, None));
    said(&mut fx, second, "session 2's words");
    fx.turn_completed(second);
    assert_eq!(
        summary(&fx),
        (
            Some("session 2's words".into()),
            Some(SummarySource::LastMessage)
        )
    );
    // Its own claim is current again.
    accepted(&mut fx, second, json!({"summary": "S2"}));
    assert_eq!(
        summary(&fx),
        (Some("S2".into()), Some(SummarySource::TaskDone))
    );
}

/// A review rework. Rung 1 sends the findings to the same session, whose claim stays
/// current until it claims again (D-1 compares sessions); rung 2's fresh session drops
/// it.
#[test]
fn a_review_rework_shows_the_current_sessions_account() {
    let (mut fx, window) = working_on(PROFILE, CHECK_MODE);
    let claim = |fx: &mut Fixture, window: u32, text: &str| {
        let effects = accepted(fx, window, json!({ "summary": text }));
        let (op, _) = only_op(&effects, "Check");
        let effects = fx.done(op, check_result(true));
        assert_eq!(fx.task("t1").state, TaskState::Review);
        let (op, _) = only_op(&effects, "PrepareReview");
        reviewer(fx, op, "diff --git a/x b/x").0
    };
    let rwindow = claim(&mut fx, window, "S1");
    submit(&mut fx, rwindow, verdict("changes", blocking()));
    assert_eq!(fx.task("t1").rung, 1);
    // The retired reviewer's process ends (T13-I2: round 2 waits for it).
    super::turns::exited(&mut fx, rwindow);
    said(&mut fx, window, "Fixing the findings");
    fx.turn_completed(window);
    assert_eq!(
        summary(&fx),
        (Some("S1".into()), Some(SummarySource::TaskDone))
    );

    let rwindow = claim(&mut fx, window, "S1 again, findings fixed");
    assert_eq!(
        summary(&fx),
        (
            Some("S1 again, findings fixed".into()),
            Some(SummarySource::TaskDone)
        )
    );
    let effects = submit(&mut fx, rwindow, verdict("changes", blocking()));
    assert!(kills(&effects, window), "{effects:#?}");
    assert_eq!(fx.task("t1").rung, 2);
    let second = fresh_session(&mut fx, window);
    said(&mut fx, second, "Reworking from scratch");
    fx.turn_completed(second);
    assert_eq!(
        summary(&fx),
        (
            Some("Reworking from scratch".into()),
            Some(SummarySource::LastMessage)
        )
    );
}

/// A claim from a `run.json` written before D-1 (no `session`) counts as current.
#[test]
fn an_old_claim_without_a_session_is_current() {
    let (mut fx, window) = working_on(PROFILE, CHECK_MODE);
    accepted(&mut fx, window, json!({"summary": "S1"}));
    let mut done = serde_json::to_value(fx.task("t1").done.as_ref().unwrap()).unwrap();
    assert!(done.as_object_mut().unwrap().remove("session").is_some());
    let old = serde_json::from_value(done).expect("a claim without a session loads");
    let t1 = fx.task_mut("t1");
    t1.done = Some(old);
    assert_eq!(t1.done.as_ref().unwrap().session, None);
    // Even with a later worker session, an unknown session keeps the old behaviour.
    let mut later = t1.rounds[0].clone();
    later.session = 2;
    later.round = 2;
    t1.rounds.push(later);
    assert_eq!(
        summary(&fx),
        (Some("S1".into()), Some(SummarySource::TaskDone))
    );
}
