//! Milestone 9 task M9.13a: `stop_and_wait` and `paused(message)` (decision 42c), and a
//! worker's `task_note` (decision 42f). Split from `worker_messages.rs` for size.

use proto::{AgentRole, BlockReason, MessageKind, PlanEdit, RunState, TaskState, ToolCall};
use serde_json::{Value, json};

use super::control::{override_task, retry};
use super::control_restore::restart;
use super::dispatch::{edit, replies};
use super::done::{done_args, one_reply};
use super::fixture::*;
use super::holds::delivers;
use super::orch::{add, answer, edit_plan, launched};
use super::turns::working;
use super::worker_messages::{ack, from_user, msg};
use crate::run::contract::amend_message;
use crate::run::engine::{Effect, EventKind, OrchEvent};
use crate::run::orch::contract::{
    NOTE_LIMIT, NOTE_RECORDED, PAUSE_RELEASED, STOP_AND_WAIT_REFUSAL,
};
use crate::run::snapshot::attention;

/// `t1` working, then paused by the user's `stop_and_wait`, its stop turn delivered
/// and closed.
pub(super) fn paused() -> (Fixture, u32) {
    let (mut fx, window) = working();
    edit(
        &mut fx,
        vec![msg(&["t1"], MessageKind::StopAndWait, "hold on")],
    );
    let effects = fx.turn_completed(window);
    ack(&mut fx, &effects);
    fx.turn_completed(window);
    (fx, window)
}

fn is_paused(fx: &Fixture) -> bool {
    let task = fx.task("t1");
    task.state == TaskState::Blocked
        && task
            .block
            .as_ref()
            .is_some_and(|b| b.reason == BlockReason::MessagePause)
}

/// A worker's `task_note` from `window`, as the driver routes it (`OrchEvent::Tool`).
pub(super) fn note(fx: &mut Fixture, window: u32, kind: &str, text: &str) -> Vec<Effect> {
    let reply = fx.reply();
    fx.next(EventKind::Orch(OrchEvent::Tool {
        reply,
        call: ToolCall {
            run_id: RUN_ID.into(),
            task_id: Some("t1".into()),
            role: AgentRole::Worker,
            window_id: window,
            tool: "task_note".into(),
            args: json!({"kind": kind, "text": text}),
            scout_id: None,
            epic: None,
            chain: None,
            lane: None,
        },
        refusals: Vec::new(),
    }))
}

/// A planned run with its orchestrator, `t1` submitted and working: its window.
pub(super) fn orchestrated() -> (Fixture, u32) {
    let mut fx = launched(true);
    let submit = json!({"edits": [add("t1", "a")], "submit": true});
    assert!(answer(&edit_plan(&mut fx, submit)).0);
    let window = fx.launch_all()[0].1;
    (fx, window)
}

pub(super) fn wake_notes(fx: &Fixture) -> Vec<String> {
    fx.run().orch.orchestrator.as_ref().unwrap().notes.clone()
}

#[test]
fn stop_and_wait_pauses_refuses_task_done_and_a_message_resumes() {
    let (mut fx, window) = working();
    let effects = edit(
        &mut fx,
        vec![msg(&["t1"], MessageKind::StopAndWait, "hold on")],
    );
    assert!(replies(&effects)[0].is_ok(), "{effects:#?}");
    assert!(is_paused(&fx));
    assert_eq!(
        fx.task("t1").block.as_ref().unwrap().text,
        "asked to stop and wait: hold on"
    );
    assert!(
        !effects
            .iter()
            .any(|e| matches!(e, Effect::Interrupt { .. })),
        "the turn is never cut short"
    );
    // A worker still in its turn hears the stop at once.
    let effects = fx.tool(window, "task_done", done_args());
    assert_eq!(one_reply(&effects), Err(STOP_AND_WAIT_REFUSAL.to_string()));
    // Otherwise at the end of the turn, and that turn's end starts no fallback.
    let effects = fx.turn_completed(window);
    assert_eq!(
        delivers(&effects),
        vec![from_user(MessageKind::StopAndWait, "hold on")]
    );
    ack(&mut fx, &effects);
    let effects = fx.turn_completed(window);
    assert!(ops_in(&effects, "CountCommits").is_empty(), "{effects:#?}");
    // A second stop is refused; an `info` message releases the task.
    let again = edit(&mut fx, vec![msg(&["t1"], MessageKind::StopAndWait, "x")]);
    assert_eq!(
        replies(&again),
        vec![Err(
            "message: no recipient can take it: task t1 is already paused(message)".into()
        )]
    );
    let effects = edit(&mut fx, vec![msg(&["t1"], MessageKind::Info, "go on")]);
    assert_eq!(fx.task("t1").state, TaskState::Working);
    assert_eq!(fx.task("t1").block, None);
    assert_eq!(
        delivers(&effects),
        vec![from_user(MessageKind::Info, "go on")]
    );
}

#[test]
fn brief_amend_resumes_a_paused_task() {
    let (mut fx, _) = paused();
    let amend = PlanEdit::AmendTask {
        task_id: "t1".into(),
        brief: Some("New brief".into()),
        acceptance: None,
        route: None,
        test_mode: None,
        test_mode_reason: None,
        priority: None,
        size: None,
        deps: None,
        stage: None,
        race: None,
        pair: None,
    };
    let effects = edit(&mut fx, vec![amend]);
    assert!(replies(&effects)[0].is_ok(), "{effects:#?}");
    assert_eq!(fx.task("t1").state, TaskState::Working);
    assert_eq!(delivers(&effects), vec![amend_message(fx.task("t1"))]);
}

#[test]
fn resume_plan_edit_releases_paused_tasks() {
    let (mut fx, _) = paused();
    edit(&mut fx, vec![PlanEdit::Pause]);
    assert_eq!(fx.run().state, RunState::Paused);
    let effects = edit(&mut fx, vec![PlanEdit::Resume]);
    assert!(replies(&effects)[0].is_ok(), "{effects:#?}");
    assert_eq!(fx.run().state, RunState::Running);
    assert_eq!(fx.task("t1").state, TaskState::Working);
    assert_eq!(delivers(&effects), vec![PAUSE_RELEASED.to_string()]);
}

#[test]
fn answer_to_a_paused_task_is_refused() {
    let (mut fx, _) = paused();
    let answer = PlanEdit::Answer {
        task_id: "t1".into(),
        text: "x".into(),
    };
    assert_eq!(
        replies(&edit(&mut fx, vec![answer])),
        vec![Err(
            "task t1 is blocked(message_pause); only blocked(question) or \
                  working tasks can be answered"
                .into()
        )]
    );
    assert!(is_paused(&fx));
}

#[test]
fn paused_task_refuses_retry_override_and_route_amend() {
    let (mut fx, _) = paused();
    let refusal = "task t1 is paused(message); send it a message to resume it".to_string();
    assert_eq!(one_reply(&retry(&mut fx, "t1")), Err(refusal.clone()));
    assert_eq!(one_reply(&override_task(&mut fx, "t1", "r")), Err(refusal));
    let route: Value = json!({"op": "amend_task", "task_id": "t1", "size": "M"});
    let amend: PlanEdit = serde_json::from_value(route).unwrap();
    assert_eq!(
        replies(&edit(&mut fx, vec![amend])),
        vec![Err(
            "task t1 is blocked(message_pause); size can be amended only on \
                  pending, queued or blocked tasks"
                .into()
        )]
    );
    assert!(is_paused(&fx));
}

#[test]
fn paused_task_holds_no_writer_slot_and_blocks_completion() {
    let plan = plan_with(
        &profile_with("max_writers = 1"),
        &[task("t1", "S", "a", ""), task("t2", "S", "b", "")],
    );
    let mut fx = Fixture::new(&plan);
    fx.ready(true);
    fx.launch_all();
    assert_eq!(fx.task("t2").state, TaskState::Queued, "one writer slot");
    let effects = edit(
        &mut fx,
        vec![msg(&["t1"], MessageKind::StopAndWait, "wait")],
    );
    assert_eq!(tasks_of(&effects, "PrepareWorktree"), vec!["t2"]);
    fx.launch_all();
    fx.merge("t2", "m2m2m2m2m2m2m2m2m2m2m2m2m2m2m2m2m2m2m2m2");
    assert!(is_paused(&fx));
    assert_eq!(
        fx.run().state,
        RunState::Running,
        "not complete while paused"
    );
}

#[test]
fn a_pause_survives_restore_and_run_resume() {
    let (mut fx, _) = paused();
    restart(&mut fx, Vec::new());
    assert_eq!(fx.run().state, RunState::Paused);
    super::control::resume(&mut fx);
    assert_eq!(fx.run().state, RunState::Running);
    assert!(
        is_paused(&fx),
        "only a message, an amend or the resume edit releases it"
    );
}

#[test]
fn paused_attention_only_after_ten_minutes() {
    let (fx, _) = paused();
    let since = fx.task("t1").phase_since;
    let lines = attention(fx.run(), since + 599);
    assert!(lines.iter().all(|l| !l.starts_with("t1 ")), "{lines:?}");
    let lines = attention(fx.run(), since + 600);
    assert!(
        lines.contains(&"t1 paused(message) for 10 min".to_string()),
        "{lines:?}"
    );
}

#[test]
fn paused_block_adds_no_wake_note() {
    let (mut fx, _) = orchestrated();
    let before = wake_notes(&fx);
    let call = json!({"edits": [{"op": "message", "to": ["t1"], "kind": "stop_and_wait",
                                  "text": "hold"}]});
    assert!(answer(&edit_plan(&mut fx, call)).0);
    assert!(is_paused(&fx));
    fx.tick();
    assert_eq!(wake_notes(&fx), before);
}

#[test]
fn task_note_is_recorded_and_changes_no_state() {
    let (mut fx, window) = working();
    let before = fx.task("t1").state;
    let effects = note(&mut fx, window, "discovery", "the parser is shared");
    assert_eq!(one_reply(&effects), Ok(NOTE_RECORDED.to_string()));
    let task = fx.task("t1");
    assert_eq!(task.state, before);
    assert_eq!(task.orch.worker_notes.len(), 1);
    assert_eq!(
        task.orch.worker_notes[0].seq, 1,
        "stored through add_worker_note"
    );
    assert_eq!(
        task.history.last().unwrap().text,
        "note (discovery): the parser is shared"
    );
    // Another window is refused.
    let effects = note(&mut fx, window + 50, "risk", "x");
    assert_eq!(
        one_reply(&effects),
        Err("this window is not the current worker of task t1".into())
    );
}

#[test]
fn task_note_from_a_paused_task_is_accepted() {
    let (mut fx, window) = paused();
    let effects = note(&mut fx, window, "progress", "waiting");
    assert_eq!(one_reply(&effects), Ok(NOTE_RECORDED.to_string()));
    assert!(is_paused(&fx));
}

#[test]
fn notes_are_capped() {
    let (mut fx, window) = working();
    fx.run_mut().limits.orch.note_max_per_task = 2;
    for n in 0..2 {
        let effects = note(&mut fx, window, "progress", &format!("n{n}"));
        assert_eq!(one_reply(&effects), Ok(NOTE_RECORDED.to_string()));
    }
    let effects = note(&mut fx, window, "progress", "n2");
    assert_eq!(one_reply(&effects), Err(NOTE_LIMIT.to_string()));
    assert_eq!(fx.task("t1").orch.worker_notes.len(), 2);
}

#[test]
fn note_max_per_task_zero_refuses_every_note() {
    let (mut fx, window) = working();
    fx.run_mut().limits.orch.note_max_per_task = 0;
    let effects = note(&mut fx, window, "risk", "r");
    assert_eq!(one_reply(&effects), Err(NOTE_LIMIT.to_string()));
    assert!(fx.task("t1").orch.worker_notes.is_empty());
}

#[test]
fn discovery_and_risk_notes_wake_and_progress_does_not() {
    let (mut fx, window) = orchestrated();
    let before = wake_notes(&fx).len();
    note(&mut fx, window, "progress", "half way");
    assert_eq!(wake_notes(&fx).len(), before);
    note(&mut fx, window, "discovery", "the API is shared");
    note(&mut fx, window, "risk", "migration may lock");
    let notes = wake_notes(&fx);
    assert_eq!(
        notes[before..].to_vec(),
        vec![
            "t1 noted a discovery: the API is shared".to_string(),
            "t1 noted a risk: migration may lock".to_string()
        ]
    );
    // The run view's attention list (decision 42i); the digest has `task_notes`.
    let snapshot = crate::run::snapshot::snapshot(&fx.state, fx.now);
    let lines = &snapshot.runs[0].attention;
    assert!(
        lines.contains(&"t1 noted a risk: migration may lock".to_string()),
        "{lines:?}"
    );
    assert!(!lines.iter().any(|l| l.contains("half way")), "{lines:?}");
}
