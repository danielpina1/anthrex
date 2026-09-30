//! A session's events that reach the engine before its round knows its window (the
//! followups file's "A session's signals and tool calls before its `CreateWindow`
//! result are lost"): the driver sends the `Window` result only after the op's `done`
//! line is synced, and the session's process starts before that. The engine holds a
//! window's signals and tool calls while a launch is in flight and replays them, in
//! order and with their own `now`, once the round has the window.

use proto::{AgentRole, TaskState};

use super::dispatch::replies;
use super::done::{done_args, working};
use super::fixture::*;
use super::gates::{CHECK_MODE, only_op, working_on};
use super::gates_review::{in_review, submit, verdict};
use crate::run::contract::{DONE_ACCEPTED, REVIEW_RECORDED};
use crate::run::engine::{AgentSignal, Effect, EventKind, OpResult, TurnOutcome};
use crate::run::model::OpId;

const EARLY: u32 = 7;

/// A single-task run whose worker's `CreateWindow` is in flight: its op.
fn launching() -> (Fixture, OpId) {
    let mut fx = Fixture::new(&plan_with(PROFILE, &[task("t1", "S", "a", "")]));
    fx.ready(true);
    fx.complete_prepares();
    let (op, _) = fx.op("CreateWindow");
    assert_eq!(fx.task("t1").rounds[0].window_id, None);
    (fx, op)
}

fn bind(fx: &mut Fixture, op: OpId, window_id: u32) -> Vec<Effect> {
    fx.done(
        op,
        OpResult::Window {
            window_id,
            pid: None,
        },
    )
}

fn not_worker() -> String {
    "this window is not the current worker of task t1".to_string()
}

#[test]
fn a_signal_before_the_window_is_replayed_once_the_round_has_it() {
    let (mut fx, op) = launching();
    fx.signal(EARLY, AgentSignal::ProcessStarted { pid: 42 });
    fx.signal(
        EARLY,
        AgentSignal::Init {
            session_id: "s-1".into(),
        },
    );
    fx.signal(
        EARLY,
        AgentSignal::ToolUse {
            name: "Edit".into(),
            target: None,
        },
    );
    let ended_at = fx.now + 1;
    fx.turn_ended(EARLY, TurnOutcome::Completed);
    // Nothing is applied yet: no round has the window.
    let round = &fx.task("t1").rounds[0];
    assert_ne!(round.session_id.as_deref(), Some("s-1"));
    assert_eq!(round.tool_calls, 0);
    assert!(round.turn_open);

    fx.now += 5;
    bind(&mut fx, op, EARLY);
    let round = &fx.task("t1").rounds[0];
    assert_eq!(round.window_id, Some(EARLY));
    assert_eq!(round.pid, Some(42));
    assert_eq!(round.session_id.as_deref(), Some("s-1"));
    assert_eq!(round.tool_calls, 1);
    assert!(!round.turn_open, "the held turn end closed the turn");
    assert_eq!(round.last_event, ended_at, "replayed with its own now");
    assert!(fx.state.pending.is_empty());
}

#[test]
fn a_task_done_before_the_window_is_answered_after_binding() {
    let (mut fx, op) = launching();
    let effects = fx.tool(EARLY, "task_done", done_args());
    assert!(
        replies(&effects).is_empty(),
        "held, not refused: {effects:#?}"
    );
    assert!(ops_in(&effects, "VerifyDone").is_empty());

    let effects = bind(&mut fx, op, EARLY);
    assert_eq!(fx.task("t1").state, TaskState::Working);
    let (verify, _) = only_op(&effects, "VerifyDone");
    assert!(replies(&effects).is_empty(), "{effects:#?}");
    assert!(fx.task("t1").claim.is_some(), "the claim is recorded");

    let result = fx.clean_check("t1");
    let effects = fx.done(verify, result);
    assert_eq!(replies(&effects), vec![Ok(DONE_ACCEPTED.to_string())]);
}

#[test]
fn a_review_before_the_reviewer_window_is_answered_after_binding() {
    let (mut fx, window) = working_on(PROFILE, CHECK_MODE);
    let (op, _) = in_review(&mut fx, window);
    let effects = fx.done(
        op,
        OpResult::Review {
            base: BASE.into(),
            head: HEAD.into(),
            patch: "diff --git a/x b/x".into(),
        },
    );
    let (launch, _) = only_op(&effects, "CreateWindow");
    let rwindow = 90;
    let effects = submit(&mut fx, rwindow, verdict("approve", vec![]));
    assert!(
        replies(&effects).is_empty(),
        "held, not refused: {effects:#?}"
    );

    let effects = bind(&mut fx, launch, rwindow);
    assert_eq!(replies(&effects), vec![Ok(REVIEW_RECORDED.to_string())]);
    assert!(fx.task("t1").reviews[0].verdict.is_some());
    assert!(fx.state.pending.is_empty());
}

#[test]
fn a_failed_launch_discards_the_held_signals_and_refuses_the_held_call() {
    let (mut fx, op) = launching();
    fx.signal(
        EARLY,
        AgentSignal::Init {
            session_id: "s-1".into(),
        },
    );
    let effects = fx.tool(EARLY, "task_done", done_args());
    assert!(replies(&effects).is_empty());

    let effects = fx.done(
        op,
        OpResult::Failed {
            message: "no pty".into(),
        },
    );
    assert_eq!(replies(&effects), vec![Err(not_worker())]);
    assert!(fx.state.pending.is_empty(), "{:?}", fx.state.pending);
    assert_ne!(fx.task("t1").rounds[0].session_id.as_deref(), Some("s-1"));
}

#[test]
fn events_of_a_window_with_no_launch_in_flight_are_dropped() {
    let (mut fx, _) = working();
    fx.signal(
        99,
        AgentSignal::Init {
            session_id: "other".into(),
        },
    );
    assert!(fx.state.pending.is_empty());
    let effects = fx.tool(99, "task_done", done_args());
    assert_eq!(replies(&effects), vec![Err(not_worker())]);
    assert!(fx.state.pending.is_empty());
}

#[test]
fn a_held_call_is_refused_when_its_launch_takes_too_long() {
    let (mut fx, _) = launching();
    let effects = fx.tool(EARLY, "task_done", done_args());
    assert!(replies(&effects).is_empty());
    let limit = crate::run::engine::HOLD_LIMIT_SECS;
    fx.now += limit - 2;
    assert!(replies(&fx.tick()).is_empty(), "still held");
    let effects = fx.tick();
    assert_eq!(replies(&effects), vec![Err(not_worker())]);
    assert!(fx.state.pending.is_empty());
}

#[test]
fn a_window_holds_at_most_the_cap_and_keeps_its_turn_end() {
    let cap = crate::run::engine::HOLD_CAP;
    let (mut fx, op) = launching();
    // A burst within one second, well inside the hold's limit.
    let at = fx.now + 1;
    let burst = |fx: &mut Fixture, signal| {
        fx.send(
            at,
            EventKind::Signal {
                window_id: EARLY,
                signal,
            },
        );
    };
    burst(&mut fx, AgentSignal::ProcessStarted { pid: 42 });
    for _ in 0..cap + 40 {
        burst(&mut fx, AgentSignal::Activity);
    }
    let end = AgentSignal::TurnEnded {
        outcome: TurnOutcome::Completed,
        usage: None,
        denials: vec![],
    };
    burst(&mut fx, end);
    assert_eq!(fx.state.pending[&EARLY].events.len(), cap);

    bind(&mut fx, op, EARLY);
    let round = &fx.task("t1").rounds[0];
    // The oldest counter-only signals made room; the process and the turn end stayed.
    assert_eq!(round.pid, Some(42));
    assert!(!round.turn_open);
    assert_eq!(round.last_event, at);
}

#[test]
fn at_most_the_window_cap_of_windows_hold_events() {
    let cap = crate::run::engine::HOLD_WINDOWS_CAP;
    let (mut fx, _) = launching();
    let first = 1000;
    // A burst within one second, well inside the hold's limit.
    let at = fx.now + 1;
    let activity = |fx: &mut Fixture, window_id| {
        fx.send(
            at,
            EventKind::Signal {
                window_id,
                signal: AgentSignal::Activity,
            },
        );
    };
    for w in first..first + cap as u32 + 10 {
        activity(&mut fx, w);
    }
    assert_eq!(fx.state.pending.len(), cap);
    // A window already holding keeps holding; a new one past the cap is answered at once.
    activity(&mut fx, first);
    assert_eq!(fx.state.pending[&first].events.len(), 2);
    let effects = fx.tool(EARLY, "task_done", done_args());
    assert_eq!(replies(&effects), vec![Err(not_worker())]);
    assert_eq!(fx.state.pending.len(), cap);
}

#[test]
fn a_held_call_of_another_role_is_refused_at_once() {
    let (mut fx, _) = launching();
    let effects = fx.tool_as(
        AgentRole::Reviewer,
        EARLY,
        "t1",
        "submit_review",
        verdict("approve", vec![]),
    );
    assert_eq!(
        replies(&effects),
        vec![Err("this window is not the reviewer of task t1".to_string())]
    );
    assert!(fx.state.pending.is_empty());
}

/// Task M9.8: a sub-planner's `submit_epic` joins the hold. A run's planner whose
/// `StartPlanner` is in flight, epic `mail`.
fn planner_launching() -> (Fixture, OpId) {
    let mut fx = super::orch::launched(false);
    super::planners::spawn(&mut fx, "mail");
    let (op, _) = fx.op("StartPlanner");
    (fx, op)
}

fn early_submit(fx: &mut Fixture) -> Vec<Effect> {
    let edits = serde_json::json!([super::planners::task_in("m1", "mail")]);
    super::planners::planner_tool(
        fx,
        (EARLY, "mail"),
        "submit_epic",
        serde_json::json!({ "edits": edits }),
    )
}

#[test]
fn a_planners_submit_before_its_window_is_answered_after_binding() {
    let (mut fx, op) = planner_launching();
    let effects = early_submit(&mut fx);
    assert!(replies(&effects).is_empty(), "held: {effects:#?}");
    let effects = fx.done(op, OpResult::PlannerStarted { window_id: EARLY });
    assert_eq!(
        replies(&effects),
        vec![Ok(crate::run::engine::planners::EPIC_RECORDED.to_string())]
    );
    assert!(effects.contains(&Effect::PlannerAccepted { window_id: EARLY }));
    assert_eq!(fx.task("m1").spec.epic.as_deref(), Some("mail"));
    assert!(fx.state.pending.is_empty());
}

#[test]
fn a_planners_held_submit_is_refused_when_its_launch_fails() {
    let (mut fx, op) = planner_launching();
    assert!(replies(&early_submit(&mut fx)).is_empty());
    let effects = fx.done(
        op,
        OpResult::Failed {
            message: "no binary".into(),
        },
    );
    let text = format!("this window is not the sub-planner of epic mail of run {RUN_ID}");
    let error = serde_json::json!({ "error": text }).to_string();
    assert_eq!(replies(&effects), vec![Err(error)]);
    assert!(fx.run().task("m1").is_none());
    assert!(fx.state.pending.is_empty());
}

/// Task M9.9: a research task's `submit_scout_report` joins the hold (the early-events
/// fix's default for a headless role's tool), so its first turn's report is not lost.
#[test]
fn a_research_report_before_its_window_is_answered_after_binding() {
    use super::kinds::{report_args, research, running, submit_report};
    let mut fx = running("", &[research("r1", "")]);
    let (op, _) = fx.op("CreateWindow");
    let effects = submit_report(&mut fx, EARLY, "r1", report_args());
    assert!(replies(&effects).is_empty(), "held: {effects:#?}");
    assert_eq!(fx.task("r1").state, TaskState::Working);
    let effects = bind(&mut fx, op, EARLY);
    assert_eq!(
        replies(&effects),
        vec![Ok(crate::scout::service::REPORT_ACCEPTED.to_string())]
    );
    assert_eq!(fx.task("r1").state, TaskState::Reported);
}

/// `call` of `role` from `window`, `mail`'s for a planner.
fn m9_call(role: AgentRole, window: u32, tool: &str) -> proto::ToolCall {
    proto::ToolCall {
        run_id: RUN_ID.into(),
        task_id: None,
        role,
        window_id: window,
        tool: tool.into(),
        args: serde_json::json!({}),
        scout_id: None,
        epic: (role == AgentRole::Planner).then(|| "mail".to_string()),
    }
}

/// Task M9.11 (review finding 1): a sub-planner's read before its `StartPlanner`
/// result names its window waits with the rule its `submit_epic` is held by, and so
/// does no call once the launch is over, bound or failed.
#[test]
fn a_planners_read_before_its_window_waits_for_its_launch() {
    use crate::run::engine::early::awaits_launch;
    for bound in [true, false] {
        let (mut fx, op) = planner_launching();
        let read = m9_call(AgentRole::Planner, EARLY, "get_context");
        assert!(awaits_launch(&fx.state, &read));
        assert!(awaits_launch(
            &fx.state,
            &m9_call(AgentRole::Planner, EARLY, "submit_epic")
        ));
        // Another role's call never waits for a sub-planner's launch.
        assert!(!awaits_launch(
            &fx.state,
            &m9_call(AgentRole::Worker, EARLY, "task_note")
        ));
        let result = if bound {
            OpResult::PlannerStarted { window_id: EARLY }
        } else {
            OpResult::Failed {
                message: "no binary".into(),
            }
        };
        fx.done(op, result);
        assert!(!awaits_launch(&fx.state, &read), "bound: {bound}");
    }
}

/// Task M9.11 (review finding 1): the orchestrator's calls, reads and writes, before
/// its `CreateOrchestrator` result names its window wait for it; once bound or failed
/// they do not, and the orchestrator's own window never waits.
#[test]
fn the_orchestrators_calls_before_its_window_wait_for_its_launch() {
    use super::orch::ORCH;
    use crate::run::engine::early::awaits_launch;
    for bound in [true, false] {
        let mut fx = super::orch::planned(false);
        let (op, _) = fx.op("CreateRunBranch");
        fx.done(op, OpResult::Worktree { head: BASE.into() });
        let (op, _) = fx.op("CreateOrchestrator");
        for tool in ["get_context", "run_status", "edit_plan"] {
            let call = m9_call(AgentRole::Orchestrator, ORCH, tool);
            assert!(awaits_launch(&fx.state, &call), "{tool}");
        }
        let result = if bound {
            OpResult::Window {
                window_id: ORCH,
                pid: None,
            }
        } else {
            OpResult::Failed {
                message: "no binary".into(),
            }
        };
        fx.done(op, result);
        for window in [ORCH, EARLY] {
            let call = m9_call(AgentRole::Orchestrator, window, "run_status");
            assert!(!awaits_launch(&fx.state, &call), "bound: {bound}, {window}");
        }
    }
}

/// Milestone 9 task M9.13a: a worker's `task_note` joins the hold (the PR #22 note's
/// default), so a note made in the session's first moments is recorded, not refused.
#[test]
fn a_task_note_before_the_window_is_recorded_after_binding() {
    let (mut fx, op) = launching();
    let reply = fx.reply();
    let call = proto::ToolCall {
        run_id: RUN_ID.into(),
        task_id: Some("t1".into()),
        role: AgentRole::Worker,
        window_id: EARLY,
        tool: "task_note".into(),
        args: serde_json::json!({"kind": "discovery", "text": "early"}),
        scout_id: None,
        epic: None,
    };
    let refusals = Vec::new();
    let event = crate::run::engine::OrchEvent::Tool {
        reply,
        call,
        refusals,
    };
    let effects = fx.next(EventKind::Orch(event));
    assert!(replies(&effects).is_empty(), "held: {effects:#?}");
    let effects = bind(&mut fx, op, EARLY);
    let note = crate::run::orch::contract::NOTE_RECORDED.to_string();
    assert_eq!(replies(&effects), vec![Ok(note)]);
    assert_eq!(fx.task("t1").orch.worker_notes.len(), 1);
}
