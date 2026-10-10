//! Milestone 9.11 decision 7 (A10; pre-flight P13, the user's option (b)): a worker's
//! failed tool result that Claude's sandbox could not run is recorded on its round, as
//! the scout does, and named in the task's next block, stall or failure. It blocks
//! nothing by itself.

use proto::{BlockReason, TaskState};

use super::carried_messages::{done_fails, fresh_first_turn, working_generated};
use super::fixture::*;
use super::gates_support::reviewed;
use super::turns::working;
use crate::headless::FailureKind;
use crate::headless::failure::{sandbox_command_failure, with_sandbox_hint};
use crate::run::engine::{AgentSignal, Effect, EventKind, TurnOutcome};

const ROOMY: &str = "[task.budget]\ntool_calls = 1000\nminutes = 1000";

/// Two different sandbox failures' notes, as `observe::translate` makes them.
fn note(n: u8) -> String {
    let text = match n {
        1 => "bwrap: apply-seccomp: Operation not permitted",
        _ => "setting up uid map: nested userns is capability-restricted",
    };
    sandbox_command_failure(text).expect("a sandbox failure")
}

fn broken(fx: &mut Fixture, window: u32, note: String) -> Vec<Effect> {
    fx.signal(window, AgentSignal::SandboxBroken { note })
}

/// The session in `window` stalls past its interrupt's grace (rung 2 at the first
/// failure, `ladder_roles.rs`'s clock): the step's effects.
fn stall(fx: &mut Fixture, window: u32) -> Vec<Effect> {
    let quiet = fx.task("t1").rounds.last().expect("a session").last_event;
    fx.send(quiet + 601, EventKind::Tick);
    let effects = fx.send(quiet + 632, EventKind::Tick);
    assert!(
        effects.contains(&Effect::KillWindow { window_id: window }),
        "{effects:#?}"
    );
    effects
}

fn stalled_line(fx: &Fixture) -> String {
    (fx.task("t1").history.iter())
        .rev()
        .find(|e| e.text.starts_with("stalled: "))
        .expect("a stall")
        .text
        .clone()
}

fn lines_naming(fx: &Fixture, text: &str) -> usize {
    (fx.task("t1").history.iter())
        .filter(|e| e.text.contains(text))
        .count()
}

#[test]
fn a_sandbox_failure_blocks_nothing() {
    let (mut fx, window) = working();
    let before = fx.task("t1").clone();
    let effects = broken(&mut fx, window, note(1));
    // Nothing is done: no kill, op, delivery or reply. The round's change is saved and
    // published as any stream event's is.
    let saved = |e: &Effect| matches!(e, Effect::Persist { .. } | Effect::Publish { .. });
    assert!(effects.iter().all(saved), "{effects:#?}");
    let t1 = fx.task("t1");
    assert_eq!(
        (t1.state, &t1.block, &t1.history),
        (TaskState::Working, &before.block, &before.history)
    );
    assert_eq!((t1.rung, t1.failures), (before.rung, before.failures));
    assert_eq!(t1.rounds[0].sandbox_note, Some(note(1)));
    assert!(!t1.rounds[0].ended && t1.rounds[0].turn_open);
}

#[test]
fn a_stall_after_a_sandbox_failure_names_it() {
    let (mut fx, window) = working();
    broken(&mut fx, window, note(1));
    stall(&mut fx, window);
    let tail = format!("; {}", note(1));
    let line = stalled_line(&fx);
    assert!(line.ends_with(&tail), "{line}");
    let t1 = fx.task("t1");
    assert_eq!(t1.rung, 2);
    let cause = &t1.fresh_session.as_ref().expect("rung 2").reason;
    assert!(cause.ends_with(&tail), "{cause}");
    assert_eq!(t1.rounds[0].sandbox_note, None, "taken once");
}

#[test]
fn a_block_after_a_sandbox_failure_names_it_once() {
    // Two done-gate failures (rung 2), then the fresh session's sandbox failure and its
    // third failure: rung 3.
    let (mut fx, window) = working_generated(ROOMY);
    done_fails(&mut fx, window);
    done_fails(&mut fx, window);
    fresh_first_turn(&mut fx, window);
    let second = fx.complete_windows()[0].1;
    broken(&mut fx, second, note(1));
    done_fails(&mut fx, second);
    let t1 = fx.task("t1");
    assert_eq!((t1.state, t1.rung), (TaskState::Blocked, 3));
    let block = t1.block.clone().expect("blocked");
    assert_eq!(block.reason, BlockReason::MisSized);
    let expected = with_sandbox_hint(&format!("{RUNG_3_BLOCK}; {}", note(1)));
    assert_eq!(block.text, expected);
    assert_eq!(
        lines_naming(&fx, &note(1)),
        1,
        "{:#?}",
        fx.task("t1").history
    );
}

/// Pinning: today's rung-3 block text, with no sandbox failure recorded.
#[test]
fn a_block_without_a_sandbox_failure_is_unchanged() {
    let (mut fx, window) = working_generated(ROOMY);
    done_fails(&mut fx, window);
    done_fails(&mut fx, window);
    fresh_first_turn(&mut fx, window);
    let second = fx.complete_windows()[0].1;
    done_fails(&mut fx, second);
    let block = fx.task("t1").block.clone().expect("blocked");
    assert_eq!(block.text, RUNG_3_BLOCK);
}

/// The rung-3 block of three done-gate failures, as the engine words it today.
const RUNG_3_BLOCK: &str = "the done gate failed 3 times (3 failures in all); last: [anthrex] task_done rejected: you changed generated files outside this task's owns: Cargo.lock. Revert them (git checkout <start> -- Cargo.lock, then commit), or this task must own them. Then call task_done again.";

#[test]
fn the_first_note_of_a_round_is_kept() {
    let (mut fx, window) = working();
    broken(&mut fx, window, note(1));
    broken(&mut fx, window, note(2));
    assert_ne!(note(1), note(2));
    stall(&mut fx, window);
    let line = stalled_line(&fx);
    assert!(line.ends_with(&format!("; {}", note(1))), "{line}");
    assert!(!line.contains(&note(2)), "{line}");
}

#[test]
fn a_new_round_does_not_inherit_the_note() {
    let (mut fx, window) = working();
    broken(&mut fx, window, note(1));
    stall(&mut fx, window);
    let cause = fx.task("t1").fresh_session.clone().expect("rung 2").reason;
    assert!(
        cause.ends_with(&note(1)),
        "rung 2's cause names it: {cause}"
    );
    let _ = fresh_first_turn(&mut fx, window);
    let second = fx.complete_windows()[0].1;
    // An older round's note, however it is there, is never the new round's.
    fx.task_mut("t1").rounds[0].sandbox_note = Some(note(2));
    stall(&mut fx, second);
    let line = stalled_line(&fx);
    assert!(
        !line.contains(&note(1)) && !line.contains(&note(2)),
        "{line}"
    );
    let t1 = fx.task("t1");
    assert_eq!(t1.rounds[0].sandbox_note, Some(note(2)), "left where it is");
}

#[test]
fn a_reviewers_note_names_its_tasks_block() {
    let (mut fx, _, rwindow) = reviewed(PROFILE, "");
    broken(&mut fx, rwindow, note(2));
    assert_eq!(
        fx.task("t1").state,
        TaskState::Review,
        "nothing blocked yet"
    );
    fx.turn_ended(
        rwindow,
        TurnOutcome::Failed {
            error: "not logged in".into(),
            kind: FailureKind::Authentication,
        },
    );
    let block = fx.task("t1").block.clone().expect("blocked");
    assert_eq!(block.reason, BlockReason::Environment);
    assert_eq!(block.text, format!("not logged in; {}", note(2)));
}

#[test]
fn a_note_survives_a_save() {
    let (mut fx, window) = working();
    broken(&mut fx, window, note(1));
    // The whole run, as `run.json` holds it.
    let value = serde_json::to_value(fx.run()).unwrap();
    let back: crate::run::model::Run = serde_json::from_value(value.clone()).unwrap();
    assert_eq!(&back, fx.run());
    assert_eq!(back.tasks[0].rounds[0].sandbox_note, Some(note(1)));
    // A run file written before the field: the key is absent, and it loads as `None`.
    let mut old = value;
    let round = &mut old["tasks"][0]["rounds"][0];
    assert!(
        round
            .as_object_mut()
            .unwrap()
            .remove("sandbox_note")
            .is_some()
    );
    let back: crate::run::model::Run = serde_json::from_value(old).unwrap();
    assert_eq!(back.tasks[0].rounds[0].sandbox_note, None);
    // A round with no note is written with no such key.
    fx.task_mut("t1").rounds[0].sandbox_note = None;
    let value = serde_json::to_value(fx.run()).unwrap();
    assert!(value["tasks"][0]["rounds"][0].get("sandbox_note").is_none());
}

/// Stream E review m2: a reason that already ends with the bubblewrap hint (a
/// missing-sandbox startup failure) gets the note appended and the hint not again.
#[test]
fn a_hinted_reason_is_not_hinted_twice() {
    use crate::headless::failure::{SANDBOX_HINT, SANDBOX_UNAVAILABLE};
    let (mut fx, window) = working();
    broken(&mut fx, window, note(1));
    let reason = format!("claude could not start: {SANDBOX_UNAVAILABLE}{SANDBOX_HINT}");
    let text = crate::run::engine::rounds::with_sandbox_note(fx.run_mut(), 0, reason.clone());
    assert_eq!(text, format!("{reason}; {}", note(1)));
    assert_eq!(text.matches(SANDBOX_HINT).count(), 1, "{text}");
}
