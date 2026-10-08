//! Task M9.6.3: the design flow's per-run fields persist through `run.json`
//! (`journal::save_run`, the driver's serializer, and `journal::load_all`): the frozen
//! mode (`Run.design_mode`) and the frozen design limits (`OrchLimits.design`); the
//! frozen `brainstorm` model list (carry M-4) went with the lists in milestone 9.8 (task
//! M9.8.13). A protocol-16 (9.5) `run.json` loads with the mode `Off` and writes none of
//! them back.

use proto::{DesignMode, Effort, Runtime, Strength};

use super::Run;
use super::tuning::tests::{count, keys, save_and_load, tmp};
use crate::run::orch::DesignLimits;

/// A planned goal's `run.json` as milestone 9.5's code wrote it at the start (state
/// `planning`, an orchestrator record, a frozen `review` list), captured in task
/// M9.6.3 (commit 00ec0f18) before any of its changes.
const M95_RUN: &str = include_str!("../../tests/fixtures/run/m95-run.json");

/// Every key task M9.6.3 adds to the persisted run (less the `brainstorm` list's,
/// removed in M9.8.13).
const NEW_KEYS: [&str; 2] = ["design_mode", "design"];

fn old_run() -> Run {
    serde_json::from_str(M95_RUN).expect("m95-run.json parses")
}

#[test]
fn an_old_run_json_loads_with_design_off() {
    let mut run = old_run();
    assert_eq!(run.state, proto::RunState::Planning);
    assert!(run.orch.orchestrator.is_some(), "a planned goal's run");
    assert_eq!(run.design_mode, DesignMode::Off);
    assert_eq!(run.limits.orch.design, DesignLimits::default());

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
    // Milestone 9.8 (task M9.8.13): the roster, the scouts' routing keys and the model
    // lists are no longer part of the run; the rest is written back as 9.5 wrote it.
    let mut captured = captured;
    crate::run::test_support::without_pre_9_8_keys(&mut captured);
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

    let dir = tmp();
    let (loaded, text) = save_and_load(&mut run, dir.path());
    assert_eq!(loaded, run);
    let written = keys(&serde_json::from_str(&text).expect("run.json is JSON"));
    for key in NEW_KEYS {
        assert!(count(&written, key) > 0, "run.json lacks {key}");
    }
    assert!(text.contains("\"design_mode\":\"full\""), "{text}");
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
        uncapped: false,
    };
    let route = Route {
        runtime: Runtime::Codex,
        model: "gpt-6".into(),
        strength: Strength::Frontier,
        effort: Effort::HIGH,
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
            round: 1,
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
            round: 1,
        }),
        reviews: vec![DocReviewRecord {
            doc: DocKind::Spec,
            n: 1,
            findings: vec![finding.clone()],
            failed: None,
            // Task M9.6.10: the cycle the review belongs to, and its runtime.
            after: 1,
            same_runtime: true,
            // Ruling T15-9: a dropped round's review is kept, marked.
            dropped: true,
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
        spec_path: Some("docs/anthrex/specs/2026-10-05-reset.md".into()),
        // Task M9.6.7: a budget halt's state and the rethinks; the texts' cache is
        // never written.
        halted_from: Some(proto::RunState::Specifying),
        rethinks: 2,
        texts: Vec::new(),
        // Task M9.6.8 fix round 1 (ruling T8-1): a held draft is never written either.
        held: Vec::new(),
        held_findings: None,
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
                // Ruling T15-1: each later round's approved amendment.
                amendments: vec![crate::run::design::pack::EarlierAmendment {
                    round: 2,
                    path: "/tmp/data/runs/prev-run-0001/design/spec-v2.md".into(),
                    version: version(DocKind::Spec, 2),
                }],
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
        plan_fingerprint: None,
        // Task M9.6.13 (ruling T13-1): each phase's spend, and a rethink's pending
        // starts.
        spend: vec![crate::run::design::state::PhaseSpend {
            round: 1,
            phase: "brainstorming".into(),
            secs: 420,
            agents: vec![crate::run::design::state::AgentSpend {
                label: "codex".into(),
                role: AgentRole::Brainstormer,
                route: Route {
                    runtime: Runtime::Codex,
                    model: "gpt-6".into(),
                    strength: Strength::Frontier,
                    effort: Effort::HIGH,
                },
                sessions: 2,
                calls: 11,
                tokens: 1_200,
                secs: 300,
                outcome: "ok".into(),
            }],
        }],
        rethink_starts: vec!["claude".into()],
        // Task M9.6.15: round 2's design and the round whose documents were committed.
        round: Some(crate::run::design::round::DesignRound {
            n: 2,
            mode: proto::RoundDesign::Amend,
            base: vec![Requirement {
                id: "R1".into(),
                text: "one".into(),
            }],
            spec_before: Some(1),
            goal_before: "Reset.".into(),
            interfaces_before: String::new(),
            versions_before: [1, 1, 1],
            rethinks_before: 2,
            packs_before: 0,
            amended: vec!["R1".into()],
            reviews_before: 1,
            spec_text_before: None,
        }),
        committed_round: 1,
        // Ruling T15-1: the specs approved before round 2.
        approved_before: vec![crate::run::design::round::ApprovedSpec {
            round: 1,
            version: 1,
        }],
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
    assert_eq!(json["orch"]["design"]["round"]["mode"], "amend");
    assert!(json["orch"]["design"].get("texts").is_none());

    // Without it, `orch` has no `design` key.
    let mut plain = old_run();
    let (loaded, text) = save_and_load(&mut plain, dir.path());
    assert_eq!(loaded.orch.design, None);
    let json: serde_json::Value = serde_json::from_str(&text).expect("run.json is JSON");
    assert!(json["orch"].get("design").is_none(), "{}", json["orch"]);
}

/// The final fix wave's FW-15 (ruling T15-15, re-review m1): a later round's landed
/// documents commit (`Round.committed_stage`) survives a save and load.
#[test]
fn a_rounds_committed_stage_survives_save_and_load() {
    let mut run = old_run();
    let mut round = super::Round::first(&run);
    round.n = 2;
    round.committed_stage = Some(2);
    run.rounds = vec![super::Round::first(&run), round];
    let dir = tmp();
    let (loaded, text) = save_and_load(&mut run, dir.path());
    let stages: Vec<Option<u16>> = loaded.rounds.iter().map(|r| r.committed_stage).collect();
    assert_eq!(stages, [None, Some(2)]);
    assert!(text.contains("\"committed_stage\":2"), "{text}");
}
