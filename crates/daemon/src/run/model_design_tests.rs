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
        report: Some(proto::ReportSummary {
            agree: 1,
            disagree: 2,
            approaches: vec![proto::ApproachTag {
                name: "Stored tokens".into(),
                tag: "both".into(),
            }],
        }),
        draft_review: None,
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
            listed: false,
            unsubmitted: false,
        }],
        reviewer: Some(DesignAgent {
            label: "spec-r1".into(),
            role: AgentRole::DocReviewer,
            route,
            session: 1,
            window_id: None,
            state: DesignAgentState::Running,
            calls: 2,
            tokens: 100,
            started: None,
            listed: false,
            // Task 8's re-review (m2): a relaunch's mark goes to disk and back.
            unsubmitted: true,
        }),
        reviews: vec![DocReviewRecord {
            doc: DocKind::Spec,
            n: 1,
            findings: vec![finding.clone()],
            failed: None,
            // Task M9.6.10: the cycle the review belongs to, and its runtime.
            after: 1,
            same_runtime: true,
        }],
        versions: vec![
            version(DocKind::Brainstorm, 1),
            version(DocKind::Spec, 2),
            // Ruling T5-1: a review draft.
            DocVersion {
                n: 0,
                draft_review: Some(2),
                ..version(DocKind::Spec, 0)
            },
        ],
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
        // Task M9.6.8 fix round 1 (ruling T8-1): a held draft is never written either.
        held: Vec::new(),
        // Ruling T9-1a: in memory only.
        unread: Vec::new(),
        read_back_owed: false,
        // Ruling T8-5.
        drafts_settled: true,
        // Ruling T8-2: the pack's frozen inputs.
        pack: Some(crate::run::design::pack::FrozenPack {
            reports: vec!["s1".into()],
            earlier: Some(crate::run::design::pack::EarlierSpec {
                run: "prev-run-0001".into(),
                path: "/tmp/data/runs/prev-run-0001/design/spec-v1.md".into(),
                version: version(DocKind::Spec, 1),
            }),
            // Ruling T8-6: the round and its written pack.
            round: 2,
            file: Some(crate::run::design::pack::PackFile {
                bytes: 120,
                sha256: "c".repeat(64),
            }),
            // Task M9.6.9: a rethink's note and the report it replaces.
            rethink: Some(crate::run::design::pack::FrozenRethink {
                note: "think about SSO".into(),
                path: "/tmp/data/runs/r/design/brainstorm-v1.md".into(),
                version: version(DocKind::Brainstorm, 1),
            }),
        }),
        // Task M9.6.10: the approved spec, whose requirements are stored.
        approved_spec: Some(2),
        // Task M9.6.11: its Interfaces section, a failed read-back (ruling T10-3) and
        // a plan revision's note kept for its next version.
        interfaces_section: "`reset(token)`".into(),
        spec_unread: true,
        plan_revision: Some("Add mail.".into()),
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
    assert_eq!(json["orch"]["design"]["reviewer"]["unsubmitted"], true);
    assert_eq!(json["orch"]["design"]["versions"][2]["draft_review"], 2);
    assert_eq!(json["orch"]["design"]["reviews"][0]["after"], 1);
    assert_eq!(json["orch"]["design"]["approved_spec"], 2);
    assert!(json["orch"]["design"].get("texts").is_none());

    // Without it, `orch` has no `design` key.
    let mut plain = old_run();
    let (loaded, text) = save_and_load(&mut plain, dir.path());
    assert_eq!(loaded.orch.design, None);
    let json: serde_json::Value = serde_json::from_str(&text).expect("run.json is JSON");
    assert!(json["orch"].get("design").is_none(), "{}", json["orch"]);
}
