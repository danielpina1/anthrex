//! Milestone 9.6 task M9.6.10's fixtures: a design run in specifying, the
//! orchestrator's spec, the document reviewer's findings and session end, and readers
//! of the stored versions. The reviewer's session is stubbed by its op's result and its
//! ended event.

use proto::{AgentRole, DocGateAction, DocGateKind, DocKind, RunState, Runtime};
use proto::{TokenUsage, ToolCall};
use serde_json::{Value, json};

use super::design_agents::{launches, started};
use super::design_fixture::*;
use super::dispatch::replies;
use super::fixture::*;
use super::orch::{ORCH, orch_tool};
use crate::run::design::state::{DesignState, DocVersion};
use crate::run::engine::{Effect, EventKind, OrchEvent, ScoutEnd};

/// The document reviewer's window in these tests.
pub(super) const REVIEWER: u32 = 901;

/// A design run in specifying: its brainstorm report approved, and a Codex model at
/// its Claude orchestrator's strength in its roster, its peer.
pub(super) fn specifying() -> Fixture {
    let mut fx = at_brainstorm_gate(false);
    fx.run_mut().roster.push(proto::ModelEntry {
        runtime: Runtime::Codex,
        model: "gpt-6".into(),
        strength: proto::Strength::Frontier,
        note: String::new(),
    });
    act(&mut fx, DocGateKind::Brainstorm, DocGateAction::Approve).unwrap();
    assert_eq!(fx.run().state, RunState::Specifying);
    fx
}

/// The orchestrator's spec, `ready` or a review draft, with `responses` when given.
pub(super) fn submit_spec(fx: &mut Fixture, ready: bool, responses: Value) -> Vec<Effect> {
    let mut args = json!({"kind": "spec", "text": SPEC, "ready": ready});
    if !responses.is_null() {
        args["responses"] = responses;
    }
    orch_tool(fx, ORCH, "submit_doc", args)
}

/// A tool call's one answer: its JSON, or its refusal's text.
pub(super) fn outcome(effects: &[Effect]) -> Result<Value, String> {
    match &replies(effects)[..] {
        [Ok(text)] => Ok(serde_json::from_str(text).unwrap()),
        [Err(text)] => {
            let value: Value = serde_json::from_str(text).unwrap();
            Err(value["error"].as_str().unwrap_or_default().to_string())
        }
        other => panic!("{other:?}"),
    }
}

/// The document reviewer's `submit_findings` from `window`.
pub(super) fn submit_findings(fx: &mut Fixture, window: u32, findings: Value) -> Vec<Effect> {
    let reply = fx.reply();
    fx.next(EventKind::Orch(OrchEvent::Tool {
        reply,
        call: ToolCall {
            run_id: RUN_ID.into(),
            task_id: None,
            role: AgentRole::DocReviewer,
            window_id: window,
            tool: "submit_findings".into(),
            args: json!({ "findings": findings }),
            scout_id: None,
            epic: None,
            chain: None,
            lane: None,
        },
        refusals: Vec::new(),
    }))
}

/// Three findings, one of them blocking.
pub(super) fn three_findings() -> Value {
    json!([
        {"id": "F1", "severity": "blocking", "place": "## Requirements", "text": "R2 has no check."},
        {"id": "F2", "severity": "minor", "place": "## Risks", "text": "Name a second risk."},
        {"id": "F3", "severity": "minor", "place": "## Testing", "text": "Say which suite."},
    ])
}

/// Every finding answered `fixed`, but `kept` for the ids given.
pub(super) fn answers(ids: &[&str], kept: &[&str]) -> Value {
    let all: Vec<Value> = (ids.iter())
        .map(|id| match kept.contains(id) {
            true => json!({"id": id, "answer": "kept: out of scope"}),
            false => json!({"id": id, "answer": "fixed"}),
        })
        .collect();
    Value::Array(all)
}

/// `session` of the document reviewer `label` ended.
pub(super) fn reviewer_ended(
    fx: &mut Fixture,
    label: &str,
    session: u32,
    outcome: ScoutEnd,
) -> Vec<Effect> {
    fx.next(EventKind::Orch(OrchEvent::DesignAgentEnded {
        run_id: RUN_ID.into(),
        role: AgentRole::DocReviewer,
        label: label.into(),
        session,
        outcome,
        usage: TokenUsage::default(),
        calls: 3,
    }))
}

/// A spec draft sent to review, its reviewer live in `window` and its `findings` in.
pub(super) fn reviewed(fx: &mut Fixture, window: u32, findings: Value) {
    outcome(&submit_spec(fx, false, Value::Null)).unwrap();
    let label = launches(fx).last().unwrap().1.kind.label();
    started(fx, &label, window);
    outcome(&submit_findings(fx, window, findings)).unwrap();
}

pub(super) fn design(fx: &Fixture) -> &DesignState {
    fx.run().orch.design.as_ref().unwrap()
}

/// The spec's gate version `n`.
pub(super) fn spec_v(fx: &Fixture, n: u32) -> DocVersion {
    design(fx).find(DocKind::Spec, Some(n)).unwrap().clone()
}

/// The paths of every `WriteDoc` in `effects`, file names only.
pub(super) fn written(effects: &[Effect]) -> Vec<String> {
    (effects.iter())
        .filter_map(|e| match e {
            Effect::WriteDoc { path, .. } => path.file_name().map(|n| n.display().to_string()),
            _ => None,
        })
        .collect()
}
