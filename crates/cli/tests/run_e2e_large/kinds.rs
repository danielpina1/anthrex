//! The per-epic integration review (decision 37) and the research and review kinds
//! (decisions 24, 35 and 36): what they report, and that nothing of theirs merges.

use proto::{IntegrationState, RunState, TaskState, Verdict};
use serde_json::{Value, json};

use crate::common::*;
use crate::epics::*;
use crate::support::orch_script::*;
use crate::support::run_harness::{RUN_WAIT, RunHarness, git_in};
use crate::support::run_orch::ORCH_WAIT;
use crate::support::run_plans::{approve, changes, finding, report_with};

#[test]
fn e2e_integration_review_asks_for_changes_and_a_fix_task_closes_it() {
    let h = harness_with(triage(&["code"], "large"));
    planner(
        &h,
        "a",
        &[submit_epic(vec![epic_task("a1", "src/a/one.rs", &[])])],
    );
    green(&h, "a1", "src/a/one.rs");
    green(&h, "a9", "src/a/fix.rs");
    h.script(
        "reviewer-a-int1-1",
        &[changes(&[finding(
            "critical",
            "src/a/one.rs",
            1,
            "one.rs is never called",
        )])],
    );
    h.script("reviewer-a-int2-1", &[approve()]);
    let mut steps = vec![prompt(), subplanner("a", &["src/a/**"])];
    steps.extend(until_planners_finished(1));
    steps.extend([
        edit_plan(vec![], json!({"submit": true})),
        until("/gate/state", json!("approved"), ORCH_WAIT),
        // The wake-up of decision 39, pasted once the verdict is in.
        read(Some(
            "integration review of epic a: changes (1 critical, 0 important)",
        )),
        edit_plan(
            vec![add(plan_task(
                "a9",
                &["src/a/fix.rs"],
                json!({"epic": "a"}),
            ))],
            json!({}),
        ),
        until("/run/complete", json!(true), RUN_WAIT * 3),
        marker(),
        read(None),
    ]);
    let (run, _) = start(&h, &steps);
    approve_plan(&h, &run);

    // The first round asks for changes: the epic holds the run's completion.
    let changed = |r: &proto::RunInfo| {
        r.integration
            .iter()
            .any(|i| i.epic == "a" && i.state == IntegrationState::Changes)
            || r.tasks.iter().any(|t| t.id == "a9")
    };
    let info = h.wait_run(&run, changed, RUN_WAIT * 2);
    let int1 = task(&info, "a-int1");
    assert_eq!(int1.state, TaskState::Reported);
    assert_eq!(int1.reviews[0].verdict, Some(Verdict::Changes));
    assert_ne!(info.state, RunState::Complete);

    let info = h.wait_run(&run, |r| r.state == RunState::Complete, RUN_WAIT * 3);
    wait_passed(&h, 1);
    let a9 = task(&info, "a9");
    assert_eq!(a9.state, TaskState::Merged);
    assert_eq!(a9.epic.as_deref(), Some("a"));
    // The fix task's merge started round 2, which approved the epic.
    let int2 = task(&info, "a-int2");
    assert_eq!(int2.state, TaskState::Reported);
    assert_eq!(int2.reviews[0].verdict, Some(Verdict::Approve));
    let integration = info.integration.iter().find(|i| i.epic == "a").unwrap();
    assert_eq!(integration.state, IntegrationState::Approved);
    assert_eq!(integration.tasks, vec!["a-int1", "a-int2"]);
}

/// A research or review task (`kind`) with no `owns` (decision 24), and `more`.
fn reader_task(id: &str, kind: &str, more: Value) -> Value {
    let mut task = json!({
        "id": id, "title": format!("Task {id}"), "brief": format!("Do {id}."),
        "acceptance": [format!("{id} is done")], "owns": [], "size": "S", "kind": kind,
    });
    for (key, value) in more.as_object().cloned().unwrap_or_default() {
        task[key] = value;
    }
    task
}

/// The orchestrator's plan of one `task`, submitted; it waits for the run to complete.
fn one_task_plan(task: Value) -> Vec<Value> {
    vec![
        prompt(),
        edit_plan(vec![add(task)], json!({"submit": true})),
        until("/run/complete", json!(true), RUN_WAIT),
        marker(),
        read(None),
    ]
}

/// `run accept` answered `n`, then `run accept --yes`; the first's stderr.
fn accept_both_ways(h: &RunHarness, run: &str) -> String {
    let out = h.anthrex_input(&["run", "accept", run], "n\n");
    assert!(!out.status.success(), "{}", stdout(&out));
    let asked = stderr(&out);
    assert_eq!(h.run(run).unwrap().state, RunState::Complete);
    ok(&h.anthrex(&["run", "accept", run, "--yes"]));
    assert_eq!(h.run(run).unwrap().state, RunState::Accepted);
    asked
}

#[test]
fn e2e_research_goal_reports_without_merging() {
    let h = harness("");
    h.decider("triage", 1, triage(&["research"], "plan"));
    h.script(
        "scout-t1-1",
        &[call(
            "submit_scout_report",
            json!({"summary": "check.sh is the whole check.",
                "files": [{"path": "check.sh", "why": "the check"}]}),
        )],
    );
    let (run, _) = start(&h, &one_task_plan(reader_task("t1", "research", json!({}))));
    approve_plan(&h, &run);
    let info = h.wait_run(&run, |r| r.state == RunState::Complete, RUN_WAIT);
    wait_passed(&h, 1);

    let t1 = task(&info, "t1");
    assert_eq!(t1.state, TaskState::Reported);
    assert!(t1.rounds.iter().all(|r| r.role == proto::AgentRole::Scout));
    assert_eq!(info.run_head, info.base_sha, "nothing merged");
    let report = info.research_report.clone().expect("a research report");
    let text = std::fs::read_to_string(&report).unwrap();
    assert!(text.contains("check.sh is the whole check."), "{text}");
    report_with(&info, "## Research");

    let asked = accept_both_ways(&h, &run);
    assert!(
        asked.starts_with(&format!("research report: {}\n", report.display())),
        "{asked}"
    );
    assert!(
        asked.contains(&format!(
            "nothing to merge; accept run {run} and remove its branches? [y/N] "
        )),
        "{asked}"
    );
    assert_eq!(git_in(&h.repo, &["rev-parse", "main"]), info.base_sha);
}

#[test]
fn e2e_review_goal_reviews_a_range_without_merging() {
    let h = harness("");
    h.decider("triage", 1, triage(&["review"], "plan"));
    let base = git_in(&h.repo, &["rev-parse", "main"]);
    git_in(&h.repo, &["checkout", "-q", "-b", "feature"]);
    for name in ["f1.txt", "f2.txt"] {
        std::fs::write(h.repo.join(name), format!("{name}\n")).unwrap();
        git_in(&h.repo, &["add", name]);
        git_in(&h.repo, &["commit", "-q", "-m", &format!("add {name}")]);
    }
    let head = git_in(&h.repo, &["rev-parse", "feature"]);
    git_in(&h.repo, &["checkout", "-q", "main"]);
    let text = "f2.txt repeats f1.txt";
    h.script(
        "reviewer-r1-1",
        &[changes(&[finding("important", "f2.txt", 1, text)])],
    );
    let plan = one_task_plan(reader_task(
        "r1",
        "review",
        json!({"review_target": "main..feature"}),
    ));
    let (run, _) = start(&h, &plan);
    approve_plan(&h, &run);
    let info = h.wait_run(&run, |r| r.state == RunState::Complete, RUN_WAIT);
    wait_passed(&h, 1);

    // The reviewer was shown the range by both its shas (decision 36).
    let prompt = h.io_lines("reviewer-r1-1", "stdin").join("\n");
    assert!(
        prompt.contains(&format!("Base: {}", &base[..7])),
        "{prompt}"
    );
    assert!(
        prompt.contains(&format!("Head: {}", &head[..7])),
        "{prompt}"
    );
    assert!(prompt.contains("of main..feature"), "{prompt}");
    // A `changes` verdict still ends the task `reported`; nothing is sent back.
    let r1 = task(&info, "r1");
    assert_eq!(r1.state, TaskState::Reported);
    assert_eq!(r1.reviews.len(), 1, "{:?}", r1.reviews);
    assert_eq!(r1.reviews[0].verdict, Some(Verdict::Changes));
    let report = report_with(&info, text);
    assert!(report.contains("## Review findings"), "{report}");
    assert_eq!(info.run_head, info.base_sha, "nothing merged");

    accept_both_ways(&h, &run);
    assert_eq!(git_in(&h.repo, &["rev-parse", "main"]), base);
    assert_eq!(git_in(&h.repo, &["rev-parse", "feature"]), head);
}
