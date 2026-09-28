//! Milestone 9 task M9.7: `run promote` gives a fast-path run an orchestrator
//! (decision 29). The fast-path task runs on; what the orchestrator adds before it
//! submits waits under hold `promotion`; a promotion recorded before milestone 9 is
//! performed on resume or on the tick.

use proto::{HoldState, OrchestratorChoice, RunPath, RunState, Runtime, TaskState};
use serde_json::json;

use super::dispatch::replies;
use super::fixture::*;
use super::orch::{ORCH, add, answer, edit_plan};
use crate::run::engine::{Effect, EngineState, EventKind, OpKind, OpResult};
use crate::run::model::Run;
use crate::run::triage::mark_fast;

/// A running fast-path run of `t1` with the deciders off, its run branch made.
fn fast() -> Fixture {
    let mut fx = Fixture::new(&plan_with(PROFILE, &[task("t1", "S", "auth", "")]));
    fx.start_with(false, |run: &mut Run| {
        mark_fast(run, super::orch::triage(RunPath::Fast), None);
    });
    let (op, _) = fx.op("CreateRunBranch");
    fx.done(op, OpResult::Worktree { head: BASE.into() });
    fx
}

fn promote(fx: &mut Fixture, choice: Option<OrchestratorChoice>) -> Vec<Effect> {
    let reply = fx.reply();
    fx.next(EventKind::Promote {
        reply,
        run_id: RUN_ID.into(),
        orchestrator: choice,
    })
}

/// [`fast`], promoted, with its orchestrator in window [`ORCH`].
fn promoted() -> Fixture {
    let mut fx = fast();
    promote(&mut fx, None);
    let (op, _) = fx.op("CreateOrchestrator");
    fx.done(
        op,
        OpResult::Window {
            window_id: ORCH,
            pid: None,
        },
    );
    fx
}

#[test]
fn promote_creates_an_orchestrator_and_leaves_t1_running() {
    let mut fx = fast();
    let t1 = fx.task("t1").clone();
    assert_eq!(t1.state, TaskState::Preparing);
    let choice = OrchestratorChoice {
        runtime: Runtime::Codex,
        model: None,
    };
    let effects = promote(&mut fx, Some(choice));
    assert_eq!(
        replies(&effects),
        vec![Ok(format!(
            "promoted: run {RUN_ID} now has an orchestrator; it starts in a moment (anthrex run status {RUN_ID})"
        ))]
    );
    let run = fx.run();
    assert_eq!(run.path, Some(RunPath::Plan));
    assert_eq!(run.promote_requested_at, Some(fx.now));
    assert_eq!(run.state, RunState::Running);
    let o = run.orch.orchestrator.as_ref().unwrap();
    assert_eq!(o.route.runtime, Runtime::Codex, "the user's choice");
    assert!(!o.plan_submitted);
    let creates = ops_in(&effects, "CreateOrchestrator");
    assert_eq!(creates.len(), 1);
    let OpKind::CreateOrchestrator { spec, .. } = &creates[0].1 else {
        unreachable!()
    };
    let first = spec.initial_prompt.clone().unwrap();
    assert!(
        first.contains("promoted from the fast path at the user's request"),
        "{first}"
    );
    assert_eq!(first, o.first_prompt);
    assert_eq!(*fx.task("t1"), t1, "t1 is exactly as it was");
    // The attention line of milestone 8b is gone: the orchestrator shows instead.
    let info = &crate::run::snapshot::snapshot(&fx.state, fx.now).runs[0];
    assert!(
        !info
            .attention
            .iter()
            .any(|a| a.starts_with("promotion requested")),
        "{:?}",
        info.attention
    );
    assert_eq!(
        info.orchestrator.as_ref().map(|o| o.route.runtime),
        Some(Runtime::Codex)
    );
}

#[test]
fn promote_repeat_reply_has_no_time() {
    let mut fx = promoted();
    let before = fx.run().clone();
    fx.now += 600;
    let effects = promote(&mut fx, None);
    assert_eq!(
        replies(&effects),
        vec![Ok(format!("run {RUN_ID} was already marked for promotion"))]
    );
    assert!(ops_in(&effects, "CreateOrchestrator").is_empty());
    assert_eq!(*fx.run(), before);
}

#[test]
fn tasks_added_before_submit_after_promote_carry_the_promotion_hold() {
    let mut fx = promoted();
    let (ok, value) = answer(&edit_plan(
        &mut fx,
        json!({"edits": [add("t2", "mail"), add("t3", "sms")]}),
    ));
    assert!(ok, "{value}");
    assert_eq!(value["held"], "promotion");
    assert_eq!(value["awaiting_approval"], false);
    let hold = fx.run().orch.gate_holds[0].clone();
    assert_eq!(hold.id, "promotion");
    assert_eq!(hold.kind, proto::HoldKind::Promotion);
    assert_eq!(hold.state, HoldState::Drafting);
    assert_eq!(hold.tasks, vec!["t2".to_string(), "t3".to_string()]);
    fx.tick();
    for id in ["t2", "t3"] {
        assert_eq!(fx.task(id).state, TaskState::Queued, "{id} waits");
        assert_eq!(fx.task(id).orch.gate_hold.as_deref(), Some("promotion"));
    }
    // A user's own `run edit` of the promoted run is not held (and is no longer
    // refused as a fast-path addition).
    let effects = super::dispatch::edit(
        &mut fx,
        vec![proto::PlanEdit::AddTask {
            task: super::holds::plan_task("t4", "[\"crates/log/**\"]"),
        }],
    );
    assert_eq!(replies(&effects), vec![Ok("applied 1 edit".to_string())]);
    assert_eq!(fx.task("t4").orch.gate_hold, None);
}

#[test]
fn promotion_hold_is_awaiting_after_submit() {
    let mut fx = promoted();
    edit_plan(&mut fx, json!({"edits": [add("t2", "mail")]}));
    let (ok, value) = answer(&edit_plan(&mut fx, json!({"edits": [], "submit": true})));
    assert!(ok, "{value}");
    let run = fx.run();
    assert_eq!(run.orch.gate_holds[0].state, HoldState::Awaiting);
    assert!(run.orch.orchestrator.as_ref().unwrap().plan_submitted);
    assert_eq!(run.state, RunState::Running);
    assert_eq!(fx.task("t2").state, TaskState::Queued);
    // After the submit, the orchestrator's additions are no longer held.
    edit_plan(&mut fx, json!({"edits": [add("t3", "sms")]}));
    assert_eq!(fx.task("t3").orch.gate_hold, None);
    let reply = fx.reply();
    let effects = fx.next(EventKind::Orch(
        crate::run::engine::OrchEvent::ApproveHold {
            reply,
            run_id: RUN_ID.into(),
            hold: "promotion".into(),
        },
    ));
    assert!(replies(&effects)[0].is_ok());
    assert_eq!(fx.task("t2").state, TaskState::Preparing);
}

/// A fast-path run on which milestone 8b recorded `run promote`.
fn recorded() -> Fixture {
    let mut fx = fast();
    fx.run_mut().promote_requested_at = Some(1_500);
    fx
}

#[test]
fn pre_m9_promote_request_is_performed_on_resume_and_on_tick() {
    // On the tick, while it runs.
    let mut fx = recorded();
    let effects = fx.tick();
    assert_eq!(ops_in(&effects, "CreateOrchestrator").len(), 1);
    assert_eq!(fx.run().path, Some(RunPath::Plan));
    assert_eq!(
        fx.run().promote_requested_at,
        Some(1_500),
        "its own time kept"
    );
    assert!(ops_in(&fx.tick(), "CreateOrchestrator").is_empty(), "once");

    // After a daemon restart the run is paused: nothing on the tick, then on resume.
    let mut fx = recorded();
    let runs: Vec<_> = fx.state.runs.values().cloned().collect();
    fx.state = EngineState::default();
    fx.next(EventKind::Restore {
        runs,
        replay: Vec::new(),
        held: Vec::new(),
    });
    assert_eq!(fx.run().state, RunState::Paused);
    assert!(ops_in(&fx.tick(), "CreateOrchestrator").is_empty());
    let reply = fx.reply();
    let effects = fx.next(EventKind::Resume {
        reply,
        run_id: RUN_ID.into(),
        rebaseline: None,
    });
    assert_eq!(ops_in(&effects, "CreateOrchestrator").len(), 1);
    assert!(fx.run().orch.orchestrator.is_some());

    // The control: a fast-path run with no recorded wish is left alone.
    let mut fx = fast();
    assert!(ops_in(&fx.tick(), "CreateOrchestrator").is_empty());
}
