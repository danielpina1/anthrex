//! Milestone 9.9 task M9.9.7 (OFA §4.3): `ask_user`, the orchestrator's one question to
//! the user, its pending record and the user's `AnswerAsk`.

use serde_json::json;

use super::dispatch::replies;
use super::fixture::*;
use super::orch::{ORCH, error, orch_tool};
use super::wake_notes::{approved, clear, notes};
use crate::run::engine::{EventKind, OrchEvent};

fn ask(fx: &mut Fixture, args: serde_json::Value) -> Vec<crate::run::engine::Effect> {
    orch_tool(fx, ORCH, "ask_user", args)
}

fn answer(fx: &mut Fixture, id: u64, choice: Option<u32>) -> Vec<crate::run::engine::Effect> {
    let reply = fx.reply();
    fx.next(EventKind::Orch(OrchEvent::AnswerAsk {
        reply,
        run_id: RUN_ID.into(),
        ask: id,
        choice,
    }))
}

fn pending(fx: &Fixture) -> Option<proto::AskInfo> {
    crate::run::snapshot::snapshot(&fx.state, fx.now).runs[0]
        .orchestrator
        .as_ref()
        .unwrap()
        .ask
        .clone()
}

#[test]
fn a_question_is_stored_and_answered_by_option() {
    let mut fx = approved();
    let effects = ask(
        &mut fx,
        json!({"question": "tabs or spaces?", "options": ["tabs", "spaces"], "context": "the style guide is silent"}),
    );
    assert_eq!(
        replies(&effects)[0].clone().unwrap(),
        "asked; the answer arrives as a message"
    );
    let info = pending(&fx).unwrap();
    assert_eq!(
        (info.id, info.question.as_str(), info.options.len()),
        (1, "tabs or spaces?", 2)
    );
    clear(&mut fx);
    let effects = answer(&mut fx, 1, Some(1));
    assert!(replies(&effects)[0].is_ok());
    assert_eq!(pending(&fx), None);
    assert_eq!(notes(&fx), vec!["the user chose: spaces".to_string()]);
}

#[test]
fn a_new_question_replaces_an_unanswered_one_and_the_old_id_is_refused() {
    let mut fx = approved();
    ask(&mut fx, json!({"question": "one?"}));
    let effects = ask(&mut fx, json!({"question": "two?", "options": ["a"]}));
    assert_eq!(
        replies(&effects)[0].clone().unwrap(),
        "asked, replacing your earlier question; the answer arrives as a message"
    );
    assert!(
        fx.run()
            .log
            .iter()
            .any(|e| e.text == "orchestrator asks: two? (replaces \"one?\")")
    );
    let effects = answer(&mut fx, 1, Some(0));
    assert_eq!(
        replies(&effects)[0].clone().unwrap_err(),
        "that question was replaced; answer the new one"
    );
    let effects = answer(&mut fx, 2, Some(3));
    assert_eq!(replies(&effects)[0].clone().unwrap_err(), "choose 1 to 1");
    assert!(pending(&fx).is_some());
}

#[test]
fn answering_in_the_window_clears_it_without_a_note() {
    let mut fx = approved();
    ask(&mut fx, json!({"question": "go on?"}));
    clear(&mut fx);
    assert!(replies(&answer(&mut fx, 1, None))[0].is_ok());
    assert_eq!(pending(&fx), None);
    assert!(notes(&fx).is_empty());
    assert_eq!(
        replies(&answer(&mut fx, 1, None))[0].clone().unwrap_err(),
        format!("run {RUN_ID} has no pending question")
    );
}

#[test]
fn a_question_is_dropped_when_the_run_completes() {
    let mut fx = super::actions_fixtures::complete_orchestrated();
    ask(&mut fx, json!({"question": "anything else?"}));
    assert!(pending(&fx).is_some(), "a complete run may still ask");
    fx.run_mut().state = proto::RunState::Accepted;
    fx.tick();
    assert_eq!(pending(&fx), None);
}

#[test]
fn bounds_are_checked_again_by_the_daemon() {
    let mut fx = approved();
    let ten: Vec<String> = (0..10).map(|n| n.to_string()).collect();
    let effects = ask(&mut fx, json!({"question": "q", "options": ten}));
    assert_eq!(error(&effects), "invalid arguments: options: at most 9");
    let effects = ask(&mut fx, json!({"question": ""}));
    assert_eq!(error(&effects), "invalid arguments: question: empty");
}

#[test]
fn a_question_asked_while_complete_waits_through_a_step() {
    let mut fx = approved();
    ask(&mut fx, json!({"question": "still there?"}));
    fx.run_mut().state = proto::RunState::Complete;
    // `before` of this step is Complete too: the run did not enter it.
    fx.tick();
    assert!(pending(&fx).is_some());
}

#[test]
fn a_run_entering_complete_in_a_step_drops_its_question() {
    use super::kinds_integration::{C1, merge_real};
    use super::orch::{add, edit_plan, launched};
    use crate::run::engine::OpResult;
    let mut fx = launched(false);
    edit_plan(
        &mut fx,
        json!({"edits": [add("t1", "auth")], "submit": true}),
    );
    fx.approve();
    merge_real(&mut fx, "t1", C1);
    ask(&mut fx, json!({"question": "before the end?"}));
    assert_ne!(fx.run().state, proto::RunState::Complete);
    assert!(pending(&fx).is_some());
    let (op, _) = fx.op("VerifyRefs");
    fx.done(op, OpResult::RefsOk);
    assert_eq!(fx.run().state, proto::RunState::Complete);
    assert_eq!(pending(&fx), None);
}

#[test]
fn a_terminal_run_refuses_a_question_with_its_state() {
    let mut fx = approved();
    fx.run_mut().state = proto::RunState::Failed;
    let effects = ask(&mut fx, json!({"question": "late"}));
    assert_eq!(error(&effects), format!("run {RUN_ID} is failed"));
}

#[test]
fn an_option_choice_on_a_question_without_options_is_refused() {
    let mut fx = approved();
    ask(&mut fx, json!({"question": "free text?"}));
    let effects = answer(&mut fx, 1, Some(0));
    assert_eq!(
        replies(&effects)[0].clone().unwrap_err(),
        "this question has no options"
    );
    assert!(pending(&fx).is_some());
}

#[test]
fn a_pending_question_survives_a_restart_and_an_old_run_has_none() {
    let mut fx = approved();
    ask(&mut fx, json!({"question": "q?", "options": ["a"]}));
    let text = serde_json::to_string(&fx.run().orch).unwrap();
    let back: crate::run::orch::RunOrch = serde_json::from_str(&text).unwrap();
    assert_eq!((back.ask_seq, back.ask), (1, fx.run().orch.ask.clone()));
    let empty = serde_json::to_string(&crate::run::orch::RunOrch::default()).unwrap();
    assert!(!empty.contains("ask"), "{empty}");
}

/// Final review M-4: `run_status`'s digest carries the pending question (its id and
/// text), so a relaunched orchestrator sees what the user is answering.
#[test]
fn the_digest_shows_the_pending_question_until_it_is_answered() {
    let mut fx = approved();
    ask(&mut fx, json!({"question": "tabs or spaces?", "options": ["tabs", "spaces"]}));
    let status = |fx: &mut Fixture| crate::run::orch::digest::digest(fx.run(), fx.now)["ask"].clone();
    assert_eq!(
        status(&mut fx),
        json!({"id": 1, "question": "tabs or spaces?"})
    );
    answer(&mut fx, 1, Some(0));
    assert_eq!(status(&mut fx), serde_json::Value::Null);
}
