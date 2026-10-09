//! The plan path and its gate (decisions 16, 26, 30 and 38), and what the orchestrator
//! may never do (decisions 7 and 15).

use proto::{RunState, TaskState};
use serde_json::json;

use crate::common::*;
use crate::support::orch_script::*;
use crate::support::run_harness::{REQUEST_WAIT, RUN_WAIT};
use crate::support::run_orch::ORCH_WAIT;
use crate::support::run_plans::{git_read, report_with};

const SUMMARY: &str = "Both files were added, a.txt and b.txt.";

#[test]
fn e2e_plan_path_scouts_plans_and_reads_the_approval() {
    let h = harness("");
    h.script(
        "scout-core-1",
        &[call(
            "submit_scout_report",
            json!({"summary": "check.sh checks the repository.",
                "files": [{"path": "check.sh", "why": "the check"}]}),
        )],
    );
    green(&h, "t1", "a.txt");
    green(&h, "t2", "b.txt");
    let refs = json!({"scout_refs": ["{{scout}}"]});
    let steps = [
        prompt(),
        call("get_context", json!({})),
        call(
            "spawn_scout",
            json!({"id": "core", "question": "How is the repository checked?",
                "area": ["check.sh", "tests/**"]}),
        ),
        until("/scouts/0/state", json!("reported"), ORCH_WAIT),
        capture_json("scout", "/scouts/0/id"),
        edit_plan(
            vec![
                add(plan_task("t1", &["a.txt"], refs.clone())),
                add(plan_task("t2", &["b.txt"], refs)),
            ],
            json!({"submit": true}),
        ),
        expect("/awaiting_approval", json!(true)),
        until("/gate/state", json!("approved"), ORCH_WAIT),
        until("/run/complete", json!(true), RUN_WAIT * 2),
        edit_plan(vec![], json!({"summary": SUMMARY})),
        // A real long-poll: nothing changes a complete run's digest, so a `since` of
        // its revision waits out `wait_secs` (decision 16).
        call("run_status", json!({})),
        capture_json("rev", "/revision"),
        call("run_status", json!({"since": "{{#rev}}", "wait_secs": 3})),
        marker(),
        read(None),
    ];
    let (run, _) = start(&h, &steps);
    approve_plan(&h, &run);
    let info = h.wait_run(&run, |r| r.state == RunState::Complete, RUN_WAIT * 2);
    for id in ["t1", "t2"] {
        assert_eq!(task(&info, id).state, TaskState::Merged, "{id}");
    }
    wait_passed(&h, 1);

    // The scout's report reached the tasks the orchestrator planned from it.
    let scout = format!("{}-core", &run[run.len() - 4..]);
    let json = h.run_json(&run);
    for t in json["tasks"].as_array().unwrap() {
        assert_eq!(t["spec"]["scout_refs"], json!([scout]), "{t}");
    }
    // The summary opens the report (decision 38).
    let info = h.wait_run(
        &run,
        |r| r.orchestrator.as_ref().is_some_and(|o| o.summary.is_some()),
        REQUEST_WAIT,
    );
    let report = report_with(&info, SUMMARY);
    let first = report.lines().find(|l| l.starts_with("## "));
    assert_eq!(first, Some("## Summary from the orchestrator"), "{report}");
    // Every `run_status` answered within its `wait_secs` and 5 s more (decision 16).
    let log = h.mcp_log();
    let calls: Vec<_> = log
        .iter()
        .filter(|l| l["script"] == ORCH && l["tool"] == "run_status")
        .collect();
    assert!(calls.len() >= 4, "{calls:?}");
    let polled: Vec<_> = calls
        .iter()
        .filter(|c| c["args"]["since"].is_u64())
        .collect();
    assert_eq!(polled.len(), 1, "{polled:?}");
    let waited = polled[0]["ms"].as_u64().unwrap();
    assert!(waited >= 3_000, "the long-poll answered after {waited} ms");
    for c in calls {
        let wait = c["args"]["wait_secs"].as_u64().unwrap_or(0);
        let ms = c["ms"].as_u64().expect("the call's duration");
        assert!(ms <= (wait + 5) * 1_000, "{c}");
    }
}

#[test]
fn e2e_rejected_plan_discards_the_run_and_the_orchestrator_learns_it() {
    let h = harness("");
    let steps = [
        prompt(),
        edit_plan(
            vec![add(plan_task("t1", &["a.txt"], json!({})))],
            json!({"submit": true}),
        ),
        until("/run/state", json!("discarded"), ORCH_WAIT),
        call_err(
            "edit_plan",
            json!({"edits": [add(plan_task("t2", &["b.txt"], json!({})))]}),
        ),
        // Milestone 9.3 decision 21: the discarded run's orchestrator is idle.
        expect_error("has ended; start a new goal with start_goal"),
        marker(),
        read(None),
    ];
    let (run, window) = start(&h, &steps);
    h.wait_run(&run, |r| r.state == RunState::AwaitingApproval, ORCH_WAIT);
    ok(&h.anthrex(&["run", "reject", &run, "--confirm", &run]));
    h.wait_run(&run, |r| r.state == RunState::Discarded, REQUEST_WAIT);
    let log = wait_passed(&h, 1);
    let refused = log
        .iter()
        .rfind(|l| l["script"] == ORCH && l["tool"] == "edit_plan")
        .unwrap();
    assert_eq!(refused["ok"], false, "{refused}");
    assert!(
        refused["result"].as_str().unwrap().contains(&format!(
            "run {} has ended; start a new goal with start_goal when the user gives you one",
            &run[run.len() - 4..]
        )),
        "{refused}"
    );
    // Decision 30: the window is a plain one now; the user may remove it.
    ok(&h.anthrex(&["rm", &window.to_string()]));
    h.wait_windows(
        "the window is gone",
        |ws| ws.iter().all(|w| w.id != window),
        REQUEST_WAIT,
    );
}

#[test]
fn e2e_orchestrator_cannot_approve_merge_or_write() {
    let h = harness("");
    let steps = [
        prompt(),
        edit_plan(
            vec![add(plan_task("t1", &["a.txt"], json!({})))],
            json!({"submit": true}),
        ),
        call_err(
            "edit_plan",
            json!({"edits": [{"op": "override", "task_id": "t1", "reason": "trust me"}]}),
        ),
        // Milestone 9.9 (D2) made `override` an orchestrator op; it still cannot get
        // past the user's plan gate: the run is refused while it awaits approval.
        expect_error("is awaiting_approval"),
        call_err("task_done", json!({"summary": "merged it myself"})),
        expect_error("tool task_done is not available to the orchestrator role"),
        marker(),
        read(None),
    ];
    let (run, _) = start(&h, &steps);
    wait_passed(&h, 1);
    // Nothing was approved: the plan still waits for the user.
    let info = h.run(&run).unwrap();
    assert_eq!(info.state, RunState::AwaitingApproval);
    assert_eq!(task(&info, "t1").state, TaskState::Queued);
    // The Claude orchestrator runs with the writing tools disallowed (decision 7).
    let argv: Vec<String> = serde_json::from_str(&h.io_lines(ORCH, "args")[0]).unwrap();
    let at = argv.iter().position(|a| a == "--disallowedTools").unwrap();
    assert!(
        argv[at + 1].starts_with("Edit,Write,NotebookEdit,Bash,Agent"),
        "{argv:?}"
    );
    // The run branch has no commit past its base.
    let branch = format!("refs/heads/{}", info.run_branch);
    let head = crate::support::run_plans::until("the run branch", REQUEST_WAIT, || {
        git_read(&h.repo, &["rev-parse", "--verify", "-q", &branch])
    });
    assert_eq!(head, info.base_sha);
}

/// Milestone 9.8 decision 31: the orchestrator sizes; it never routes. A route it sends
/// with `add_task` is ignored: the worker runs on its size's row (the harness's `S` row
/// is the built-in Claude Sonnet), and the run log says so.
#[test]
fn an_orchestrator_route_is_ignored_and_the_run_says_so() {
    let h = harness("");
    green(&h, "t1", "a.txt");
    let route = json!({"route": {"runtime": "codex", "model": "gpt-6-sol"}});
    let steps = [
        prompt(),
        edit_plan(
            vec![add(plan_task("t1", &["a.txt"], route))],
            json!({"submit": true}),
        ),
        expect("/awaiting_approval", json!(true)),
        marker(),
        read(None),
    ];
    let (run, _) = start(&h, &steps);
    wait_passed(&h, 1);
    approve_plan(&h, &run);
    wait_task(&h, &run, "t1", TaskState::Merged);
    let argv: Vec<String> = serde_json::from_str(&h.io_lines("worker-t1-1", "args")[0]).unwrap();
    let at = argv.iter().position(|a| a == "--model").expect("--model");
    assert_eq!(argv[at + 1], "claude-sonnet-5", "{argv:?}");
    let log = crate::support::run_pr::log_lines(&h, &run);
    assert!(
        log.iter()
            .any(|l| l == "route model ignored: models come from the role table"),
        "{log:#?}"
    );
}
