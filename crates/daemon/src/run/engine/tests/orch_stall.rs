//! Milestone 9.9 task M9.9.6: a stalled or dead orchestrator is reported stuck.

use proto::OrchestratorStuck;
use serde_json::json;

use super::fixture::*;
use super::orch::{ORCH, orch_tool};
use super::wake_notes::approved;
use crate::run::engine::{EventKind, OrchEvent};

fn stuck(fx: &Fixture) -> Option<OrchestratorStuck> {
    crate::run::snapshot::snapshot(&fx.state, fx.now).runs[0]
        .orchestrator
        .as_ref()
        .unwrap()
        .stuck
}

/// The orchestrator acts (so the approval's note is answered), then the user edits:
/// one note, "the user edited the plan", unanswered from now.
fn user_edit(fx: &mut Fixture) {
    orch_tool(fx, ORCH, "edit_plan", json!({"edits": []}));
    assert_eq!(stuck(fx), None);
    super::dispatch::edit(
        fx,
        vec![proto::PlanEdit::CancelTask {
            task_id: "t2".into(),
        }],
    );
}

#[test]
fn a_note_unanswered_for_stall_after_secs_stalls_it_and_a_tool_call_clears_it() {
    let mut fx = approved();
    let stall = fx.run().limits.stall_after_secs;
    user_edit(&mut fx);
    let since = fx.now;
    fx.send(since + stall - 1, EventKind::Tick);
    assert_eq!(stuck(&fx), None);
    let before = fx.run().revision;
    fx.send(since + stall, EventKind::Tick);
    assert_eq!(stuck(&fx), Some(OrchestratorStuck::Stalled { since }));
    assert!(fx.run().revision > before, "the change is published");
    assert!(fx.run().log.iter().any(|e| e.text
        == format!(
            "the orchestrator has not acted for {} min; its alerts go to you",
            stall / 60
        )));
    orch_tool(&mut fx, ORCH, "edit_plan", json!({"edits": []}));
    assert_eq!(stuck(&fx), None);
    assert!(
        fx.run()
            .log
            .iter()
            .any(|e| e.text == "the orchestrator is acting again")
    );
}

#[test]
fn a_digest_read_counts_as_acting() {
    let mut fx = approved();
    let stall = fx.run().limits.stall_after_secs;
    user_edit(&mut fx);
    let since = fx.now;
    let seq = crate::run::engine::notes_seq(fx.run());
    fx.next(EventKind::Orch(OrchEvent::DigestRead {
        run_id: RUN_ID.into(),
        digest_revision: fx.run().orch.digest_rev,
        notes_seq: seq,
        at: fx.now,
    }));
    fx.send(since + stall + 5, EventKind::Tick);
    assert_eq!(stuck(&fx), None);
}

#[test]
fn an_exited_window_on_a_running_run_is_dead() {
    let mut fx = approved();
    let launch = fx.run().orch.orchestrator.as_ref().unwrap().launches;
    fx.next(EventKind::Orch(OrchEvent::OrchestratorWindow {
        run_id: RUN_ID.into(),
        window_id: ORCH,
        live: false,
        launch,
    }));
    assert!(matches!(stuck(&fx), Some(OrchestratorStuck::Dead { .. })));
}

#[test]
fn a_complete_run_with_an_exited_window_is_not_stuck() {
    let mut fx = super::actions_fixtures::complete_orchestrated();
    let launch = fx.run().orch.orchestrator.as_ref().unwrap().launches;
    fx.next(EventKind::Orch(OrchEvent::OrchestratorWindow {
        run_id: RUN_ID.into(),
        window_id: ORCH,
        live: false,
        launch,
    }));
    assert!(!fx.run().orch.orchestrator.as_ref().unwrap().live);
    assert_eq!(stuck(&fx), None);
}
