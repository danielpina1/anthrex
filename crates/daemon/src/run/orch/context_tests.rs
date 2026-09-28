//! Task M9.6: `get_context` (decision 17, Interfaces "The context"). Pure.

use std::collections::BTreeMap;

use proto::{
    DeciderSource, RepoProfile, RunPath, RunState, Scale, ScoutFile, ScoutKind, ScoutReport,
    TaskKind, TokenUsage, TriageInfo,
};
use serde_json::{Value, json};

use super::*;
use crate::run::orch::json::size;
use crate::run::orch::test_support::*;
use crate::run::orch::{EpicRecord, PlannerPhase, RunScoutState};

fn report(id: &str, kind: ScoutKind, summary: &str, files: usize) -> ScoutReport {
    ScoutReport {
        id: id.into(),
        kind,
        run_id: None,
        question: format!("What does {id} hold?"),
        summary: summary.into(),
        files: (0..files)
            .map(|i| ScoutFile {
                path: format!("crates/x/src/f{i}.rs"),
                why: "it matters".into(),
            })
            .collect(),
        modules: vec!["crates/x".into()],
        interfaces: vec!["fn x()".into()],
        risks: vec!["none".into()],
        profile: None,
        route: route(),
        window_id: None,
        started_at: 1,
        finished_at: 2,
        tool_calls: 3,
        usage: TokenUsage::default(),
    }
}

fn planning_run() -> Run {
    let mut run = run_with(&[
        task_toml("t0", "M", "[\"crates/proto/**\"]", ""),
        task_toml("a1", "S", "[\"crates/a/**\"]", "epic = \"a\""),
        task_toml("b1", "S", "[\"crates/b/**\"]", "epic = \"b\""),
    ]);
    run.state = RunState::Planning;
    run.path = Some(RunPath::Large);
    run.triage = Some(TriageInfo {
        kinds: vec![TaskKind::Code],
        scale: Scale::Large,
        path: RunPath::Large,
        reason: "three crates".into(),
        source: DeciderSource::Decider,
        fallback_reason: None,
        at: 1,
    });
    run.orch.orchestrator = Some(orchestrator());
    let mut a = EpicRecord::new("a", PlannerPhase::Planning);
    a.brief = "Plan the a crate.".into();
    a.scout_refs = vec!["3f9a-ref".into()];
    run.orch.epics = vec![a, EpicRecord::new("b", PlannerPhase::Finished)];
    run.orch.run_scouts = vec![
        scout("3f9a-ref", RunScoutState::Reported, &["docs/**"]),
        scout("3f9a-a", RunScoutState::Reported, &["crates/a/src/**"]),
        scout("3f9a-b", RunScoutState::Reported, &["crates/b/**"]),
        scout("3f9a-q", RunScoutState::Queued, &["crates/a/**"]),
    ];
    run.orch.installed = BTreeMap::from([("claude".into(), true), ("codex".into(), false)]);
    run
}

fn reports() -> Vec<ScoutReport> {
    vec![
        report("onboarding-1", ScoutKind::Onboarding, "The layout.", 1),
        report("3f9a-ref", ScoutKind::Area, "Docs.", 1),
        report("3f9a-a", ScoutKind::Area, "The a crate.", 2),
        report("3f9a-b", ScoutKind::Area, "The b crate.", 1),
    ]
}

fn profile() -> RepoProfile {
    RepoProfile {
        languages: vec!["rust".into()],
        check: Some("cargo test".into()),
        ..RepoProfile::default()
    }
}

fn ask(run: &Run, asker: Asker, only: Option<Vec<String>>) -> Value {
    let profile = profile();
    context(&ContextInputs {
        run,
        asker,
        profile: Some(&profile),
        reports: reports(),
        only,
    })
}

fn ids(value: &Value, key: &str, field: &str) -> Vec<String> {
    value[key]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v[field].as_str().unwrap().to_string())
        .collect()
}

#[test]
fn context_for_the_orchestrator() {
    let run = planning_run();
    let c = ask(&run, Asker::Orchestrator, None);
    assert_eq!(
        c["run"],
        json!({
            "id": run.id, "goal": "Test goal", "path": "large", "state": "planning",
            "root": "/tmp/x", "base_branch": "main", "base_sha": "b".repeat(40),
            "triage": {"kinds": ["code"], "scale": "large", "reason": "three crates"},
        })
    );
    assert_eq!(c["you"], json!({"role": "orchestrator", "epic": null}));
    assert_eq!(c["profile"]["summary"], crate::profile::summary(&profile()));
    assert_eq!(c["profile"]["modules"], json!(["crates/*"]));
    assert_eq!(c["profile"]["hub"], json!(["crates/proto/**"]));
    assert_eq!(c["profile"]["check"], "cargo test");
    assert_eq!(c["profile"]["single_test"], "cargo test -- --exact {test}");
    let limits = &c["limits"];
    assert_eq!(limits["planner_task_cap"], 12);
    assert_eq!(limits["max_writers"], 3);
    assert_eq!(limits["max_scouts"], 12);
    assert_eq!(limits["sizes"]["L"], "never executed: split it");
    assert_eq!(
        limits["sizes"]["S"],
        "one file, no interface change, a mechanical check exists, about 20 changed lines"
    );
    for key in ["max_tasks", "max_readers", "max_bounces"] {
        assert!(limits[key].is_u64(), "{key}");
    }
    assert_eq!(c["roster"].as_array().unwrap().len(), run.roster.len());
    // Every report, the onboarding one first as `onboarding`, and the queued scout.
    assert_eq!(
        ids(&c, "scouts", "id"),
        ["onboarding", "3f9a-ref", "3f9a-a", "3f9a-b", "3f9a-q"]
    );
    assert_eq!(
        c["scouts"][2],
        json!({
            "id": "3f9a-a", "state": "reported", "question": "What is in 3f9a-a?",
            "area": ["crates/a/src/**"], "summary": "The a crate.",
            "files": [{"path": "crates/x/src/f0.rs", "why": "it matters"},
                      {"path": "crates/x/src/f1.rs", "why": "it matters"}],
            "modules": ["crates/x"], "interfaces": ["fn x()"], "risks": ["none"],
        })
    );
    assert_eq!(c["scouts"][4]["state"], "queued");
    assert_eq!(c["scouts"][4]["summary"], Value::Null);
    assert_eq!(
        c["epics"][0],
        json!({"epic": "a", "title": "Epic a", "area": ["crates/a/**"], "state": "planning", "tasks": 1})
    );
    assert_eq!(ids(&c, "plan", "id"), ["t0", "a1", "b1"]);
    assert_eq!(
        c["plan"][0],
        json!({"id": "t0", "title": "Title t0", "epic": null, "kind": "code", "size": "M",
               "hub": true, "owns": ["crates/proto/**"], "deps": [], "state": "pending"})
    );
    assert_eq!(c["omitted"], json!({"scouts": 0}));
}

#[test]
fn context_for_a_planner_filters_reports_and_tasks() {
    let run = planning_run();
    let c = ask(&run, Asker::Planner { epic: "a".into() }, None);
    assert_eq!(
        c["you"],
        json!({"role": "planner", "epic": {"epic": "a", "title": "Epic a",
               "area": ["crates/a/**"], "brief": "Plan the a crate."}})
    );
    // Its scout_refs, the reports whose area meets its own, and the onboarding one.
    assert_eq!(
        ids(&c, "scouts", "id"),
        ["onboarding", "3f9a-ref", "3f9a-a", "3f9a-q"]
    );
    // Tasks with no epic, and its own.
    assert_eq!(ids(&c, "plan", "id"), ["t0", "a1"]);
    assert_eq!(ids(&c, "epics", "epic"), ["a", "b"]);
}

#[test]
fn only_listed_scouts_are_returned() {
    let run = planning_run();
    let only = Some(vec!["3f9a-b".to_string(), "onboarding".to_string()]);
    let c = ask(&run, Asker::Orchestrator, only);
    assert_eq!(ids(&c, "scouts", "id"), ["onboarding", "3f9a-b"]);
    // A planner's filter still applies to a listed report outside its epic.
    let only = Some(vec!["3f9a-b".to_string(), "3f9a-a".to_string()]);
    let c = ask(&run, Asker::Planner { epic: "a".into() }, only);
    assert_eq!(ids(&c, "scouts", "id"), ["3f9a-a"]);
}

#[test]
fn installed_is_carried() {
    let run = planning_run();
    let c = ask(&run, Asker::Orchestrator, None);
    for entry in c["roster"].as_array().unwrap() {
        let expected = entry["runtime"] == "claude";
        assert_eq!(entry["installed"], expected, "{entry}");
        for key in ["model", "strength", "note"] {
            assert!(entry[key].is_string(), "{key}: {entry}");
        }
    }
    // A run built before the check (no entry) lists nothing as installed.
    let mut run = planning_run();
    run.orch.installed.clear();
    let c = ask(&run, Asker::Orchestrator, None);
    assert!(
        c["roster"]
            .as_array()
            .unwrap()
            .iter()
            .all(|e| e["installed"] == false)
    );
}

#[test]
fn context_names_the_base_sha() {
    let mut run = planning_run();
    run.base_sha = "1a2b3c4d".repeat(5);
    let c = ask(&run, Asker::Planner { epic: "a".into() }, None);
    assert_eq!(c["run"]["base_sha"], run.base_sha);
    assert_eq!(c["run"]["root"], "/tmp/x");
}

#[test]
fn context_is_capped_at_96_kib_with_omitted_counts() {
    let mut run = planning_run();
    run.orch.run_scouts = (0..12)
        .map(|i| scout(&format!("3f9a-s{i:02}"), RunScoutState::Reported, &["x/**"]))
        .collect();
    let reports: Vec<ScoutReport> = (0..12)
        .map(|i| {
            report(
                &format!("3f9a-s{i:02}"),
                ScoutKind::Area,
                &"s".repeat(20_000),
                60,
            )
        })
        .collect();
    let c = context(&ContextInputs {
        run: &run,
        asker: Asker::Orchestrator,
        profile: None,
        reports,
        only: None,
    });
    assert!(size(&c) <= CONTEXT_MAX_BYTES, "{}", size(&c));
    let scouts = c["scouts"].as_array().unwrap();
    assert_eq!(scouts.len(), 12);
    let len = |i: usize| scouts[i]["summary"].as_str().unwrap().chars().count();
    // Each summary is cut to 8000 characters; the later ones, to 1000.
    assert_eq!(len(0), SUMMARY_MAX + 1); // with `…`
    assert_eq!(len(11), SUMMARY_TRIMMED + 1);
    let cut = (0..12).filter(|i| len(*i) == SUMMARY_TRIMMED + 1).count();
    assert!(cut > 0 && cut < 12, "{cut}");
    // Later reports are cut before earlier ones.
    assert!((0..11).all(|i| len(i) >= len(i + 1)));
    assert_eq!(c["omitted"]["scouts"], cut as u64);
    assert_eq!(c["profile"]["summary"], Value::Null);
}

#[test]
fn file_lists_go_after_the_summaries() {
    let mut run = planning_run();
    run.orch.run_scouts = (0..40)
        .map(|i| scout(&format!("3f9a-s{i:02}"), RunScoutState::Reported, &["x/**"]))
        .collect();
    let reports: Vec<ScoutReport> = (0..40)
        .map(|i| {
            report(
                &format!("3f9a-s{i:02}"),
                ScoutKind::Area,
                &"s".repeat(2_000),
                60,
            )
        })
        .collect();
    let c = context(&ContextInputs {
        run: &run,
        asker: Asker::Orchestrator,
        profile: None,
        reports,
        only: None,
    });
    assert!(size(&c) <= CONTEXT_MAX_BYTES, "{}", size(&c));
    let scouts = c["scouts"].as_array().unwrap();
    assert!(scouts[39]["files"].as_array().unwrap().is_empty());
    assert_eq!(scouts[0]["files"].as_array().unwrap().len(), 60);
    let shortened = scouts
        .iter()
        .filter(|s| s["summary"].as_str().unwrap().chars().count() <= SUMMARY_TRIMMED + 1)
        .count();
    assert_eq!(
        shortened, 40,
        "every summary is cut before any file list goes"
    );
    assert_eq!(c["omitted"]["scouts"], 40);
}

/// Carry-forward rule: a scout's text stays inside its JSON string.
#[test]
fn untrusted_text_stays_inside_its_json_string() {
    let run = planning_run();
    let mut forged = reports();
    forged[2].summary = FORGED.into();
    forged[2].files[0].why = FORGED.into();
    let c = context(&ContextInputs {
        run: &run,
        asker: Asker::Orchestrator,
        profile: None,
        reports: forged,
        only: None,
    });
    assert_contained(&c);
}
