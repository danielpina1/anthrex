//! Milestone 9.11 decision 4 (A2, A4): a worker's waiting messages are carried into
//! its next session, never dropped. Rung 2 appends them to its hand-over; rung 3,
//! rung 4 and a lane out leave them in the outbox (a blocked task holds its mail), and
//! `run retry`'s fresh session takes them; a failed resume carries its in-flight ones.

use proto::{LaneState, MessageKind, RaceLane, TaskState};
use serde_json::json;

use super::control::retry;
use super::dispatch::edit;
use super::fixture::*;
use super::gates::only_op;
use super::holds::answer;
use super::race::{claim, lane};
use super::race_crown::crowned;
use super::race_end::b_out;
use super::race_lanes::proof;
use super::turns::{killed_exit, queue, working_on};
use super::worker_messages::msg;
use crate::run::contract::{answer_message, handover_prompt};
use crate::run::engine::{AgentSignal, Effect, OpKind, OpResult};
use crate::run::orch::contract::notes_section;

/// Room for every tool call and minute a test makes, so no budget rung interferes.
const ROOMY: &str = "[task.budget]\ntool_calls = 1000\nminutes = 1000";

fn args() -> serde_json::Value {
    json!({"summary": "did it", "test": "a::works", "red": "abcdef1"})
}

/// A `task_done` from `window` whose check fails (a generated file outside the task's
/// `owns`): one done-gate failure, its text in the tool's reply (`told`).
fn done_fails(fx: &mut Fixture, window: u32) -> Vec<Effect> {
    let effects = fx.tool(window, "task_done", args());
    let (op, _) = ops_in(&effects, "VerifyDone")[0].clone();
    let mut result = fx.clean_check("t1");
    if let OpResult::DoneChecked {
        generated_outside_owns,
        ..
    } = &mut result
    {
        *generated_outside_owns = vec!["Cargo.lock".into()];
    }
    fx.done(op, result)
}

/// A working `t1` whose done gate can fail (its profile names a generated file), on
/// `extra`'s budget; its window, its first turn open.
fn working_generated(extra: &str) -> (Fixture, u32) {
    let profile = PROFILE.replace("setup = ", "generated = [\"Cargo.lock\"]\nsetup = ");
    let mut fx = Fixture::new(&plan_with(&profile, &[task("t1", "S", "a", extra)]));
    fx.ready(true);
    let window = fx.launch_all()[0].1;
    assert!(fx.task("t1").rounds[0].turn_open);
    (fx, window)
}

/// The replaced session in `window` exits and its diff is answered: the fresh
/// session's `CreateWindow` first turn.
fn fresh_first_turn(fx: &mut Fixture, window: u32) -> String {
    let effects = killed_exit(fx, window);
    let (op, _) = only_op(&effects, "DiffSoFar");
    let effects = fx.done(
        op,
        OpResult::Diff {
            stat: String::new(),
            patch: String::new(),
        },
    );
    let (_, kind) = only_op(&effects, "CreateWindow");
    let OpKind::CreateWindow { first_turn, .. } = kind else {
        unreachable!()
    };
    first_turn
}

/// The texts waiting in the outbox for `address`, not yet delivered.
fn waiting(fx: &Fixture, address: &str) -> Vec<String> {
    (fx.run().outbox.iter())
        .filter(|m| m.task_id == address && m.delivered_at.is_none())
        .map(|m| m.text.clone())
        .collect()
}

#[test]
fn an_undelivered_answer_reaches_the_rung_2_hand_over() {
    let (mut fx, window) = working_generated(ROOMY);
    edit(&mut fx, vec![answer("use the users table")]);
    let text = answer_message("use the users table");
    assert_eq!(waiting(&fx, "t1"), vec![text.clone()], "the turn is open");
    done_fails(&mut fx, window);
    done_fails(&mut fx, window);
    assert_eq!(fx.task("t1").rung, 2);
    let first_turn = fresh_first_turn(&mut fx, window);
    assert!(first_turn.ends_with(&format!("\n\n{text}")), "{first_turn}");
    assert_eq!(first_turn.matches(&text).count(), 1, "{first_turn}");
    assert!(fx.run().outbox.is_empty(), "{:#?}", fx.run().outbox);
}

#[test]
fn a_budget_wrap_up_reaches_the_rung_2_hand_over() {
    // Five tool calls reach the soft budget (the wrap-up, queued while the turn is
    // open); the two `task_done` calls make seven, under the hard 7.5.
    let (mut fx, window) = working_generated("[task.budget]\ntool_calls = 5\nminutes = 1000");
    for _ in 0..5 {
        fx.signal(
            window,
            AgentSignal::ToolUse {
                name: "Bash".into(),
                target: None,
            },
        );
    }
    let queued = waiting(&fx, "t1");
    assert_eq!(queued.len(), 1, "the wrap-up: {queued:#?}");
    let wrap_up = queued[0].clone();
    done_fails(&mut fx, window);
    done_fails(&mut fx, window);
    assert_eq!((fx.task("t1").rung, fx.task("t1").budget_exceeded), (2, 0));
    let first_turn = fresh_first_turn(&mut fx, window);
    assert!(
        first_turn.ends_with(&format!("\n\n{wrap_up}")),
        "{first_turn}"
    );
    assert!(fx.run().outbox.is_empty(), "{:#?}", fx.run().outbox);
}

#[test]
fn a_message_already_in_the_notes_is_not_appended_twice() {
    let (mut fx, window) = working_generated(ROOMY);
    edit(
        &mut fx,
        vec![msg(&["t1"], MessageKind::Info, "the schema moved")],
    );
    assert_eq!(waiting(&fx, "t1").len(), 1, "queued while the turn is open");
    done_fails(&mut fx, window);
    done_fails(&mut fx, window);
    let first_turn = fresh_first_turn(&mut fx, window);
    let task = fx.task("t1");
    let notes = notes_section(&task.orch.messages);
    assert!(notes.contains("the schema moved"), "{notes}");
    let reason = format!(
        "the done gate failed again: {}",
        task.failure_log[1].lines().next().unwrap()
    );
    let expected = handover_prompt(fx.run(), task, &reason, "", "", "", &notes);
    assert_eq!(first_turn, expected, "no appended second copy");
    assert!(fx.run().outbox.is_empty(), "{:#?}", fx.run().outbox);
}

#[test]
fn an_answer_to_a_task_blocked_mis_sized_waits_for_the_retry() {
    let (mut fx, window) = working_generated(ROOMY);
    done_fails(&mut fx, window);
    done_fails(&mut fx, window);
    fresh_first_turn(&mut fx, window);
    let second = fx.complete_windows()[0].1;
    edit(&mut fx, vec![answer("keep the old API")]);
    let text = answer_message("keep the old API");
    assert_eq!(waiting(&fx, "t1"), vec![text.clone()], "the turn is open");
    done_fails(&mut fx, second);
    let t1 = fx.task("t1");
    assert_eq!((t1.state, t1.rung), (TaskState::Blocked, 3));
    assert_eq!(waiting(&fx, "t1"), vec![text.clone()], "held while blocked");
    killed_exit(&mut fx, second);
    assert_eq!(
        waiting(&fx, "t1"),
        vec![text.clone()],
        "held after the exit"
    );

    let effects = retry(&mut fx, "t1");
    let (op, _) = only_op(&effects, "DiffSoFar");
    let effects = fx.done(
        op,
        OpResult::Diff {
            stat: String::new(),
            patch: String::new(),
        },
    );
    let (_, kind) = only_op(&effects, "CreateWindow");
    let OpKind::CreateWindow { first_turn, .. } = kind else {
        unreachable!()
    };
    assert!(first_turn.ends_with(&format!("\n\n{text}")), "{first_turn}");
    assert!(fx.run().outbox.is_empty(), "{:#?}", fx.run().outbox);
}

#[test]
fn rung_4_and_a_lane_out_keep_the_mail() {
    // Rung 4: the M ceiling (150 tool calls, decision 40) is reached with a message
    // waiting for the open turn.
    let (mut fx, window) = working_on("");
    fx.task_mut("t1").spent_total.tool_calls = 148;
    queue(&mut fx, "stop at the parser");
    for _ in 0..2 {
        fx.signal(
            window,
            AgentSignal::ToolUse {
                name: "Bash".into(),
                target: None,
            },
        );
    }
    let t1 = fx.task("t1");
    assert_eq!((t1.state, t1.rung), (TaskState::Blocked, 4));
    assert_eq!(waiting(&fx, "t1"), vec!["stop at the parser".to_string()]);

    // The last lane's second gate failure takes it out of its race (rung 2 in its
    // view) and it is adopted (decision 20): a message queued while its claim was in
    // the proof gate becomes the task's, and the fresh session after the crown takes it.
    let (mut fx, a, _) = b_out();
    let effects = claim(&mut fx, RaceLane::A, a, HEAD);
    let (op, _) = only_op(&effects, "Proof");
    fx.done(op, proof(false));
    let effects = claim(&mut fx, RaceLane::A, a, HEAD);
    let (op, _) = only_op(&effects, "Proof");
    queue(&mut fx, "the lexer is shared");
    let text = "the lexer is shared".to_string();
    assert_eq!(waiting(&fx, "t1.a"), vec![text.clone()]);
    let effects = fx.done(op, proof(false));
    assert_eq!(lane(&fx, RaceLane::A).state, LaneState::Adopted);
    assert_eq!(lane(&fx, RaceLane::A).gates.rung, 2);
    assert_eq!(
        waiting(&fx, "t1.a"),
        vec![text.clone()],
        "the lane keeps it"
    );
    let (op, _) = only_op(&effects, "CrownRacer");
    crowned(&mut fx, op, HEAD);
    let first_turn = fresh_first_turn(&mut fx, a);
    assert!(first_turn.ends_with(&format!("\n\n{text}")), "{first_turn}");
    assert!(fx.run().outbox.is_empty(), "{:#?}", fx.run().outbox);
}
