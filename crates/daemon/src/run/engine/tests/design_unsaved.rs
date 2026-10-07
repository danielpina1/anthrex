//! Milestone 9.6 ruling WB-A-W2 (the final fix wave's FW-24 and FW-25, safety): a
//! design agent never runs with session persistence on. With no installed runtime that
//! can run one unsaved, the brainstorm, or the document review, halts with its exact
//! text and nothing is queued; there is no fallback to a runtime that saves its session.

use proto::{RunState, Runtime};

use crate::run::engine::ScoutEnd;
use serde_json::{Value, json};

use super::design_agents::{agents, launches};
use super::design_fixture::*;
use super::design_plan_fixture::*;
use super::design_review_fixture::{outcome, specifying, submit_spec};
use super::fixture::*;
use super::orch::{ORCH, orch_tool};
use crate::decider::argv::with_caps;
use crate::decider::{DECIDER_CAPS, DeciderCaps};
use crate::run::orch::roles::lists::{UNSAVED_BRAINSTORMER, UNSAVED_REVIEWER};

/// Caps with `claude --no-session-persistence` and `codex exec --ephemeral` as given.
fn caps(claude: bool, codex: bool) -> DeciderCaps {
    DeciderCaps {
        claude_no_session_persistence: claude,
        codex_ephemeral: codex,
        ..DECIDER_CAPS
    }
}

/// The run halted with `text`, as every halt is.
fn halted_with(fx: &Fixture, text: &str) {
    assert_eq!(fx.run().state, RunState::Halted);
    assert_eq!(fx.run().halted_reason.as_deref(), Some(text));
    assert!(
        log_lines(fx)
            .iter()
            .any(|l| *l == format!("halted: {text}"))
    );
}

#[test]
fn no_runtime_that_runs_unsaved_halts_the_brainstorm() {
    let exact =
        "design flow: no installed runtime can run a brainstormer without saving its session";
    assert_eq!(UNSAVED_BRAINSTORMER, exact);
    let mut fx = design_launched(false);
    let args = json!({"answers": "skip"});
    let effects = with_caps(caps(false, false), || {
        orch_tool(&mut fx, ORCH, "start_brainstorm", args)
    });
    assert_eq!(outcome(&effects), Err(exact.to_string()));
    halted_with(&fx, exact);
    assert!(agents(&fx).is_empty(), "{:?}", agents(&fx));
    assert!(launches(&fx).is_empty());
    // The answers are not taken, so a resumed run can start its brainstorm again.
    assert_eq!(fx.run().orch.design.as_ref().unwrap().answers, None);
}

/// The roster has no model of the one runtime that runs unsaved: the orchestrator's own
/// route, whose runtime saves its sessions, is no fallback.
#[test]
fn the_orchestrators_saving_runtime_is_no_brainstorm_fallback() {
    let mut fx = design_launched(false);
    fx.run_mut().roster.retain(|e| e.runtime != Runtime::Codex);
    let args = json!({"answers": "skip"});
    let effects = with_caps(caps(false, true), || {
        orch_tool(&mut fx, ORCH, "start_brainstorm", args)
    });
    assert_eq!(outcome(&effects), Err(UNSAVED_BRAINSTORMER.to_string()));
    halted_with(&fx, UNSAVED_BRAINSTORMER);
    assert!(launches(&fx).is_empty());
}

/// One runtime runs unsaved: both brainstormers run on it.
#[test]
fn one_runtime_that_runs_unsaved_runs_both_brainstormers() {
    let mut fx = design_launched(false);
    let args = json!({"answers": "skip"});
    let effects = with_caps(caps(true, false), || {
        orch_tool(&mut fx, ORCH, "start_brainstorm", args)
    });
    outcome(&effects).unwrap();
    let runtimes: Vec<Runtime> = agents(&fx).iter().map(|a| a.route.runtime).collect();
    assert_eq!(runtimes, [Runtime::Claude, Runtime::Claude]);
    assert_eq!(fx.run().state, RunState::Brainstorming);
}

#[test]
fn no_runtime_that_runs_unsaved_halts_the_spec_review() {
    let exact =
        "design flow: no installed runtime can run a document reviewer without saving its session";
    assert_eq!(UNSAVED_REVIEWER, exact);
    let mut fx = specifying();
    let before = launches(&fx).len();
    let effects = with_caps(caps(false, false), || {
        submit_spec(&mut fx, false, Value::Null)
    });
    assert_eq!(outcome(&effects), Err(exact.to_string()));
    halted_with(&fx, exact);
    let design = fx.run().orch.design.as_ref().unwrap();
    assert!(design.reviewer.is_none(), "{:?}", design.reviewer);
    assert!(design.reviews.is_empty());
    assert!(design.draft(proto::DocKind::Spec, 1).is_none());
    assert_eq!(launches(&fx).len(), before);
}

/// Ruling WB-B m4: the peer is not installed, and the orchestrator's own runtime saves
/// its sessions: no fallback to it.
#[test]
fn the_orchestrators_saving_runtime_is_no_review_fallback() {
    let mut fx = specifying();
    fx.run_mut().orch.installed = [("codex".to_string(), false)].into();
    let effects = with_caps(caps(false, true), || {
        submit_spec(&mut fx, false, Value::Null)
    });
    assert_eq!(outcome(&effects), Err(UNSAVED_REVIEWER.to_string()));
    halted_with(&fx, UNSAVED_REVIEWER);
    assert!(fx.run().orch.design.as_ref().unwrap().reviewer.is_none());
}

#[test]
fn no_runtime_that_runs_unsaved_halts_the_plan_review() {
    let mut fx = planning();
    let before = launches(&fx).len();
    let edits = json!([covering("t1", &["R1", "R2"])]);
    let refused = with_caps(caps(false, false), || {
        plan_submit(&mut fx, edits, Value::Null)
    });
    assert_eq!(refused, Err(UNSAVED_REVIEWER.to_string()));
    halted_with(&fx, UNSAVED_REVIEWER);
    let design = fx.run().orch.design.as_ref().unwrap();
    assert!(design.reviewer.is_none());
    assert!(!design.plan_review_done);
    assert_eq!(launches(&fx).len(), before);
}

/// WB-C M-2 (the final fix wave's FW-44): a reviewer picked on the orchestrator's own
/// runtime is recorded `same_runtime`, as its review says, even when the orchestrator's
/// stored route differs from the reviewer's by its relaunch.
#[test]
fn a_same_runtime_reviewers_record_states_its_reason() {
    use super::design_agents::started;
    use super::design_review_fixture::{REVIEWER, reviewer_ended};
    use crate::scout::design_spec::DOC_REVIEWER_TEXTS;
    let mut fx = specifying();
    let effects = with_caps(caps(true, false), || {
        submit_spec(&mut fx, false, Value::Null)
    });
    outcome(&effects).unwrap();
    let design = fx.run().orch.design.as_ref().unwrap();
    assert!(design.reviews.last().unwrap().same_runtime);
    let reviewer = design.reviewer.clone().unwrap();
    assert_eq!(reviewer.route.runtime, Runtime::Claude);
    started(&mut fx, "spec-r1", REVIEWER);
    let orchestrator = fx.run_mut().orch.orchestrator.as_mut().unwrap();
    orchestrator.route.effort = if orchestrator.route.effort == proto::Effort::LOW {
        proto::Effort::HIGH
    } else {
        proto::Effort::LOW
    };
    let reason = crate::scout::machine::unsubmitted(&DOC_REVIEWER_TEXTS);
    reviewer_ended(&mut fx, "spec-r1", 1, ScoutEnd::Unsubmitted { reason });
    let sources: Vec<(String, String)> = (fx.run().role_routing_decisions.iter())
        .filter(|d| d.role == proto::AgentRole::DocReviewer)
        .map(|d| (d.session_id.clone(), d.source.clone()))
        .collect();
    let same = |s: &str| (s.to_string(), "same_runtime".to_string());
    assert_eq!(sources, [same("spec-r1/1"), same("spec-r1/2")]);
}
