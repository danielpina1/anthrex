//! Milestone 9.6 task M9.6.14, ruling T14-1 (DF §8.4, review focus 1): a design run's
//! lost or restarted orchestrator window starts its fresh session with the stored first
//! prompt and, built at that moment, where the run is: its phase, its open gate and the
//! approved documents' paths. A run without the design flow relaunches as in 9.5, byte
//! for byte.

use proto::{RunState, Runtime};

use super::design_fixture::*;
use super::fixture::*;
use super::orch::{ORCH, launched, mcp_ready, planned_on};
use super::orch_restore::{restart, resume};
use crate::run::engine::{Effect, EventKind, OpKind, OpResult, OrchEvent};

/// The window a relaunch's `CreateOrchestrator` makes in these tests.
const FRESH: u32 = ORCH + 40;

/// Where the run is at its open plan gate, v1 ([`at_plan_gate`]).
fn at_the_plan_gate() -> String {
    let dir = format!("/tmp/data/runs/{RUN_ID}/design");
    format!(
        "Where the run is now (run_status has anything newer): phase planning; open gate: \
         plan v1, waiting for the user; approved documents: brainstorm v1 \
         ({dir}/brainstorm-v1.md), spec v1 ({dir}/spec-v1.md)."
    )
}

/// The first-turn wake-ups' texts.
fn first_turns(effects: &[Effect]) -> Vec<String> {
    (effects.iter())
        .filter_map(|e| match e {
            Effect::WakeOrchestrator {
                text,
                first_turn: true,
                ..
            } => Some(text.clone()),
            _ => None,
        })
        .collect()
}

/// The orchestrator's window was lost; the driver's handoff took it off the record.
fn lose_window(fx: &mut Fixture, window: u32) {
    fx.next(EventKind::Orch(OrchEvent::AdoptLost {
        run_id: RUN_ID.into(),
        window_id: window,
        first_prompt: "the handoff".into(),
    }));
}

/// `run resume` launches a fresh session: its `CreateOrchestrator`'s initial prompt (a
/// Codex window's; a Claude window starts with none), once its window is up.
fn relaunch(fx: &mut Fixture) -> Option<String> {
    let effects = resume(fx);
    let launches = ops_in(&effects, "CreateOrchestrator");
    assert_eq!(launches.len(), 1, "{effects:#?}");
    let (op, OpKind::CreateOrchestrator { spec, .. }) = launches[0].clone() else {
        unreachable!()
    };
    fx.done(
        op,
        OpResult::Window {
            window_id: FRESH,
            pid: None,
        },
    );
    spec.initial_prompt
}

fn stored_first_prompt(fx: &Fixture) -> String {
    fx.run()
        .orch
        .orchestrator
        .as_ref()
        .unwrap()
        .first_prompt
        .clone()
}

/// Ruling T14-1: at an open plan gate, a lost Claude window's fresh session is pasted
/// its handoff and the line naming the gate and the approved spec and brainstorm; the
/// stored first prompt stays the handoff, so a second relaunch has the line once.
#[test]
fn a_relaunch_at_an_open_plan_gate_names_it() {
    let mut fx = at_plan_gate(false);
    assert_eq!(fx.run().state, RunState::AwaitingApproval);
    lose_window(&mut fx, ORCH);
    assert_eq!(
        relaunch(&mut fx),
        None,
        "a Claude window starts with no prompt"
    );
    let effects = mcp_ready(&mut fx, FRESH);
    let sent = format!("the handoff\n{}", at_the_plan_gate());
    assert_eq!(first_turns(&effects), vec![sent.clone()]);
    assert_eq!(stored_first_prompt(&fx), "the handoff");

    super::orch::first_turn_woken(&mut fx);
    lose_window(&mut fx, FRESH);
    relaunch(&mut fx);
    let effects = mcp_ready(&mut fx, FRESH);
    assert_eq!(first_turns(&effects), vec![sent]);
}

/// A Codex orchestrator has its first prompt on its command line (ruling T5a-2): the
/// relaunch's carries the line too; and a fresh restart in its own window, whose command
/// line is the launch's, is told where the run is now as a wake note.
#[test]
fn a_codex_relaunch_at_an_open_plan_gate_names_it() {
    let mut fx = at_plan_gate(false);
    let route = &mut fx.run_mut().orch.orchestrator.as_mut().unwrap().route;
    route.runtime = Runtime::Codex;
    lose_window(&mut fx, ORCH);
    let sent = format!("the handoff\n{}", at_the_plan_gate());
    assert_eq!(relaunch(&mut fx), Some(sent));
    assert_eq!(stored_first_prompt(&fx), "the handoff");

    restart(&mut fx);
    let effects = resume(&mut fx);
    let restarts = ops_in(&effects, "RestartOrchestrator");
    assert_eq!(restarts.len(), 1, "{effects:#?}");
    fx.done(restarts[0].0, OpResult::RestartedFresh);
    assert!(notes(&fx).contains(&at_the_plan_gate()), "{:?}", notes(&fx));
}

/// A design run's first session is told where it is as well: brainstorming, nothing
/// open, nothing approved.
#[test]
fn a_design_runs_first_session_is_told_where_it_is() {
    let mut fx = design_planned(false);
    let (op, _) = fx.op("CreateRunBranch");
    fx.done(op, OpResult::Worktree { head: BASE.into() });
    let (op, _) = fx.op("CreateOrchestrator");
    fx.done(
        op,
        OpResult::Window {
            window_id: ORCH,
            pid: None,
        },
    );
    let first = stored_first_prompt(&fx);
    let effects = mcp_ready(&mut fx, ORCH);
    let line = "Where the run is now (run_status has anything newer): phase brainstorming; \
                open gate: none; approved documents: none.";
    let step = "Then follow rule 48.";
    assert_eq!(
        first_turns(&effects),
        vec![format!("{first}\n{line}\n{step}")]
    );
}

/// Ruling T14-3: a relaunch in specifying (past the questions) is sent its stored first
/// prompt and where the run is, and no step back to rule 48: a Claude window's fresh
/// restart in its own window, which pastes the run's own first prompt again.
#[test]
fn a_relaunch_in_specifying_is_not_sent_back_to_rule_48() {
    let mut fx = at_brainstorm_gate(false);
    act(
        &mut fx,
        proto::DocGateKind::Brainstorm,
        proto::DocGateAction::Approve,
    )
    .unwrap();
    assert_eq!(fx.run().state, RunState::Specifying);
    let first = stored_first_prompt(&fx);
    restart(&mut fx);
    let effects = resume(&mut fx);
    let restarts = ops_in(&effects, "RestartOrchestrator");
    assert_eq!(restarts.len(), 1, "{effects:#?}");
    fx.done(restarts[0].0, OpResult::RestartedFresh);
    let effects = mcp_ready(&mut fx, ORCH);
    let dir = format!("/tmp/data/runs/{RUN_ID}/design");
    let line = format!(
        "Where the run is now (run_status has anything newer): phase specifying; open gate: \
         none; approved documents: brainstorm v1 ({dir}/brainstorm-v1.md)."
    );
    let sent = first_turns(&effects);
    assert_eq!(sent, vec![format!("{first}\n{line}")]);
    assert!(!sent[0].contains("follow rule 48"), "{sent:?}");
}

/// Ruling T14-1: a run without the design flow relaunches as in 9.5: the handoff alone,
/// pasted (Claude) or on the command line (Codex), and a Codex fresh restart gets no
/// note.
#[test]
fn a_non_design_relaunch_is_byte_identical() {
    let mut fx = launched(false);
    lose_window(&mut fx, ORCH);
    assert_eq!(relaunch(&mut fx), None);
    let effects = mcp_ready(&mut fx, FRESH);
    assert_eq!(first_turns(&effects), vec!["the handoff".to_string()]);

    let mut fx = planned_on(false, Some(Runtime::Codex));
    let (op, _) = fx.op("CreateRunBranch");
    fx.done(op, OpResult::Worktree { head: BASE.into() });
    let (op, _) = fx.op("CreateOrchestrator");
    fx.done(
        op,
        OpResult::Window {
            window_id: ORCH,
            pid: None,
        },
    );
    lose_window(&mut fx, ORCH);
    assert_eq!(relaunch(&mut fx).as_deref(), Some("the handoff"));
    let before = fx.run().orch.orchestrator.as_ref().unwrap().notes.clone();
    restart(&mut fx);
    let effects = resume(&mut fx);
    let restarts = ops_in(&effects, "RestartOrchestrator");
    fx.done(restarts[0].0, OpResult::RestartedFresh);
    let after = fx.run().orch.orchestrator.as_ref().unwrap().notes.clone();
    assert!(
        !after.iter().any(|n| n.contains("Where the run is")),
        "{before:?} -> {after:?}"
    );
}
