//! Milestone 9 task M9.13b review fixes (decision 43): how an orchestrator session's
//! record ends when its window exits while the run goes on.

use proto::{AgentRole, RoleOutcome};
use serde_json::json;

use super::fixture::*;
use super::orch::{ORCH, add, edit_plan, launched};
use super::role_history::{of_role, role_lines, with_history};
use crate::run::engine::{Effect, EventKind, OrchEvent};

fn window_exits(fx: &mut Fixture) -> Vec<Effect> {
    let launch = fx.run().orch.orchestrator.as_ref().unwrap().launches;
    fx.next(EventKind::Orch(OrchEvent::OrchestratorWindow {
        run_id: RUN_ID.into(),
        window_id: ORCH,
        live: false,
        launch,
    }))
}

/// Review M-1: `completed` means the orchestrator had delivered what its session is for,
/// its plan, when its window exited (or it was live until the run ended); an exit
/// before it submitted a plan is `failed`.
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
        Some("its window exited before it submitted a plan")
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
        Some("its window exited after it submitted the plan, before the run ended")
    );
}
