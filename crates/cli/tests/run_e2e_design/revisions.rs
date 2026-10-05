//! The user's ways back at a gate (decision 7): changes at each gate, each making the
//! next version; a back from the spec to the brainstorm; and a rethink, which runs both
//! brainstormers again.

use proto::{DocGateKind, DocKind, RunInfo, RunState};
use serde_json::{Value, json};

use crate::common::*;
use crate::support::orch_script::*;
use crate::support::run_design::*;
use crate::support::run_orch::ORCH_WAIT;

/// The merged report, revised: its judgment names the user's answer.
fn report_v2() -> String {
    let report = report(LABELS);
    let revised = report.replace(
        "Judgment: write it directly.",
        "Judgment: write it directly, with the newline the user asked for.",
    );
    assert_ne!(revised, report);
    revised
}

/// The spec, revised: its edge case says what happens to an old line.
fn spec_v2() -> String {
    let revised = SPEC.replace(
        "An existing a.txt is replaced.",
        "An existing a.txt is replaced; its old line is dropped.",
    );
    assert_ne!(revised, SPEC);
    revised
}

/// The versions of `kind` the run lists, each with its reason.
fn versions(info: &RunInfo, kind: DocKind) -> Vec<(u32, String)> {
    (info.docs.iter())
        .filter(|d| d.kind == kind)
        .map(|d| (d.version, d.reason.clone()))
        .collect()
}

/// `amend_task` of `t1`'s brief, with a line added: a change to the plan's text.
fn amended_brief() -> Value {
    let brief = format!("{}Keep the line short.\n", brief(PLAN_FILE));
    json!({"op": "amend_task", "task_id": "t1", "brief": brief})
}

/// The scripts a run through all three gates needs: both drafts, the spec's and the
/// plan's first reviews with no findings, and `t1`.
fn agents(h: &crate::support::run_harness::RunHarness) {
    drafts(h);
    h.reviewer("spec", 1, 1, Findings::Submits(vec![]));
    h.reviewer("plan", 1, 1, Findings::Submits(vec![]));
    green(h, "t1", PLAN_FILE);
}

#[test]
fn e2e_changes_at_each_gate_makes_a_new_version() {
    const NOTE_B: &str = "say why the newline matters";
    const NOTE_S: &str = "say what happens to an old line";
    const NOTE_P: &str = "keep the line short";
    let h = harness("");
    agents(&h);
    let mut steps = ask(None);
    steps.extend(merge(&labels(), &report(LABELS)));
    steps.push(revising(NOTE_B));
    let report_v2 = report_v2();
    steps.push(call(
        "submit_doc",
        json!({"kind": "brainstorm", "text": report_v2}),
    ));
    steps.push(approved("brainstorm"));
    steps.extend(spec_for_review(SPEC, 1));
    steps.push(spec_ready(SPEC, &[]));
    // DF §4.2: a revision the user asked no review of is submitted ready directly.
    steps.push(revising(NOTE_S));
    steps.push(spec_ready(&spec_v2(), &[]));
    steps.push(approved("spec"));
    steps.extend(plan_for_review(plan_edits()));
    steps.push(plan_ready(&[]));
    // Ruling T7-8: while the plan gate revises, the orchestrator may change the plan,
    // and its next submit opens v2 with no second review (ruling T7-4).
    steps.push(revising(NOTE_P));
    steps.push(edit_plan(vec![amended_brief()], json!({"submit": true})));
    steps.push(expect("/awaiting_approval", json!(true)));
    steps.push(approved("plan"));
    steps.extend([marker(), read(None)]);
    let (run, _) = start(&h, &steps, &[]);

    for (kind, note) in [
        (DocGateKind::Brainstorm, NOTE_B),
        (DocGateKind::Spec, NOTE_S),
        (DocGateKind::Plan, NOTE_P),
    ] {
        let gate = kind.label();
        h.wait_doc_gate(&run, kind, 1, ORCH_WAIT);
        ok(
            &h,
            &["run", "changes", &run, "--gate", gate, "--note", note],
        );
        let info = h.wait_doc_gate(&run, kind, 2, ORCH_WAIT);
        // The version the user saw first is not the one waiting now (ruling T17-1).
        let stale = ["run", "approve", &run, "--gate", gate, "--version", "1"];
        refused(
            &h,
            &stale,
            &format!("the {gate} is now v2; review it before approving"),
        );
        let doc = match kind {
            DocGateKind::Brainstorm => DocKind::Brainstorm,
            DocGateKind::Spec => DocKind::Spec,
            DocGateKind::Plan => DocKind::Plan,
        };
        let listed: Vec<u32> = versions(&info, doc).iter().map(|v| v.0).collect();
        assert_eq!(listed, [1, 2], "{gate}: {:?}", versions(&info, doc));
        approve(&h, &run, kind, 2);
    }
    wait_passed(&h, 1);
    assert!(
        refusals(&h, "edit_plan").is_empty(),
        "{:?}",
        refusals(&h, "edit_plan")
    );
    assert!(
        refusals(&h, "submit_doc").is_empty(),
        "{:?}",
        refusals(&h, "submit_doc")
    );
    // Each v2 holds the revision: the stored files.
    let report = stored(&h, &run, "brainstorm-v2.md");
    assert!(report.starts_with(&report_v2), "{report}");
    assert_eq!(stored(&h, &run, "spec-v2.md"), spec_v2());
    let plan = stored(&h, &run, "plan-v2.md");
    assert!(plan.contains("Keep the line short."), "{plan}");
    // The run runs the approved plan v2.
    h.wait_run(&run, |r| r.state == RunState::Complete, DESIGN_RUN_WAIT);
}

#[test]
fn e2e_back_from_spec_to_brainstorm() {
    const NOTE: &str = "the brainstorm missed the generator's cost";
    let h = harness("");
    agents(&h);
    let report_v2 = report_v2();
    let mut steps = ask(None);
    steps.extend(merge(&labels(), &report(LABELS)));
    steps.push(approved("brainstorm"));
    steps.extend(spec_for_review(SPEC, 1));
    steps.push(spec_ready(SPEC, &[]));
    // The back reopens the brainstorm gate, revising with the user's note.
    steps.push(until("/gate/doc_gate/kind", json!("brainstorm"), ORCH_WAIT));
    steps.push(revising(NOTE));
    steps.push(call(
        "submit_doc",
        json!({"kind": "brainstorm", "text": report_v2}),
    ));
    steps.push(approved("brainstorm"));
    steps.push(spec_ready(&spec_v2(), &[]));
    steps.push(approved("spec"));
    steps.extend(plan_for_review(plan_edits()));
    steps.push(plan_ready(&[]));
    steps.push(approved("plan"));
    steps.extend([marker(), read(None)]);
    let (run, _) = start(&h, &steps, &[]);

    approve(&h, &run, DocGateKind::Brainstorm, 1);
    h.wait_doc_gate(&run, DocGateKind::Spec, 1, ORCH_WAIT);
    let out = ok(&h, &["run", "back", &run, "--gate", "spec", "--note", NOTE]);
    assert!(out.contains("back to the brainstorm"), "{out}");
    // The brainstorm v2, approved; then the spec v2, which was not sent for review.
    let info = approve(&h, &run, DocGateKind::Brainstorm, 2);
    assert_eq!(versions(&info, DocKind::Spec).len(), 1);
    let info = approve(&h, &run, DocGateKind::Spec, 2);
    let gate = info.doc_gate.unwrap();
    assert_eq!(
        gate.not_reviewed.as_deref(),
        Some("the orchestrator submitted it without a review")
    );
    approve(&h, &run, DocGateKind::Plan, 1);
    wait_passed(&h, 1);
    let info = h.wait_run(&run, |r| r.state == RunState::Complete, DESIGN_RUN_WAIT);
    let brainstorms: Vec<u32> = versions(&info, DocKind::Brainstorm)
        .iter()
        .map(|v| v.0)
        .collect();
    assert_eq!(brainstorms, [1, 2]);
    // The commit holds the spec the user approved last: v2.
    let spec_path = doc_path(&h, &run, "specs", "");
    let branch = format!("{}:{spec_path}", info.run_branch);
    assert_eq!(git_text(&h.repo, &["cat-file", "blob", &branch]), spec_v2());
    for tool in ["submit_doc", "edit_plan"] {
        assert!(
            refusals(&h, tool).is_empty(),
            "{tool}: {:?}",
            refusals(&h, tool)
        );
    }
}

#[test]
fn e2e_rethink() {
    const NOTE: &str = "weigh a template file as well";
    let h = harness("");
    agents(&h);
    // The rethought drafts differ from the first ones, so a stale read shows.
    let second = |label: &str| draft(label).replace("reading)", "second reading)");
    for label in LABELS {
        h.brainstormer(label, 2, Draft::Text(&second(label)));
    }
    // The orchestrator reads each round's drafts from their wake-up (ruling T20-1: a
    // wake note is pasted once, so a duplicate of the first can never pass for the
    // second).
    let report_v2 = report_v2();
    let mut steps = ask(None);
    steps.extend(merge(&labels(), &report(LABELS)));
    steps.push(until("/gate/state", json!("brainstorming"), ORCH_WAIT));
    steps.extend(merge(&labels(), &report_v2));
    steps.push(approved("brainstorm"));
    steps.extend([marker(), read(None)]);
    let (run, _) = start(&h, &steps, &[]);

    h.wait_doc_gate(&run, DocGateKind::Brainstorm, 1, ORCH_WAIT);
    let out = ok(&h, &["run", "rethink", &run, "--note", NOTE]);
    assert!(out.contains("rethought"), "{out}");
    let info = approve(&h, &run, DocGateKind::Brainstorm, 2);
    let log = crate::support::run_pr::log_lines(&h, &run);
    let asked = "the user asked to rethink the brainstorm v1".to_string();
    assert!(log.contains(&asked), "{log:#?}");
    let drafts = (log.iter()).filter(|l| *l == "the brainstorm drafts are in");
    assert_eq!(drafts.count(), 2, "{log:#?}");
    wait_passed(&h, 1);
    assert_eq!(h.run(&run).unwrap().state, RunState::Specifying);
    let sessions: Vec<_> = (info.design_agents.iter())
        .map(|a| (a.label.as_str(), a.sessions))
        .collect();
    // One row an agent (ruling T18-1), in its second session.
    assert_eq!(sessions, [("claude", 2), ("codex", 2)]);

    // Both brainstormers ran a second session, each told the user's note: the frozen
    // pack of brainstorm round 2 (ruling T8-6) carries it.
    let pack = stored(&h, &run, "brainstorm/pack-r2.md");
    assert!(pack.contains(NOTE), "{pack}");
    let codex = h.codex_messages(&brainstormer_name("codex", 2));
    assert_eq!(codex.len(), 1, "{codex:#?}");
    assert!(codex[0].contains(NOTE), "{}", codex[0]);
    for label in LABELS {
        let sessions = h.io_lines(&brainstormer_name(label, 2), "args");
        assert_eq!(sessions.len(), 1, "{label}: {sessions:#?}");
    }
    // The orchestrator read the second drafts, and the report v2 carries them in its
    // appendix.
    let reads: Vec<String> = (h.mcp_log().iter())
        .filter(|l| l["script"] == ORCH && l["tool"] == "get_doc")
        .map(|l| l["result"].as_str().unwrap_or_default().to_string())
        .collect();
    assert_eq!(reads.len(), 4, "{reads:#?}");
    for (read, label) in reads[2..].iter().zip(LABELS) {
        assert_eq!(read, &second(label));
    }
    let report = stored(&h, &run, "brainstorm-v2.md");
    assert!(report.starts_with(&report_v2), "{report}");
    let appendix = &report[report.find("## Appendix: the drafts").expect(&report)..];
    for label in LABELS {
        assert!(
            appendix.contains(&format!("({label}'s second reading)")),
            "{appendix}"
        );
    }
}
