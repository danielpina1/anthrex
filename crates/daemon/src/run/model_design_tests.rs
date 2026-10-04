//! Task M9.6.3: the design flow's per-run fields persist through `run.json`
//! (`journal::save_run`, the driver's serializer, and `journal::load_all`): the frozen
//! mode (`Run.design_mode`), the frozen design limits (`OrchLimits.design`) and the
//! frozen `brainstorm` model list (carry M-4). A protocol-16 (9.5) `run.json` loads with
//! the mode `Off` and writes none of them back.

use proto::{DesignMode, Effort, Runtime, Strength};

use super::tuning::tests::{count, keys, save_and_load, tmp};
use super::{FrozenList, ListCandidate, ListPolicy, RouteListsFrozen, Run};
use crate::run::orch::DesignLimits;

/// A planned goal's `run.json` as milestone 9.5's code wrote it at the start (state
/// `planning`, an orchestrator record, a frozen `review` list), captured in task
/// M9.6.3 (commit 00ec0f18) before any of its changes.
const M95_RUN: &str = include_str!("../../tests/fixtures/run/m95-run.json");

/// Every key task M9.6.3 adds to the persisted run.
const NEW_KEYS: [&str; 3] = ["design_mode", "design", "brainstorm"];

fn old_run() -> Run {
    serde_json::from_str(M95_RUN).expect("m95-run.json parses")
}

fn candidate(model: &str, strength: Strength, effort: Option<Effort>) -> ListCandidate {
    ListCandidate {
        runtime: Runtime::Claude,
        model: model.into(),
        strength,
        effort,
    }
}

#[test]
fn an_old_run_json_loads_with_design_off() {
    let mut run = old_run();
    assert_eq!(run.state, proto::RunState::Planning);
    assert!(run.orch.orchestrator.is_some(), "a planned goal's run");
    assert_eq!(run.design_mode, DesignMode::Off);
    assert_eq!(run.limits.orch.design, DesignLimits::default());
    assert!(run.limits.route_lists.brainstorm.is_empty());
    assert!(!run.limits.route_lists.review.is_empty());

    // Written again, it is the JSON 9.5 wrote, so it has none of the new keys.
    let dir = tmp();
    let (loaded, text) = save_and_load(&mut run, dir.path());
    assert_eq!(loaded, run);
    let mut again: serde_json::Value = serde_json::from_str(&text).expect("run.json is JSON");
    let captured: serde_json::Value = serde_json::from_str(M95_RUN).expect("fixture");
    let written = keys(&again);
    for key in NEW_KEYS {
        assert_eq!(count(&written, key), 0, "run.json gained {key}");
    }
    // `save_and_load` moved the run's data directory under the temp dir.
    again["data_dir"] = captured["data_dir"].clone();
    assert_eq!(again, captured);
}

#[test]
fn a_design_run_round_trips() {
    let mut run = old_run();
    run.design_mode = DesignMode::Full;
    run.limits.orch.design = DesignLimits {
        docs_dir: String::new(),
        commit_brainstorm: true,
        max_questions: 2,
        phase_minutes: 90,
        brainstormer: proto::Budget {
            tool_calls: 60,
            minutes: 20,
            tokens: Some(1000),
        },
        doc_reviewer: proto::Budget {
            tool_calls: 10,
            minutes: 5,
            tokens: None,
        },
    };
    run.limits.route_lists.brainstorm = FrozenList {
        candidates: vec![
            candidate("claude-opus-5-5", Strength::Frontier, Some(Effort::High)),
            candidate("claude-sonnet-5", Strength::Standard, None),
        ],
        pick: ListPolicy::First,
    };

    let dir = tmp();
    let (loaded, text) = save_and_load(&mut run, dir.path());
    assert_eq!(loaded, run);
    let written = keys(&serde_json::from_str(&text).expect("run.json is JSON"));
    for key in NEW_KEYS {
        assert!(count(&written, key) > 0, "run.json lacks {key}");
    }
    assert!(text.contains("\"design_mode\":\"full\""), "{text}");
}

/// Carry M-4: the `brainstorm` list is the ninth frozen list, frozen from config with
/// each candidate's roster strength, named last, and kept by a save and load.
#[test]
fn a_frozen_brainstorm_list_survives_save_and_load() {
    let (config, problems) = config::parse(
        r#"
[orchestrator.routes.brainstorm]
candidates = [
  { runtime = "claude", model = "claude-opus-5-5", effort = "high" },
  { runtime = "claude", model = "claude-sonnet-5" },
]
"#,
    );
    assert!(problems.is_empty(), "{problems:?}");
    let o = &config.orchestrator;
    let frozen = RouteListsFrozen::freeze(&o.tuning.routes, &o.models);
    assert_eq!(
        frozen.brainstorm,
        FrozenList {
            candidates: vec![
                candidate("claude-opus-5-5", Strength::Frontier, Some(Effort::High)),
                candidate("claude-sonnet-5", Strength::Standard, None),
            ],
            pick: ListPolicy::First,
        }
    );
    let named = frozen.named();
    assert_eq!(named.len(), 9);
    assert_eq!(named[8], ("brainstorm", &frozen.brainstorm));
    assert!(!frozen.is_empty(), "a brainstorm list alone is a list");

    let mut run = old_run();
    run.limits.route_lists = frozen.clone();
    let dir = tmp();
    let (loaded, text) = save_and_load(&mut run, dir.path());
    assert_eq!(loaded.limits.route_lists, frozen);
    assert!(text.contains("\"brainstorm\""), "{text}");
}

/// Task M9.6.5: a design run's state (`RunOrch.design`) survives a save and load: the
/// open gate, the version index, a review, an agent and the approved requirements.
/// A run without it writes no design state.
#[test]
fn the_state_survives_save_and_load() {
    use crate::run::design::state::{
        DesignAgent, DesignAgentState, DesignState, DocGate, DocReviewRecord, DocVersion,
        Requirement,
    };
    use proto::{AgentRole, DocAuthor, DocFinding, DocGateKind, DocKind, DocSeverity, Route};

    let finding = DocFinding {
        id: "F1".into(),
        severity: DocSeverity::Minor,
        place: "R1".into(),
        text: "vague".into(),
    };
    let version = |kind, n| DocVersion {
        kind,
        n,
        author: DocAuthor::Orchestrator,
        reason: "ready".into(),
        bytes: 40,
        sha256: "ab".repeat(32),
        at: 3_000 + u64::from(n),
        requirements: vec!["R1".into()],
        disputed: vec![finding.clone()],
        not_reviewed: Some("the reviewer timed out".into()),
        changes: vec!["+ R1".into()],
        same_runtime: true,
    };
    let route = Route {
        runtime: Runtime::Codex,
        model: "gpt-6".into(),
        strength: Strength::Frontier,
        effort: Effort::High,
    };
    let design = DesignState {
        phase_started: Some(3_100),
        phase_paused_base: 12,
        answers: Some("keep it small".into()),
        brainstormers: vec![DesignAgent {
            label: "codex".into(),
            role: AgentRole::Brainstormer,
            route: route.clone(),
            session: 1,
            window_id: Some(9),
            state: DesignAgentState::Failed("timed out".into()),
            calls: 4,
            tokens: 900,
            started: Some(3_000),
        }],
        reviewer: Some(DesignAgent {
            label: "spec-r1".into(),
            role: AgentRole::DocReviewer,
            route,
            session: 1,
            window_id: None,
            state: DesignAgentState::Done,
            calls: 2,
            tokens: 100,
            started: None,
        }),
        reviews: vec![DocReviewRecord {
            doc: DocKind::Spec,
            n: 1,
            findings: vec![finding.clone()],
            failed: None,
        }],
        versions: vec![version(DocKind::Brainstorm, 1), version(DocKind::Spec, 2)],
        gate: Some(DocGate {
            kind: DocGateKind::Spec,
            version: 2,
            opened_at: 3_200,
            revising: Some("shorter".into()),
            review: true,
            // Task M9.6.7 fix round 1 (m3).
            cause: crate::run::design::state::Revision::Back,
        }),
        requirements: vec![Requirement {
            id: "R1".into(),
            text: "one".into(),
        }],
        goal_section: "Reset passwords.".into(),
        plan_review_done: true,
        commit_due: true,
        committed: Some("c".repeat(40)),
        // Task M9.6.7: a budget halt's state and the rethinks; the texts' cache is
        // never written.
        halted_from: Some(proto::RunState::Specifying),
        rethinks: 2,
        texts: Vec::new(),
    };
    let mut run = old_run();
    run.design_mode = DesignMode::Full;
    run.state = proto::RunState::AwaitingApproval;
    run.orch.design = Some(design.clone());
    let dir = tmp();
    let (loaded, text) = save_and_load(&mut run, dir.path());
    assert_eq!(loaded.orch.design, Some(design));
    assert_eq!(loaded, run);
    let json: serde_json::Value = serde_json::from_str(&text).expect("run.json is JSON");
    assert_eq!(json["orch"]["design"]["gate"]["kind"], "spec");
    assert_eq!(json["orch"]["design"]["halted_from"], "specifying");
    assert_eq!(json["orch"]["design"]["gate"]["cause"], "back");
    assert!(json["orch"]["design"].get("texts").is_none());

    // Without it, `orch` has no `design` key.
    let mut plain = old_run();
    let (loaded, text) = save_and_load(&mut plain, dir.path());
    assert_eq!(loaded.orch.design, None);
    let json: serde_json::Value = serde_json::from_str(&text).expect("run.json is JSON");
    assert!(json["orch"].get("design").is_none(), "{}", json["orch"]);
}
