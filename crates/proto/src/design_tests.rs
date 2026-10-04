//! Milestone 9.6 task 2 (protocol 17): the design flow's wire types and its new requests
//! and reply round-trip in MessagePack and JSON. `design_tests_wire.rs` checks that what
//! protocol 16 sent and wrote still decodes, and that every new variant is appended.

use std::collections::BTreeMap;
use std::path::PathBuf;

use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::json;

use crate::design::*;
use crate::run_wire::request;
use crate::*;

/// M8a's round-trip fixtures, loaded as this module's own private copy.
#[allow(clippy::duplicate_mod, dead_code)]
#[path = "run_tests_fixtures.rs"]
mod fixtures;
use fixtures::{a_route, a_run_info, a_task_info};

fn both_ways<T>(value: &T)
where
    T: Serialize + DeserializeOwned + PartialEq + std::fmt::Debug,
{
    let packed = rmp_serde::to_vec_named(value).unwrap();
    let back: T = rmp_serde::from_slice(&packed).unwrap();
    assert_eq!(&back, value, "MessagePack");
    let json = serde_json::to_string(value).unwrap();
    let back: T = serde_json::from_str(&json).unwrap();
    assert_eq!(&back, value, "JSON: {json}");
}

/// Untagged (`ClientMsg::Run`) and tagged (`ClientMsg::RunTagged`).
fn request_both_ways(request: RunRequest) {
    both_ways(&ClientMsg::Run(request.clone()));
    both_ways(&ClientMsg::RunTagged { id: 17, request });
}

/// Untagged and tagged (`request_id: Some`).
fn reply_both_ways(reply: RunReply) {
    both_ways(&DaemonMsg::Run(reply.clone()));
    let snapshot = matches!(reply, RunReply::Snapshot(_));
    let tagged = reply.tagged(Some(17));
    assert_eq!(tagged.request_id(), (!snapshot).then_some(17));
    both_ways(&DaemonMsg::Run(tagged));
}

/// Protocol-16 MessagePack bytes of `value`: a named map, as `codec::encode` writes a
/// struct, with only the keys protocol 16 had.
fn p16_bytes(value: &serde_json::Value) -> Vec<u8> {
    rmp_serde::to_vec_named(value).unwrap()
}

fn a_finding(id: &str, severity: DocSeverity) -> DocFinding {
    DocFinding {
        id: id.into(),
        severity,
        place: "## Requirements, R3".into(),
        text: "R3 has no acceptance check".into(),
    }
}

fn a_doc_view() -> DocView {
    DocView {
        run: "run-a1b2".into(),
        kind: DocKind::Spec,
        version: 2,
        text: "# Reset\n\n## Requirements\nR1 a token expires after one hour\n".into(),
        diff: Some("- R1 old\n+ R1 a token expires after one hour\n".into()),
        findings: vec![
            (a_finding("f1", DocSeverity::Blocking), Some("fixed".into())),
            (
                a_finding("f2", DocSeverity::Minor),
                Some("kept: out of scope".into()),
            ),
            (a_finding("f3", DocSeverity::Minor), None),
        ],
    }
}

fn a_gate_info() -> DocGateInfo {
    DocGateInfo {
        kind: DocGateKind::Spec,
        version: 2,
        revising: Some("tighten R3".into()),
        disputed: vec![a_finding("f2", DocSeverity::Minor)],
        not_reviewed: Some("the reviewer's session failed".into()),
        changes_summary: vec!["+ R4a, R4b".into(), "~ Testing: 2 lines".into()],
        same_runtime: true,
        report: None,
    }
}

fn a_doc_info(kind: DocKind, author: DocAuthor) -> DocInfo {
    DocInfo {
        kind,
        version: 1,
        author,
        reason: "first draft".into(),
        bytes: 4_096,
        requirements: vec!["R1".into(), "R2".into()],
    }
}

fn a_phase_record() -> PhaseRecord {
    PhaseRecord {
        v: HISTORY_VERSION,
        record_id: "run-a1b2/phase/1/brainstorming".into(),
        at: 1_700_000_900,
        run_id: "run-a1b2".into(),
        round: 1,
        phase: "brainstorming".into(),
        secs: 420,
        agents: vec![
            PhaseAgent {
                role: AgentRole::Brainstormer,
                route: a_route(Runtime::Claude, Strength::Frontier, Effort::High, "opus"),
                calls: 31,
                tokens: 812_000,
                outcome: "ok".into(),
            },
            PhaseAgent {
                role: AgentRole::DocReviewer,
                route: a_route(Runtime::Codex, Strength::Frontier, Effort::Medium, ""),
                calls: 9,
                tokens: 120_000,
                outcome: "failed: the session exited".into(),
            },
        ],
        gate_versions: 2,
        disputed: 1,
    }
}

fn actions() -> [DocGateAction; 6] {
    [
        DocGateAction::Approve,
        DocGateAction::Changes {
            note: "split R2".into(),
            review: true,
        },
        DocGateAction::Edit {
            text: "# Reset\n".into(),
        },
        DocGateAction::Rethink {
            note: "consider a queue".into(),
        },
        DocGateAction::Back {
            note: "the approach is wrong".into(),
        },
        DocGateAction::Reject,
    ]
}

#[test]
fn design_types_round_trip() {
    assert_eq!(DesignMode::default(), DesignMode::Off);
    for (mode, word) in [(DesignMode::Full, "full"), (DesignMode::Off, "off")] {
        both_ways(&mode);
        assert_eq!(serde_json::to_value(mode).unwrap(), word);
    }
    for (design, word) in [
        (RoundDesign::Amend, "amend"),
        (RoundDesign::Full, "full"),
        (RoundDesign::Off, "off"),
    ] {
        both_ways(&design);
        assert_eq!(serde_json::to_value(design).unwrap(), word);
    }
    for (kind, word) in [
        (DocKind::BrainstormDraft, "brainstorm_draft"),
        (DocKind::Brainstorm, "brainstorm"),
        (DocKind::Spec, "spec"),
        (DocKind::Plan, "plan"),
    ] {
        both_ways(&kind);
        assert_eq!(serde_json::to_value(kind).unwrap(), word);
        assert_eq!(kind.label(), word);
    }
    for (kind, word) in [
        (DocGateKind::Brainstorm, "brainstorm"),
        (DocGateKind::Spec, "spec"),
        (DocGateKind::Plan, "plan"),
    ] {
        both_ways(&kind);
        assert_eq!(serde_json::to_value(kind).unwrap(), word);
        assert_eq!(kind.label(), word);
    }
    for (severity, word) in [
        (DocSeverity::Blocking, "blocking"),
        (DocSeverity::Minor, "minor"),
    ] {
        both_ways(&severity);
        assert_eq!(serde_json::to_value(severity).unwrap(), word);
    }
    let authors = [
        DocAuthor::Orchestrator,
        DocAuthor::User,
        DocAuthor::Brainstormer {
            label: "codex".into(),
        },
        DocAuthor::Engine,
    ];
    for author in &authors {
        both_ways(author);
        both_ways(&a_doc_info(DocKind::Brainstorm, author.clone()));
    }
    assert_eq!(
        serde_json::to_value(&authors[2]).unwrap(),
        json!({"brainstormer": {"label": "codex"}})
    );
    both_ways(&a_finding("f1", DocSeverity::Blocking));
    assert_eq!(
        serde_json::to_value(a_finding("f1", DocSeverity::Blocking)).unwrap(),
        json!({"id": "f1", "severity": "blocking", "place": "## Requirements, R3",
               "text": "R3 has no acceptance check"})
    );
    both_ways(&FindingAnswer {
        id: "f2".into(),
        answer: "kept: out of scope".into(),
    });
    for action in actions() {
        both_ways(&action);
    }
    assert_eq!(
        serde_json::to_value(&actions()[1]).unwrap(),
        json!({"changes": {"note": "split R2", "review": true}})
    );
    assert_eq!(serde_json::to_value(&actions()[0]).unwrap(), "approve");
    both_ways(&a_gate_info());
    both_ways(&DocGateInfo {
        revising: None,
        disputed: Vec::new(),
        not_reviewed: None,
        changes_summary: Vec::new(),
        same_runtime: false,
        ..a_gate_info()
    });
    both_ways(&a_doc_view());
    both_ways(&DocView {
        diff: None,
        findings: Vec::new(),
        ..a_doc_view()
    });
    both_ways(&a_phase_record());
    both_ways(&a_phase_record().agents[0]);

    // The action menu's entry opens the gate screen; the daemon lists it.
    let kind = ActionKind::ReviewDoc;
    assert_eq!(kind.needs(), ActionNeeds::Open);
    assert!(!kind.destructive() && !kind.is_local());
    assert_eq!(
        serde_json::to_value(&kind).unwrap(),
        json!({"kind": "review_doc"})
    );
    both_ways(&kind);

    // The two new states and roles.
    for (state, word) in [
        (RunState::Brainstorming, "brainstorming"),
        (RunState::Specifying, "specifying"),
    ] {
        assert_eq!(state.label(), word);
        assert_eq!(serde_json::to_value(state).unwrap(), word);
        assert!(!state.is_terminal());
        both_ways(&state);
    }
    for (role, word) in [
        (AgentRole::Brainstormer, "brainstormer"),
        (AgentRole::DocReviewer, "doc_reviewer"),
    ] {
        assert_eq!(serde_json::to_value(role).unwrap(), word);
        both_ways(&role);
    }

    // A snapshot with every new field set.
    let mut run = a_run_info();
    run.state = RunState::AwaitingApproval;
    run.design = DesignMode::Full;
    run.doc_gate = Some(a_gate_info());
    run.docs = vec![
        a_doc_info(DocKind::Brainstorm, DocAuthor::Orchestrator),
        a_doc_info(DocKind::Spec, DocAuthor::User),
    ];
    run.tasks[0].covers = vec!["R1".into(), "R2".into()];
    let json = serde_json::to_value(&run).unwrap();
    assert_eq!(json["design"], "full");
    assert_eq!(json["tasks"][0]["covers"], json!(["R1", "R2"]));
    reply_both_ways(RunReply::Snapshot(RunsSnapshot {
        revision: 3,
        runs: vec![run],
        now: 1_700_000_800,
        proposals: Vec::new(),
        idle_orchestrators: Vec::new(),
    }));
}

#[test]
fn doc_gate_and_show_doc_round_trip() {
    for kind in [
        DocGateKind::Brainstorm,
        DocGateKind::Spec,
        DocGateKind::Plan,
    ] {
        for action in actions() {
            request_both_ways(RunRequest::DocGate {
                run: "run-a1b2".into(),
                kind,
                action,
            });
        }
    }
    let gate: RunRequest = serde_json::from_value(json!({"DocGate": {
        "run": "run-a1b2", "kind": "spec",
        "action": {"changes": {"note": "split R2", "review": false}}
    }}))
    .unwrap();
    assert_eq!(
        gate,
        RunRequest::DocGate {
            run: "run-a1b2".into(),
            kind: DocGateKind::Spec,
            action: DocGateAction::Changes {
                note: "split R2".into(),
                review: false,
            },
        }
    );
    for kind in [
        DocKind::BrainstormDraft,
        DocKind::Brainstorm,
        DocKind::Spec,
        DocKind::Plan,
    ] {
        request_both_ways(RunRequest::ShowDoc {
            run: "run-a1b2".into(),
            kind,
            version: Some(2),
            diff: true,
            findings: true,
        });
    }
    // `version`, `diff` and `findings` may be left out: the latest version, plain.
    let show: RunRequest = rmp_serde::from_slice(&p16_bytes(
        &json!({"ShowDoc": {"run": "r1", "kind": "plan"}}),
    ))
    .expect("a bare ShowDoc");
    assert_eq!(
        show,
        RunRequest::ShowDoc {
            run: "r1".into(),
            kind: DocKind::Plan,
            version: None,
            diff: false,
            findings: false,
        }
    );
    reply_both_ways(RunReply::Doc {
        doc: Box::new(a_doc_view()),
        request_id: None,
    });
    let reply: RunReply = serde_json::from_value(json!({"Doc": {
        "doc": serde_json::to_value(a_doc_view()).unwrap()
    }}))
    .expect("a Doc reply without its request_id");
    assert_eq!(reply.request_id(), None);
    assert_eq!(request::DOC_GATE, "run doc gate");
    assert_eq!(request::SHOW_DOC, "run show");
}

#[path = "design_tests_wire.rs"]
mod wire;
