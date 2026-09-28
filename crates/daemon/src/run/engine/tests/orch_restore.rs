//! Milestone 9 task M9.7: a planned run across a daemon restart (decisions 11 and 26).
//! The orchestrator's window comes back dormant; `run resume` restarts it with a new
//! session, and returns a planning run to `planning`.

use proto::{RunState, TaskState};
use serde_json::json;

use super::dispatch::replies;
use super::fixture::*;
use super::orch::{ORCH, add, edit_plan, launched, planned};
use crate::run::engine::{EngineState, Event, EventKind, OpKind, OpResult, step};

/// The fixture's state after a daemon restart: its runs restored, nothing replayed.
fn restart(fx: &mut Fixture) {
    let runs: Vec<_> = fx.state.runs.values().cloned().collect();
    fx.state = EngineState::default();
    fx.next(EventKind::Restore {
        runs,
        replay: Vec::new(),
        held: Vec::new(),
    });
}

fn resume(fx: &mut Fixture) -> Vec<crate::run::engine::Effect> {
    let reply = fx.reply();
    fx.next(EventKind::Resume {
        reply,
        run_id: RUN_ID.into(),
        rebaseline: None,
    })
}

#[test]
fn restore_pauses_a_planning_run_and_resume_restarts_the_orchestrator() {
    let mut fx = launched(false);
    restart(&mut fx);
    let run = fx.run();
    assert_eq!(run.state, RunState::Paused);
    assert_eq!(run.paused_from, Some(RunState::Planning));
    let o = run.orch.orchestrator.as_ref().unwrap();
    assert!(!o.live, "restored dormant");
    assert_eq!(o.window_id, Some(ORCH));
    // A paused run's orchestrator tools answer M8a's paused text.
    let effects = edit_plan(&mut fx, json!({"edits": [add("t1", "auth")]}));
    assert_eq!(
        super::orch::error(&effects),
        format!("run {RUN_ID} is paused; the user must resume it")
    );
    let effects = resume(&mut fx);
    assert_eq!(replies(&effects), vec![Ok(format!("run {RUN_ID} resumed"))]);
    assert_eq!(fx.run().state, RunState::Planning);
    let restarts = ops_in(&effects, "RestartOrchestrator");
    assert_eq!(restarts.len(), 1, "{effects:?}");
    assert!(matches!(
        restarts[0].1,
        OpKind::RestartOrchestrator { window_id: ORCH }
    ));
    assert_eq!(fx.run().orch.orchestrator.as_ref().unwrap().session, 2);
    assert!(
        ops_in(&effects, "CreateWindow").is_empty(),
        "nothing else starts"
    );
    fx.done(restarts[0].0, OpResult::Restarted);
    assert!(fx.run().orch.orchestrator.as_ref().unwrap().live);
    // A second resume has nothing to restart.
    let effects = resume(&mut fx);
    assert!(ops_in(&effects, "RestartOrchestrator").is_empty());
}

#[test]
fn resume_of_awaiting_approval_restarts_a_dormant_orchestrator_without_changing_state() {
    let mut fx = launched(false);
    edit_plan(
        &mut fx,
        json!({"edits": [add("t1", "auth")], "submit": true}),
    );
    restart(&mut fx);
    assert_eq!(
        fx.run().state,
        RunState::AwaitingApproval,
        "the gate is kept"
    );
    let effects = resume(&mut fx);
    assert_eq!(
        replies(&effects),
        vec![Ok(format!("run {RUN_ID}: its orchestrator restarts"))]
    );
    assert_eq!(ops_in(&effects, "RestartOrchestrator").len(), 1);
    assert_eq!(fx.run().state, RunState::AwaitingApproval);
    assert_eq!(fx.task("t1").state, TaskState::Queued);
    // With its orchestrator live, M8a's refusal stands.
    let (op, _) = fx.op("RestartOrchestrator");
    fx.done(op, OpResult::Restarted);
    assert_eq!(
        replies(&resume(&mut fx)),
        vec![Err(format!("run {RUN_ID} is awaiting_approval"))]
    );
}

/// A `CreateOrchestrator` the restart lost is launched again by `run resume`; one the
/// journal answered comes back with its window.
#[test]
fn a_lost_orchestrator_launch_is_made_again_on_resume() {
    let mut fx = planned(false);
    let (branch, _) = fx.op("CreateRunBranch");
    fx.done(branch, OpResult::Worktree { head: BASE.into() });
    let lost = fx.op("CreateOrchestrator").0;
    restart(&mut fx);
    assert!(!fx.run().pending_ops.contains_key(&lost));
    let o = fx.run().orch.orchestrator.clone().unwrap();
    assert_eq!((o.launch_op, o.window_id), (None, None));
    let effects = resume(&mut fx);
    assert_eq!(ops_in(&effects, "CreateOrchestrator").len(), 1);

    // Replayed: the window the journal recorded, dormant.
    let (op, _) = fx.op("CreateOrchestrator");
    let mut run = fx.run().clone();
    run.state = RunState::Planning;
    let (state, _) = step(
        EngineState::default(),
        Event {
            now: fx.now + 1,
            kind: EventKind::Restore {
                runs: vec![run],
                replay: vec![(
                    RUN_ID.into(),
                    op,
                    OpResult::Window {
                        window_id: 7,
                        pid: None,
                    },
                )],
                held: Vec::new(),
            },
        },
    );
    let o = state.runs[RUN_ID].orch.orchestrator.clone().unwrap();
    assert_eq!((o.window_id, o.live), (Some(7), false));
}
