//! Milestone 9.6 task M9.6.20: a design run's second round amends its spec (decisions
//! 14, 24, 28 and 29; DF §8.1), end to end through the real binary and a real daemon
//! on temporary paths, with `fake-agent` as every agent (the harness pins every
//! `*_BIN`); no real agent and no `gh`.

mod support;

// The design scenarios' shared helpers; this binary uses some of them.
#[allow(dead_code)]
#[path = "run_e2e_design/common.rs"]
mod common;

use common::*;
use proto::{DocGateKind, RunState, TaskState};
use serde_json::json;
use support::orch_script::*;
use support::run_design::*;
use support::run_orch::ORCH_WAIT;
use support::run_rounds::{ITERATE_WAIT, SUMMARY_1, SUMMARY_2, SUMMARY_WAIT, h4};

/// A design round's completion with its summary, from its plan's approval
/// (`docs/timing-budgets.md`, "Recorded, from M9.6.20"): `SUMMARY_WAIT` and the
/// round's documents commit before its task path.
const DESIGN_SUMMARY_WAIT: std::time::Duration = SUMMARY_WAIT.saturating_add(DOCS_COMMIT_WAIT);

/// Round 2's amendment: R2 changed (it keeps its number), R3 new, continuing from
/// round 1's R2.
const AMENDED: &str = "# Add b.txt too

## Goal and success criteria
b.txt exists on the base branch beside a.txt, and a.txt ends in a newline.

## Non-goals
a.txt's line does not change.

## Approach
Write b.txt directly, as round 1 wrote a.txt.

## Design
One task writes b.txt in the repository root and checks a.txt's newline.

## Requirements
R2 a.txt holds exactly one line, ending in a newline. Acceptance: `tail -c1 a.txt | od -c` shows `\\n`.
R3 b.txt exists at the repository root. Acceptance: `test -f b.txt` succeeds.

## Interfaces
None.

## Errors and edge cases
An existing b.txt is replaced.

## Testing
A check-mode task; its reviewer reads the files.

## Risks
None.

## Open questions
";

/// The changed R2 as the approved requirements hold it: marked (decision 14).
const R2_CHANGED: &str = "a.txt holds exactly one line, ending in a newline. Acceptance: `tail -c1 a.txt | od -c` shows `\\n`. (changed in round 2)";

#[test]
fn e2e_round_amend() {
    let h = harness("");
    drafts(&h);
    for (kind, k) in [("spec", 1), ("plan", 1), ("spec", 2), ("plan", 2)] {
        h.reviewer(kind, k, 1, Findings::Submits(vec![]));
    }
    green(&h, "t1", PLAN_FILE);
    green(&h, "t2", "b.txt");
    // The orchestrator keeps its turn open through round 1 (it reads the drafts when
    // the test has seen them in), so the round-2 request is the first wake-up pasted.
    let mut steps = ask(None);
    steps.push(typed());
    steps.extend(submit_merged(&labels(), &report(LABELS)));
    steps.push(approved("brainstorm"));
    steps.extend(spec_for_review(SPEC, 1));
    steps.push(spec_ready(SPEC, &[]));
    steps.push(approved("spec"));
    steps.extend(plan_for_review(plan_edits()));
    steps.push(plan_ready(&[]));
    steps.push(approved("plan"));
    steps.push(until("/run/complete", json!(true), DESIGN_SUMMARY_WAIT));
    steps.push(edit_plan(vec![], json!({"summary": SUMMARY_1})));
    steps.push(until("/run/round", json!(2), ITERATE_WAIT));
    steps.push(read(Some("the user asks for round 2 of run ")));
    // Round 2 amends the spec: through its review (run-wide review 2), then ready.
    steps.extend(spec_for_review(AMENDED, 2));
    steps.push(spec_ready(AMENDED, &[]));
    steps.push(approved("spec"));
    // Decision 29: the round's plan covers only the new and changed requirements; R1,
    // covered in round 1, is not asked for.
    let t2 = |covers: &[&str]| covered("t2", "b.txt", covers, json!({"stage": 2}));
    steps.push(call_err(
        "edit_plan",
        json!({"edits": [t2(&["R3"])], "submit": true}),
    ));
    steps.push(expect_error("R2 are covered by no task"));
    steps.extend(plan_for_review(vec![t2(&["R2", "R3"])]));
    steps.push(plan_ready(&[]));
    steps.push(approved("plan"));
    steps.push(until("/run/complete", json!(true), DESIGN_SUMMARY_WAIT));
    steps.push(edit_plan(vec![], json!({"summary": SUMMARY_2})));
    steps.extend([marker(), read(None)]);
    let (run, window) = start(&h, &steps, &[]);
    drafts_in(&h, &run, 1);
    go(&h, window);
    for kind in [
        DocGateKind::Brainstorm,
        DocGateKind::Spec,
        DocGateKind::Plan,
    ] {
        approve(&h, &run, kind, 1);
    }
    h.wait_summary(&run, SUMMARY_1, DESIGN_SUMMARY_WAIT);
    let round1 = h.run(&run).unwrap();
    let round1_head = h.git(&["rev-parse", &round1.run_branch]);

    // `run iterate` of a design run amends by default (decision 28).
    let out = h.iterate(&run, "also add b.txt", "");
    support::run_rounds::ok(&out);
    let started = format!(
        "run {} round 2 started; its orchestrator plans it",
        h4(&run)
    );
    assert_eq!(String::from_utf8_lossy(&out.stdout).trim(), started);
    let info = h.wait_run(&run, |r| r.round == 2, ORCH_WAIT);
    assert_eq!(info.state, RunState::Specifying);
    // The round's spec gate is the spec's v2, its plan gate the plan's v2.
    approve(&h, &run, DocGateKind::Spec, 2);
    let info = approve(&h, &run, DocGateKind::Plan, 2);
    let t2 = info.tasks.iter().find(|t| t.id == "t2").unwrap();
    assert_eq!(t2.covers, ["R2", "R3"]);
    let refused = refusals(&h, "edit_plan");
    assert_eq!(refused.len(), 1, "{refused:?}");
    assert!(
        refused[0].contains("R2 are covered by no task"),
        "{refused:?}"
    );
    let info = h.wait_summary(&run, SUMMARY_2, DESIGN_SUMMARY_WAIT);
    wait_passed(&h, 1);
    assert!(info.tasks.iter().all(|t| t.state == TaskState::Merged));

    // The amendment continued the numbering and marked its change: the approved
    // requirements are R1 as round 1 approved it, R2 changed, R3 new.
    let design = &h.run_json(&run)["orch"]["design"];
    let ids: Vec<_> = (design["requirements"].as_array().unwrap().iter())
        .map(|r| (r["id"].as_str().unwrap(), r["text"].as_str().unwrap()))
        .collect();
    assert_eq!(ids[1], ("R2", R2_CHANGED), "{ids:#?}");
    assert_eq!(
        ids.iter().map(|r| r.0).collect::<Vec<_>>(),
        ["R1", "R2", "R3"]
    );
    // REPORT.md's table has every requirement, R2 covered in both rounds.
    let report = support::run_plans::report_with(&info, "| R3 | t2 | merged |");
    let rows = "| R1 | t1 | merged |\n| R2 | t1, t2 | merged |\n| R3 | t2 | merged |\n";
    assert!(report.contains(rows), "{report}");

    // The round's documents commit is its first new stage, stage 2, on round 1's head:
    // the spec with the amendment appended, and the round's plan as `…-round2.md`.
    let log = support::run_pr::log_lines(&h, &run);
    let stage2 = format!("anthrex/{run}/stage-2");
    let committed = (log.iter())
        .find(|l| l.starts_with("committed round 2 spec amendment and plan as "))
        .unwrap_or_else(|| panic!("{log:#?}"));
    assert!(committed.ends_with(&format!(" on {stage2}")), "{committed}");
    let sha7 = committed.split(' ').nth(8).unwrap();
    let commit = h.git(&["rev-parse", sha7]);
    let subject = h.git(&["log", "-1", "--format=%s", &commit]);
    assert_eq!(
        subject,
        format!("docs: round 2 spec amendment and plan for {GOAL}")
    );
    assert_eq!(h.git(&["rev-parse", &format!("{commit}^")]), round1_head);
    let spec_path = doc_path(&h, &run, "specs", "");
    let plan_path = doc_path(&h, &run, "plans", "-round2");
    let files = h.git(&["diff-tree", "--no-commit-id", "--name-only", "-r", &commit]);
    let mut expected = vec![plan_path.as_str(), spec_path.as_str()];
    expected.sort_unstable();
    assert_eq!(files.lines().collect::<Vec<_>>(), expected);
    let blob =
        |rev: &str, path: &str| git_text(&h.repo, &["cat-file", "blob", &format!("{rev}:{path}")]);
    let spec = format!(
        "{}\n## Round 2 amendment\n\n{}",
        stored(&h, &run, "spec-v1.md"),
        stored(&h, &run, "spec-v2.md")
    );
    assert_eq!(blob(&commit, &spec_path), spec);
    assert_eq!(blob(&commit, &plan_path), stored(&h, &run, "plan-v2.md"));

    // The accept lands both rounds' documents and work on main.
    h.accept(&run);
    assert_eq!(blob("main", &spec_path), spec);
    assert_eq!(blob("main", &plan_path), stored(&h, &run, "plan-v2.md"));
    assert_eq!(h.git(&["show", "main:b.txt"]), "t2");
}
