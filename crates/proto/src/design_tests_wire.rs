//! Milestone 9.6 task 2, continued from `design_tests.rs`: what protocol 16 sent and
//! wrote still decodes (requests, plans, snapshots), the `phase` history line, and the
//! appended variants keep their indices.

use super::*;
use crate::delivery::tests::{tagged_names, variant_at, variant_names};

#[test]
fn start_and_iterate_design_default_to_none() {
    // As a protocol-16 client sends them: no `design` key.
    let start = json!({"Start": {
        "plan_toml": "goal = \"g\"", "dir": "/tmp/p", "yes": false,
        "trust_project": false, "unconfined_checks": false, "delivery": null
    }});
    let Ok(RunRequest::Start { design, .. }) = rmp_serde::from_slice(&p16_bytes(&start)) else {
        panic!("a protocol-16 Start");
    };
    assert_eq!(design, None);
    let goal = json!({"StartGoal": {
        "goal": "add reset", "dir": "/tmp/p", "yes": true, "trust_project": false,
        "unconfined_checks": false, "orchestrator": null, "delivery": null,
        "continue_from": null
    }});
    let Ok(RunRequest::StartGoal { design, .. }) = rmp_serde::from_slice(&p16_bytes(&goal)) else {
        panic!("a protocol-16 StartGoal");
    };
    assert_eq!(design, None);
    let iterate = json!({"Iterate": {"run": "run-a1b2", "goal": "more"}});
    let Ok(RunRequest::Iterate { design, .. }) = rmp_serde::from_slice(&p16_bytes(&iterate)) else {
        panic!("a protocol-16 Iterate");
    };
    assert_eq!(design, None);
    let Ok(RunRequest::Iterate { design, .. }) = serde_json::from_value(iterate) else {
        panic!("a JSON Iterate");
    };
    assert_eq!(design, None);

    // And each carries a choice.
    request_both_ways(RunRequest::Start {
        plan_toml: "goal = \"g\"".into(),
        dir: PathBuf::from("/tmp/p"),
        yes: false,
        trust_project: false,
        unconfined_checks: false,
        delivery: None,
        design: Some(DesignMode::Off),
    });
    request_both_ways(RunRequest::StartGoal {
        goal: "add reset".into(),
        dir: PathBuf::from("/tmp/p"),
        yes: true,
        trust_project: false,
        unconfined_checks: false,
        orchestrator: None,
        delivery: None,
        continue_from: None,
        design: Some(DesignMode::Full),
    });
    for design in [RoundDesign::Amend, RoundDesign::Full, RoundDesign::Off] {
        request_both_ways(RunRequest::Iterate {
            run: "run-a1b2".into(),
            goal: "more".into(),
            design: Some(design),
        });
    }
}

const TASK: &str = r#"
id = "t1"
title = "Reset token"
size = "S"
owns = ["crates/auth/**"]
brief = "Files:\nTests first:\nSteps:\nAcceptance:\nVerify:"
acceptance = ["cargo test"]
"#;

#[test]
fn plan_task_covers_defaults_and_skips() {
    let task: PlanTask = toml::from_str(TASK).expect("a task without covers");
    assert!(task.covers.is_empty());
    let json = serde_json::to_value(&task).unwrap();
    assert!(json.get("covers").is_none(), "{json}");
    assert!(!toml::to_string(&task).unwrap().contains("covers"));
    both_ways(&task);

    let covered: PlanTask =
        toml::from_str(&format!("{TASK}covers = [\"R1\", \"R4\"]\n")).expect("covers");
    assert_eq!(covered.covers, ["R1", "R4"]);
    assert_eq!(
        serde_json::to_value(&covered).unwrap()["covers"],
        json!(["R1", "R4"])
    );
    both_ways(&covered);
    let text = toml::to_string(&covered).unwrap();
    assert_eq!(
        toml::from_str::<PlanTask>(&text).unwrap(),
        covered,
        "{text}"
    );

    // `deny_unknown_fields` still holds.
    let error = toml::from_str::<PlanTask>(&format!("{TASK}cover = [\"R1\"]\n")).unwrap_err();
    assert!(error.to_string().contains("unknown field"), "{error}");

    // `TaskInfo.covers` is left out while empty.
    let json = serde_json::to_value(a_task_info()).unwrap();
    assert!(json.get("covers").is_none(), "{json}");
}

#[test]
fn run_info_without_design_decodes() {
    for (name, text) in [
        ("m95_run_info.json", include_str!("m95_run_info.json")),
        ("m93_run_info.json", include_str!("m93_run_info.json")),
    ] {
        let value: serde_json::Value = serde_json::from_str(text).unwrap();
        for key in ["design", "doc_gate", "docs"] {
            assert!(value.get(key).is_none(), "{name} predates {key}");
        }
        let json: RunInfo =
            serde_json::from_str(text).unwrap_or_else(|e| panic!("{name} decodes: {e}"));
        let packed: RunInfo = rmp_serde::from_slice(&p16_bytes(&value))
            .unwrap_or_else(|e| panic!("{name} decodes from MessagePack: {e}"));
        assert_eq!(json, packed, "{name}");
        for run in [&json, &packed] {
            assert_eq!(run.design, DesignMode::Off, "{name}");
            assert_eq!(run.doc_gate, None, "{name}");
            assert!(run.docs.is_empty(), "{name}");
            assert!(!run.tasks.is_empty(), "{name}");
            assert!(run.tasks.iter().all(|t| t.covers.is_empty()), "{name}");
        }
        // Written back, it is the same shape: none of the new keys appear.
        let again = serde_json::to_value(&json).unwrap();
        for key in ["design", "doc_gate", "docs"] {
            assert!(again.get(key).is_none(), "{name} writes no {key}");
        }
        assert!(
            again["tasks"]
                .as_array()
                .unwrap()
                .iter()
                .all(|t| t.get("covers").is_none()),
            "{name}"
        );
    }
    let captured: RunInfo = serde_json::from_str(include_str!("m95_run_info.json")).unwrap();
    assert!(
        captured.tasks.iter().any(|t| t.race.is_some()),
        "the 9.5 fixture has a race"
    );
}

#[test]
fn phase_history_line_round_trips() {
    assert_eq!(HISTORY_VERSION, 5);
    let line = HistoryLine::Phase(a_phase_record());
    both_ways(&line);
    let json = serde_json::to_value(&line).unwrap();
    assert_eq!(json["type"], "phase");
    assert_eq!(json["record_id"], "run-a1b2/phase/1/brainstorming");
    assert_eq!(json["agents"][1]["role"], "doc_reviewer");
    let text = serde_json::to_string(&line).unwrap();
    assert!(!text.contains('\n'), "one line");
    assert_eq!(serde_json::from_str::<HistoryLine>(&text).unwrap(), line);
    assert_eq!(json["agents"][0]["secs"], 600);
}

/// Task M9.6.13 (ruling T13-1): a phase agent's active seconds, defaulted on a line
/// written before them.
#[test]
fn a_phase_agent_without_secs_decodes() {
    let mut json = serde_json::to_value(HistoryLine::Phase(a_phase_record())).unwrap();
    json["agents"][0].as_object_mut().unwrap().remove("secs");
    let line: HistoryLine = serde_json::from_value(json).unwrap();
    let HistoryLine::Phase(record) = line else {
        panic!("{line:?}");
    };
    assert_eq!((record.agents[0].secs, record.agents[1].secs), (0, 240));
}

/// Task M9.6.13 fix round 1 (ruling T13-3): a phase agent's sessions, read as one on a
/// line written before them; the current shape round-trips.
#[test]
fn a_phase_agent_without_sessions_reads_as_one() {
    let line = HistoryLine::Phase(a_phase_record());
    both_ways(&line);
    let mut json = serde_json::to_value(&line).unwrap();
    assert_eq!(json["agents"][0]["sessions"], 2);
    for agent in json["agents"].as_array_mut().unwrap() {
        let agent = agent.as_object_mut().unwrap();
        agent.remove("sessions");
        agent.remove("secs");
    }
    let older: HistoryLine = serde_json::from_value(json).unwrap();
    let HistoryLine::Phase(record) = older else {
        panic!("{older:?}");
    };
    let read: Vec<(u32, u64)> = (record.agents.iter())
        .map(|a| (a.sessions, a.secs))
        .collect();
    assert_eq!(read, [(1, 0), (1, 0)]);
}

#[test]
fn appended_variants_keep_their_indices() {
    // A unit variant also decodes from its index in MessagePack.
    for (index, state) in [
        (8u8, RunState::Planning),
        (9, RunState::Brainstorming),
        (10, RunState::Specifying),
    ] {
        assert_eq!(
            rmp_serde::from_slice::<RunState>(&[index]).ok(),
            Some(state)
        );
    }
    assert!(
        rmp_serde::from_slice::<RunState>(&[11]).is_err(),
        "no state after"
    );
    let states = variant_names::<RunState>();
    assert_eq!(
        states[states.len() - 3..],
        ["planning", "brainstorming", "specifying"],
        "{states:?}"
    );
    for (index, role) in [
        (7u8, AgentRole::TestWriter),
        (8, AgentRole::Brainstormer),
        (9, AgentRole::DocReviewer),
    ] {
        assert_eq!(
            rmp_serde::from_slice::<AgentRole>(&[index]).ok(),
            Some(role)
        );
    }
    assert!(
        rmp_serde::from_slice::<AgentRole>(&[10]).is_err(),
        "no role after"
    );
    let roles = variant_names::<AgentRole>();
    assert_eq!(
        roles[roles.len() - 3..],
        ["test_writer", "brainstormer", "doc_reviewer"],
        "{roles:?}"
    );

    let names = variant_names::<RunRequest>();
    assert_eq!(
        names[names.len() - 3..],
        ["McpReady", "DocGate", "ShowDoc"],
        "{names:?}"
    );
    let n = names.len() as u8;
    assert_eq!(
        variant_at::<RunRequest>(
            n - 2,
            &json!({"run": "r1", "kind": "plan", "action": "reject"})
        ),
        Some(RunRequest::DocGate {
            run: "r1".into(),
            kind: DocGateKind::Plan,
            action: DocGateAction::Reject,
        })
    );
    assert_eq!(
        variant_at::<RunRequest>(n - 1, &json!({"run": "r1", "kind": "spec", "version": 2})),
        Some(RunRequest::ShowDoc {
            run: "r1".into(),
            kind: DocKind::Spec,
            version: Some(2),
            diff: false,
            findings: false,
        })
    );
    assert_eq!(
        variant_at::<RunRequest>(n - 3, &json!({"run_id": "r1", "window_id": 3})),
        Some(RunRequest::McpReady {
            run_id: "r1".into(),
            window_id: 3,
        })
    );

    let names = variant_names::<RunReply>();
    assert_eq!(names[names.len() - 2..], ["Settings", "Doc"], "{names:?}");
    let n = names.len() as u8;
    let doc = serde_json::to_value(a_doc_view()).unwrap();
    assert_eq!(
        variant_at::<RunReply>(n - 1, &json!({"doc": doc, "request_id": 4})),
        Some(RunReply::Doc {
            doc: Box::new(a_doc_view()),
            request_id: Some(4),
        })
    );

    let names = tagged_names::<ActionKind>("kind");
    assert_eq!(
        names[names.len() - 2..],
        ["iterate", "review_doc"],
        "{names:?}"
    );
    let names = tagged_names::<HistoryLine>("type");
    assert_eq!(names[names.len() - 2..], ["round", "phase"], "{names:?}");
}

/// Task M9.6.9 (task 5's concern 2): the brainstorm gate's report summary, the Review
/// panel's agree and disagree counts and each approach's tag, rides on `DocGateInfo`,
/// appended; a gate info without it (an earlier 17 build, or any other gate) reads
/// `None` and writes no key.
#[test]
fn a_gate_info_carries_the_reports_summary() {
    let summary = ReportSummary {
        agree: 2,
        disagree: 1,
        approaches: vec![
            ApproachTag {
                name: "Signed tokens".into(),
                tag: "claude".into(),
            },
            ApproachTag {
                name: "Stored tokens".into(),
                tag: "both".into(),
            },
        ],
    };
    let info = DocGateInfo {
        kind: DocGateKind::Brainstorm,
        report: Some(summary.clone()),
        ..a_gate_info()
    };
    both_ways(&info);
    both_ways(&summary);
    let plain = serde_json::to_value(a_gate_info()).unwrap();
    assert!(plain.get("report").is_none(), "{plain}");
    let mut old = plain.clone();
    old.as_object_mut().unwrap().remove("report");
    let back: DocGateInfo = rmp_serde::from_slice(&p16_bytes(&old)).unwrap();
    assert_eq!(back, a_gate_info());
}

/// Task M9.6.11 fix round 1 (ruling T11-1): a sub-planner's `covers`, the requirement
/// ids its epic owns, appended: left out while empty, so a protocol-16 snapshot's
/// planner decodes and a run without the design flow writes 9.5's bytes.
#[test]
fn planner_info_covers_defaults_and_skips() {
    let old = json!({
        "epic": "mail", "title": "Mail", "area": ["crates/mail/**"],
        "route": a_route(Runtime::Claude, Strength::Frontier, Effort::High, "m"),
        "window_id": null, "state": "planning",
        "started_at": 1, "ended_at": null, "edits_accepted": 0, "edits_rejected": 0,
        "last_rejection": null, "replans": [],
    });
    let planner: PlannerInfo = serde_json::from_value(old).expect("a 9.5 planner");
    assert!(planner.covers.is_empty());
    let json = serde_json::to_value(&planner).unwrap();
    assert!(json.get("covers").is_none(), "{json}");
    both_ways(&planner);
    let owned = PlannerInfo {
        covers: vec!["R2".into(), "R5".into()],
        ..planner
    };
    assert_eq!(
        serde_json::to_value(&owned).unwrap()["covers"],
        json!(["R2", "R5"])
    );
    both_ways(&owned);
}
