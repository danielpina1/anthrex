//! Milestone 9 task M9.13b review fixes (decision 43): how an orchestrator session's
//! record ends when its window exits while the run goes on.

use proto::{AgentRole, RoleOutcome};
use serde_json::json;

use super::fixture::*;
use super::orch::{ORCH, add, edit_plan, launched};
use super::planners::{planner_ended, planning_mail, submit_epic, task_in};
use super::role_history::{of_role, role_lines, with_history};
use crate::run::engine::{Effect, EventKind, OrchEvent, ScoutEnd};

fn window_exits(fx: &mut Fixture) -> Vec<Effect> {
    let launch = fx.run().orch.orchestrator.as_ref().unwrap().launches;
    fx.next(EventKind::Orch(OrchEvent::OrchestratorWindow {
        run_id: RUN_ID.into(),
        window_id: ORCH,
        live: false,
        launch,
    }))
}

/// Review M-1: `completed` means the run's plan had been submitted when the
/// orchestrator's window exited (or it was live until the run ended); an exit before
/// it was is `failed`. Re-review 4: the result says what is known of the run's plan,
/// not that this session submitted it (`plan_submitted` is the run's).
#[test]
fn an_orchestrator_exit_is_failed_before_its_plan_and_completed_after() {
    let mut fx = launched(false);
    with_history(&mut fx);
    let effects = window_exits(&mut fx);
    let lines = role_lines(&effects);
    assert_eq!(lines.len(), 1, "{effects:#?}");
    let d = &lines[0].1;
    assert_eq!(d.outcome, Some(RoleOutcome::Failed));
    assert_eq!(
        d.result.as_deref(),
        Some("its window exited before the run's plan was submitted")
    );

    let mut fx = launched(false);
    with_history(&mut fx);
    edit_plan(
        &mut fx,
        json!({"edits": [add("t1", "auth")], "submit": true}),
    );
    window_exits(&mut fx);
    let records = of_role(&fx, AgentRole::Orchestrator);
    assert_eq!(records[0].outcome, Some(RoleOutcome::Completed));
    assert_eq!(
        records[0].result.as_deref(),
        Some("its window exited after the run's plan had been submitted, before the run ended")
    );
}

fn cancel(fx: &mut Fixture) -> Vec<Effect> {
    let reply = fx.reply();
    fx.next(EventKind::Cancel {
        reply,
        run_id: RUN_ID.into(),
    })
}

/// Re-review 1: `run cancel` stops only the sessions it halts. A sub-planner whose
/// epic was accepted and whose process is still ending is not stopped by it: its
/// session still ends `completed`, `epic accepted`.
#[test]
fn cancel_leaves_an_accepted_planner_to_its_own_end() {
    let mut fx = planning_mail(false);
    with_history(&mut fx);
    submit_epic(&mut fx, json!([task_in("t2", "mail")]));
    cancel(&mut fx);
    let open = of_role(&fx, AgentRole::Planner);
    assert_eq!(open[0].outcome, None, "{open:#?}");
    let effects = planner_ended(&mut fx, ("mail", 1), ScoutEnd::Reported);
    let d = role_lines(&effects).remove(0).1;
    assert_eq!(d.outcome, Some(RoleOutcome::Completed));
    assert_eq!(d.result.as_deref(), Some("epic accepted"));
}

/// Re-review 1: a sub-planner that failed at `max_rejections` (stopped by the engine,
/// its process still ending) keeps its own failure when the run is then cancelled.
#[test]
fn cancel_leaves_a_planner_failed_at_max_rejections_to_its_own_end() {
    let mut fx = planning_mail(false);
    with_history(&mut fx);
    fx.run_mut().limits.orch.planners.max_rejections = 1;
    submit_epic(&mut fx, json!([task_in("t2", "web")]));
    cancel(&mut fx);
    assert_eq!(of_role(&fx, AgentRole::Planner)[0].outcome, None);
    let reason = "the sub-planner's epic was rejected 1 times";
    let ended = ScoutEnd::Failed {
        reason: reason.into(),
    };
    let d = role_lines(&planner_ended(&mut fx, ("mail", 1), ended))
        .remove(0)
        .1;
    assert_eq!(d.outcome, Some(RoleOutcome::Failed));
    assert_eq!(
        d.result.as_deref(),
        Some("the sub-planner's epic was rejected 1 times; 1 submissions rejected")
    );
}
