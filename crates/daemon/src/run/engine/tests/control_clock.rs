//! M8a.15 fix round 1, the task clock (rulings T15-I2, T15-I3): what a stop excuses
//! (minutes and stall silence), when it is recorded, and what the snapshot shows while
//! it lasts. Split from `control_fixes.rs` for size. Every sequence ends with the
//! liveness check.

use proto::{PlanEdit, TaskState};

use super::control::{blocked, resume};
use super::control_restore::restart;
use super::dispatch::edit;
use super::fixture::*;
use super::gates::{check_result, only_op};
use super::holds::{add_dep, answer, blocked_t1};
use super::liveness::assert_alive;
use super::merge::{claim, doc_task, head_of, pending_one, start_on, window_of};
use super::turns::{exited, working, working_on};
use crate::run::engine::ladder::round_spend;
use crate::run::engine::{AgentSignal, Effect, EventKind, OpResult, TurnOutcome};
use crate::run::snapshot::snapshot;

fn interrupted(effects: &[Effect]) -> bool {
    effects
        .iter()
        .any(|e| matches!(e, Effect::Interrupt { .. }))
}

fn session_secs(fx: &Fixture) -> u64 {
    snapshot(&fx.state, fx.now).runs[0].tasks[0]
        .spent_session
        .secs
}

fn state_and_rung(fx: &Fixture) -> (TaskState, u8) {
    let t1 = fx.task("t1");
    (t1.state, t1.rung)
}

/// Ruling T15-R3: a turn open through a check (Claude started it by itself, for a
/// background sub-agent) is watched and charged: once it falls silent it is
/// interrupted, and when it ends the rest of the two-hour check is excused.
#[test]
fn a_turn_open_through_the_gates_is_watched_then_excused() {
    let (mut fx, windows) = start_on(PROFILE, &[doc_task("t1", "")]);
    let window = window_of(&windows, "t1");
    claim(&mut fx, "t1", window, &head_of("t1"));
    fx.signal(window, AgentSignal::TurnStarted);
    let (op, _) = pending_one(&fx, "Check", Some("t1"));
    let stall_after = fx.run().limits.stall_after_secs;
    let effects = fx.send(fx.now + stall_after + 1, EventKind::Tick);
    assert!(interrupted(&effects), "{effects:#?}");
    fx.turn_ended(window, TurnOutcome::Interrupted);
    fx.now += 2 * 3_600;
    fx.done(op, check_result(false));
    let effects = fx.tick();
    assert!(!interrupted(&effects), "{effects:#?}");
    assert_eq!(state_and_rung(&fx), (TaskState::Working, 1));
    let secs = session_secs(&fx);
    assert!((stall_after..stall_after + 120).contains(&secs), "{secs}");
    assert_alive(&fx);
}

/// The downtime before a restore is no session time: the clock stops at the
/// session's last sign of life, not at the restore.
#[test]
fn the_downtime_before_a_restore_is_not_charged() {
    let (mut fx, _) = working();
    let started = fx.task("t1").rounds[0].started_at;
    fx.now += 2 * 3_600;
    restart(&mut fx, Vec::new());
    resume(&mut fx);
    fx.tick();
    assert_eq!(
        state_and_rung(&fx),
        (TaskState::Working, 0),
        "{:?}",
        fx.task("t1").history.last()
    );
    assert!(
        session_secs(&fx) < 60,
        "{} since {started}",
        session_secs(&fx)
    );
    assert_alive(&fx);
}

/// Ruling T15-R3: a question asked with its turn still open is charged until the turn
/// ends (here the watchdog interrupts it once it falls silent); the rest of the
/// two-hour wait is excused, and the answer goes to the same session.
#[test]
fn a_block_is_excused_from_the_end_of_its_turn() {
    let (mut fx, window) = working();
    blocked(&mut fx, window, "question", "which table?");
    assert!(fx.task("t1").rounds[0].turn_open, "the turn is still open");
    let stall_after = fx.run().limits.stall_after_secs;
    let effects = fx.send(fx.now + stall_after + 1, EventKind::Tick);
    assert!(interrupted(&effects), "{effects:#?}");
    fx.turn_ended(window, TurnOutcome::Interrupted);
    fx.now += 2 * 3_600;
    let effects = edit(&mut fx, vec![answer("users")]);
    assert!(!interrupted(&effects), "{effects:#?}");
    assert_eq!(
        state_and_rung(&fx),
        (TaskState::Working, 0),
        "{:?}",
        fx.task("t1").history.last()
    );
    let secs = session_secs(&fx);
    assert!((stall_after..stall_after + 120).contains(&secs), "{secs}");
    assert_alive(&fx);
}

/// A task held inside the scheduler's pass (an answered task that gained a
/// dependency) stops its clock in that pass, not at the next event: the two hours
/// until its hand-back are excused.
#[test]
fn a_hold_made_in_the_pass_stops_the_clock_at_once() {
    let mut fx = blocked_t1();
    edit(&mut fx, vec![add_dep("t1", "t2"), answer("A")]);
    assert_eq!(fx.task("t1").state, TaskState::Blocked, "held");
    fx.now += 2 * 3_600;
    fx.launch_all();
    let effects = fx.merge("t2", &"c2".repeat(20));
    let (op, _) = only_op(&effects, "HandBack");
    fx.done(
        op,
        OpResult::HandedBack {
            files: vec![],
            head: None,
            onto: None,
        },
    );
    fx.tick();
    assert_eq!(
        state_and_rung(&fx),
        (TaskState::Working, 0),
        "{:?}",
        fx.task("t1").history.last()
    );
    assert!(session_secs(&fx) < 60, "{}", session_secs(&fx));
    assert_alive(&fx);
}

/// While the clock is stopped (the task blocked, its turn ended), the snapshot's
/// session spend does not grow.
#[test]
fn a_stopped_clock_shows_no_growing_spend() {
    let (mut fx, window) = working();
    blocked(&mut fx, window, "question", "which table?");
    fx.turn_completed(window);
    let before = session_secs(&fx);
    fx.now += 3_600;
    assert_eq!(session_secs(&fx), before);
    fx.tick();
    assert_eq!(session_secs(&fx), before);
    assert_alive(&fx);
}

/// A session that ended between turns is charged up to the pause, not less, once the
/// run resumes: it counts as ended at the resume until it is resumed itself.
#[test]
fn an_ended_session_keeps_its_time_to_the_pause() {
    let (mut fx, window) = working();
    fx.turn_completed(window);
    exited(&mut fx, window);
    let started = fx.task("t1").rounds[0].started_at;
    fx.now += 100;
    edit(&mut fx, vec![PlanEdit::Pause]);
    let paused = fx.now;
    fx.now += 3_600;
    resume(&mut fx);
    let round = &fx.task("t1").rounds[0];
    assert_eq!(round_spend(round, None, fx.now).secs, paused - started);
    assert_alive(&fx);
}

/// Only the latest session is treated as resumable: an older one that ended between
/// turns before a retry keeps its own end through a stop.
#[test]
fn an_older_ended_session_keeps_its_end() {
    let (mut fx, window) = working();
    blocked(&mut fx, window, "question", "which table?");
    fx.turn_completed(window);
    exited(&mut fx, window);
    let effects = super::control::retry(&mut fx, "t1");
    let (op, _) = only_op(&effects, "DiffSoFar");
    fx.done(
        op,
        OpResult::Diff {
            stat: String::new(),
            patch: String::new(),
        },
    );
    let fresh = fx.complete_windows()[0].1;
    // The fresh session's first turn ends, so the pause stops the clock (T15-R3).
    fx.turn_completed(fresh);
    let old = &fx.task("t1").rounds[0];
    assert!(
        old.ended && !old.retiring && old.session_id.is_some(),
        "{old:#?}"
    );
    let spent = round_spend(old, None, fx.now).secs;
    edit(&mut fx, vec![PlanEdit::Pause]);
    fx.now += 3_600;
    resume(&mut fx);
    assert_eq!(
        round_spend(&fx.task("t1").rounds[0], None, fx.now).secs,
        spent
    );
    assert_alive(&fx);
}

/// Ruling T15-minors (mutant V16): a claim accepted mid-turn leaves no mark on the
/// session the restart ended: after its check fails, the resumed session's turn end
/// runs the fallback.
#[test]
fn a_claim_accepted_before_the_restart_leaves_no_mark_on_the_resumed_turn() {
    let (mut fx, windows) = start_on(PROFILE, &[doc_task("t1", "")]);
    let window = window_of(&windows, "t1");
    // The claim is accepted while its turn is still open.
    let effects = fx.tool(window, "task_done", serde_json::json!({"summary": "done"}));
    let (op, _) = only_op(&effects, "VerifyDone");
    let result = fx.clean_check("t1");
    fx.done(op, result);
    assert!(fx.task("t1").rounds[0].turn_had_task_done);
    restart(&mut fx, Vec::new());
    let effects = resume(&mut fx);
    let (op, _) = only_op(&effects, "Check");
    let effects = fx.done(op, check_result(false));
    let (op, _) = only_op(&effects, "ResumeSession");
    fx.done(op, OpResult::Resumed);
    let effects = fx.turn_completed(window);
    only_op(&effects, "CountCommits");
    assert_alive(&fx);
}

/// Decision 45: a first-stage stall deadline is re-armed from the resume, so silence
/// before a pause does not carry over. (A roomy budget: the open turn is charged
/// through the pause, ruling T15-R3; the pause stays under the next size's ceiling.)
#[test]
fn a_resume_re_arms_the_first_stall_stage_from_now() {
    let (mut fx, _) = working_on("[task.budget]\ntool_calls = 1000\nminutes = 1000");
    let stall_after = fx.run().limits.stall_after_secs;
    fx.send(fx.now + stall_after - 100, EventKind::Tick);
    edit(&mut fx, vec![PlanEdit::Pause]);
    fx.now += 1_800;
    let effects = resume(&mut fx);
    assert!(!interrupted(&effects), "{effects:#?}");
    let effects = fx.send(fx.now + 200, EventKind::Tick);
    assert!(!interrupted(&effects), "{effects:#?}");
    let effects = fx.send(fx.now + stall_after, EventKind::Tick);
    assert!(interrupted(&effects), "{effects:#?}");
    assert_alive(&fx);
}
