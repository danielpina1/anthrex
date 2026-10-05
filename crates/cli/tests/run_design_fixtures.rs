//! Milestone 9.6 task M9.6.19: the design flow's fixture documents and scripted agents
//! (`support/run_design.rs`) pass task M9.6.4's real checks, so an end-to-end test
//! that submits them is refused only where it means to be. No daemon runs here.

mod support;

use std::collections::BTreeSet;

use daemon::run::design::coverage;
use daemon::run::design::requirements::{self, Requirement};
use daemon::run::design::template::{self, TemplateCtx};
use proto::DocKind;
use serde_json::Value;
use support::run_design::*;

fn admit(kind: DocKind, text: &str, ctx: &TemplateCtx) {
    if let Err(refusal) = template::admit(kind, text, ctx) {
        panic!("the {kind:?} fixture is refused: {refusal}\n{text}");
    }
}

fn ctx(labels: [&str; 2]) -> TemplateCtx {
    TemplateCtx {
        labels: labels.map(String::from).to_vec(),
        ..TemplateCtx::default()
    }
}

fn ids(requirements: &[Requirement]) -> Vec<&str> {
    requirements.iter().map(|r| r.id.as_str()).collect()
}

/// Every `mcp_call` of `tool` among `steps`: its arguments.
fn calls<'a>(steps: &'a [Value], tool: &str) -> Vec<&'a Value> {
    steps
        .iter()
        .filter(|s| s["mcp_call"]["tool"] == tool)
        .map(|s| &s["mcp_call"]["args"])
        .collect()
}

#[test]
fn the_fixture_documents_pass_the_templates() {
    for label in ["claude", "codex", "A", "B"] {
        admit(
            DocKind::BrainstormDraft,
            &draft(label),
            &TemplateCtx::default(),
        );
        let steps = brainstormer_script(label, Draft::Fixture);
        let submits = calls(&steps, "submit_doc");
        assert_eq!(submits.len(), 1, "{label}");
        assert_eq!(submits[0]["kind"], "brainstorm_draft");
        assert_eq!(submits[0]["text"], draft(label).as_str());
        let after = brainstormer_script(label, Draft::AfterNudge);
        assert_eq!(
            calls(&after, "submit_doc"),
            submits,
            "{label} after its nudge"
        );
    }

    for pair in [["claude", "codex"], ["A", "B"]] {
        let report = report(pair);
        admit(DocKind::Brainstorm, &report, &ctx(pair));
        // The approaches are `### <name> [tag]` headings.
        assert!(report.contains("\n### 1. Write the file directly [both]\n"));
        assert!(report.contains(&format!(
            "\n### 2. Generate it from a script [{}]\n",
            pair[1]
        )));
        let failed = TemplateCtx {
            failed: Some((pair[1].to_string(), "overloaded".to_string())),
            ..ctx(pair)
        };
        admit(
            DocKind::Brainstorm,
            &single_report(pair[0], pair[1], "overloaded"),
            &failed,
        );
    }

    for ready in [false, true] {
        let ctx = TemplateCtx {
            ready,
            ..TemplateCtx::default()
        };
        admit(DocKind::Spec, SPEC, &ctx);
        admit(DocKind::Spec, AMENDMENT, &ctx);
    }
    let approved = requirements::parse(SPEC).unwrap();
    assert_eq!(ids(&approved), REQUIREMENTS);
    let amended = requirements::parse_amendment(AMENDMENT, &approved).unwrap();
    assert_eq!(ids(&amended), ["R3"]);

    // The plan: every brief has decision 19's headings, every task covers only the
    // spec's requirements, and together they cover all of them.
    let known: BTreeSet<&str> = REQUIREMENTS.into_iter().collect();
    let mut all = BTreeSet::new();
    let edits = plan_edits();
    for edit in &edits {
        let task = &edit["task"];
        let brief = task["brief"].as_str().unwrap();
        assert_eq!(coverage::missing_heading(brief), None, "{brief}");
        let covers: Vec<&str> = (task["covers"].as_array().unwrap().iter())
            .map(|c| c.as_str().unwrap())
            .collect();
        assert!(!covers.is_empty() && covers.iter().all(|c| known.contains(c)));
        all.extend(covers);
    }
    assert_eq!(all, known, "the plan covers every requirement");
    // Ruling T11-1: a design run's sub-planner spawn names the requirements it owns.
    let spawn = subplanner("api", &["src/api"], &["R2"]);
    assert_eq!(
        spawn["mcp_call"]["args"]["covers"],
        serde_json::json!(["R2"])
    );

    // The scripted orchestrator submits exactly these documents.
    let steps = orch_design_steps(&OrchDesign::fixture(["claude", "codex"]));
    let docs: Vec<(&str, &str)> = (calls(&steps, "submit_doc").into_iter())
        .map(|a| (a["kind"].as_str().unwrap(), a["text"].as_str().unwrap()))
        .collect();
    let report = report(["claude", "codex"]);
    let expected = [
        ("brainstorm", report.as_str()),
        ("spec", SPEC),
        ("spec", SPEC),
    ];
    assert_eq!(docs, expected);
    let reads: Vec<&Value> = calls(&steps, "get_doc");
    assert_eq!(reads.len(), 2, "both drafts are read");
    let start = calls(&steps, "start_brainstorm");
    assert_eq!(start, [&serde_json::json!({"answers": ANSWER})]);
}

/// A reviewer's script reads its own document: a spec review its draft by number
/// (ruling T5-1), a plan review the plan and the approved spec; then one submit.
#[test]
fn reviewer_scripts_read_their_document_then_submit_once() {
    let found = vec![finding("F1", "blocking", "R2", "untestable")];
    let spec = reviewer_script("spec", 2, Findings::Submits(found.clone()));
    let reads = calls(&spec, "get_doc");
    assert_eq!(reads, [&serde_json::json!({"kind": "spec", "draft": 2})]);
    let plan = reviewer_script("plan", 1, Findings::Submits(vec![]));
    let reads = calls(&plan, "get_doc");
    assert_eq!(
        reads,
        [
            &serde_json::json!({"kind": "plan"}),
            &serde_json::json!({"kind": "spec"})
        ]
    );
    for (steps, n) in [(&spec, 1), (&plan, 0)] {
        let submits = calls(steps, "submit_findings");
        assert_eq!(submits.len(), 1);
        assert_eq!(submits[0]["findings"].as_array().unwrap().len(), n);
    }
    let unsubmitted = reviewer_script("spec", 1, Findings::Unsubmitted);
    assert!(calls(&unsubmitted, "submit_findings").is_empty());
    assert_eq!(
        unsubmitted.last().unwrap(),
        &serde_json::json!({"end_turn": {}})
    );
    assert_eq!(reviewer_name("spec", 3, 2), "doc_reviewer-spec-r3-2");
    assert_eq!(brainstormer_name("codex", 2), "brainstormer-codex-2");
}
