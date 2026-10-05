//! The whole path of a design goal (DF §2): the questions, two drafts and the merged
//! report at gate 1; the spec through one review with a disputed finding at gate 2; a
//! plan refused for coverage, then through its review to gate 3; the documents commit;
//! the tasks; REPORT.md's requirements; and the accept that lands the documents.

use std::time::Instant;

use proto::{DocGateKind, RunState, TaskState};
use serde_json::json;

use crate::common::*;
use crate::support::orch_script::*;
use crate::support::run_design::*;
use crate::support::run_harness::RUN_WAIT;
use crate::support::run_orch::ORCH_WAIT;
use crate::support::run_plans::report_with;

/// `e2e_a_goal_through_three_gates`'s deadline (`docs/timing-budgets.md`, "Recorded,
/// from M9.6.20"): from the test's start to the run's completion, three task paths by
/// `RUN_WAIT`'s convention, as the brief sets it.
const THREE_GATES_WAIT: std::time::Duration = RUN_WAIT.saturating_mul(3);

/// The spec review's finding the orchestrator keeps: a disputed finding (decision 16).
const KEPT_REASON: &str = "one task writes both lines; a split adds a merge for nothing";

#[test]
fn e2e_a_goal_through_three_gates() {
    let started = Instant::now();
    let h = harness("");
    drafts(&h);
    let split = finding(
        "F1",
        "minor",
        "Design",
        "split the file's two lines into tasks",
    );
    h.reviewer("spec", 1, 1, Findings::Submits(vec![split]));
    let title = finding("P1", "minor", "t1", "name a.txt in the task's title");
    h.reviewer("plan", 1, 1, Findings::Submits(vec![title]));
    green(&h, "t1", PLAN_FILE);
    // The ready spec ends in a blank line: the commit must hold it untrimmed.
    let spec = format!("{SPEC}\n");

    let mut steps = ask(Some(ANSWER));
    steps.extend(merge(&labels(), &report(LABELS)));
    steps.push(approved("brainstorm"));
    steps.extend(spec_for_review(SPEC, 1));
    steps.push(spec_ready(&spec, &[kept("F1", KEPT_REASON)]));
    steps.push(approved("spec"));
    // Decision 18: a plan that leaves R2 uncovered is refused, and nothing is applied.
    let short = vec![covered("t1", PLAN_FILE, &["R1"], json!({}))];
    steps.push(call_err(
        "edit_plan",
        json!({"edits": short, "submit": true}),
    ));
    steps.push(expect_error("R2 are covered by no task"));
    steps.extend(plan_for_review(plan_edits()));
    steps.push(plan_ready(&[fixed("P1")]));
    steps.push(expect("/awaiting_approval", json!(true)));
    steps.push(approved("plan"));
    steps.extend([marker(), read(None)]);
    let (run, _) = start(&h, &steps, &[]);

    // Gate 1: the merged report, v1. An approve of a version the user never saw is
    // refused (ruling T17-1); the shown version's approve passes.
    let info = h.wait_doc_gate(&run, DocGateKind::Brainstorm, 1, ORCH_WAIT);
    assert_eq!(info.state, RunState::AwaitingApproval);
    let stale = [
        "run",
        "approve",
        &run,
        "--gate",
        "brainstorm",
        "--version",
        "2",
    ];
    refused(
        &h,
        &stale,
        "the brainstorm is now v1; review it before approving",
    );
    approve(&h, &run, DocGateKind::Brainstorm, 1);
    // The user's answer reached both brainstormers (their frozen pack, ruling T8-6),
    // and the report the user saw carries both drafts in the engine's appendix.
    let pack = stored(&h, &run, "brainstorm/pack-r1.md");
    assert!(pack.contains(ANSWER), "{pack}");
    let shown = stored(&h, &run, "brainstorm-v1.md");
    let appendix = format!(
        "{}\n## Appendix: the drafts\n\n### claude\n",
        report(LABELS)
    );
    assert!(shown.starts_with(&appendix), "{shown}");
    assert!(shown.contains("(codex's reading)"), "{shown}");

    // Gate 2: the spec v1, its review's finding kept and shown as disputed.
    let info = approve(&h, &run, DocGateKind::Spec, 1);
    let gate = info.doc_gate.as_ref().unwrap();
    let disputed: Vec<_> = gate.disputed.iter().map(|f| (&*f.id, &*f.text)).collect();
    assert_eq!(disputed, [("F1", "split the file's two lines into tasks")]);
    assert_eq!(gate.not_reviewed, None);

    // Gate 3: the plan v1, after one coverage refusal and nothing else refused.
    let info = h.wait_doc_gate(&run, DocGateKind::Plan, 1, ORCH_WAIT);
    assert_eq!(
        refusals(&h, "edit_plan").len(),
        1,
        "{:?}",
        refusals(&h, "edit_plan")
    );
    assert!(refusals(&h, "submit_doc").is_empty());
    assert_eq!(info.tasks.len(), 1);
    assert_eq!(info.tasks[0].covers, ["R1", "R2"]);
    approve(&h, &run, DocGateKind::Plan, 1);
    wait_passed(&h, 1);

    // The tasks run, then the run completes.
    let left = THREE_GATES_WAIT.saturating_sub(started.elapsed());
    let info = h.wait_run(&run, |r| r.state == RunState::Complete, left);
    assert_eq!(info.tasks[0].state, TaskState::Merged);

    // The documents commit: the run branch's first commit past the base, holding the
    // approved spec's stored bytes exactly, and the plan's.
    let spec_path = doc_path(&h, &run, "specs", "");
    let plan_path = doc_path(&h, &run, "plans", "");
    let range = format!("{}..{}", info.base_sha, info.run_branch);
    let firsts = h.git(&["rev-list", "--first-parent", "--reverse", &range]);
    let docs = firsts
        .lines()
        .next()
        .expect("a commit past the base")
        .to_string();
    let subject = h.git(&["log", "-1", "--format=%s", &docs]);
    assert_eq!(subject, format!("docs: spec and plan for {GOAL}"));
    let stored_spec = stored(&h, &run, "spec-v1.md");
    assert_eq!(stored_spec, spec, "the stored spec");
    let blob =
        |rev: &str, path: &str| git_text(&h.repo, &["cat-file", "blob", &format!("{rev}:{path}")]);
    assert_eq!(blob(&docs, &spec_path), stored_spec);
    assert_eq!(blob(&docs, &plan_path), stored(&h, &run, "plan-v1.md"));
    let files = h.git(&["diff-tree", "--no-commit-id", "--name-only", "-r", &docs]);
    let mut expected = vec![plan_path.as_str(), spec_path.as_str()];
    expected.sort_unstable();
    assert_eq!(files.lines().collect::<Vec<_>>(), expected);

    // REPORT.md's requirements: both covered by t1, which merged.
    let report = report_with(&info, "## Requirements");
    let table =
        "| Req | Tasks | Outcome |\n|---|---|---|\n| R1 | t1 | merged |\n| R2 | t1 | merged |\n";
    assert!(report.contains(table), "{report}");
    assert!(report.contains("\ndesign phases: "), "{report}");
    assert!(report.contains(" calls, "), "{report}");
    assert!(report.contains(", 3 gate versions\n"), "{report}");

    // The accept lands the documents on main with the work.
    h.accept(&run);
    assert_eq!(blob("main", &spec_path), stored_spec);
    assert_eq!(h.git(&["show", &format!("main:{PLAN_FILE}")]), "t1");
}
