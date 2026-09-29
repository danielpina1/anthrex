//! The large path (decisions 21, 22, 27, 31–33 and 37): two sub-planners submit their
//! epics, a planner's edit outside its area is refused and then fixed, and a new epic
//! of a running run waits for the user's approval (decision 28).

use proto::{HoldState, PlannerState, RunPath, RunState, TaskState};
use serde_json::{Value, json};

use crate::common::*;
use crate::epics::*;
use crate::support::orch_script::*;
use crate::support::run_harness::{REQUEST_WAIT, RUN_WAIT};
use crate::support::run_orch::ORCH_WAIT;
use crate::support::run_plans::{RETIRE_AFTER, approve, commit, done};

/// A harness whose triage answers `large`.
fn large_harness() -> crate::support::run_harness::RunHarness {
    harness_with(triage(&["code"], "large"))
}

#[test]
fn e2e_large_path_two_subplanners_submit_their_epics() {
    let h = large_harness();
    planner(
        &h,
        "a",
        &[submit_epic(vec![epic_task("a1", "src/a/one.rs", &["t0"])])],
    );
    planner(
        &h,
        "b",
        &[submit_epic(vec![epic_task("b1", "src/b/one.rs", &["t0"])])],
    );
    green_with_integration(
        &h,
        &[
            ("t0", "src/api.rs"),
            ("a1", "src/a/one.rs"),
            ("b1", "src/b/one.rs"),
        ],
        &["a", "b"],
    );
    let mut steps = vec![
        prompt(),
        call("get_context", json!({})),
        // The interface task both epics build on.
        edit_plan(
            vec![add(plan_task(
                "t0",
                &["src/api.rs"],
                json!({"interface_change": true}),
            ))],
            json!({}),
        ),
        subplanner("a", &["src/a/**"]),
        subplanner("b", &["src/b/**"]),
    ];
    steps.extend(until_planners_finished(2));
    steps.extend([
        edit_plan(vec![], json!({"submit": true})),
        expect("/awaiting_approval", json!(true)),
        until("/run/complete", json!(true), RUN_WAIT * 3),
        marker(),
        read(None),
    ]);
    let (run, _) = start(&h, &steps);
    approve_plan(&h, &run);
    let info = h.wait_run(&run, |r| r.state == RunState::Complete, RUN_WAIT * 3);
    wait_passed(&h, 1);

    assert_eq!(info.path, Some(RunPath::Large));
    for id in ["t0", "a1", "b1"] {
        assert_eq!(task(&info, id).state, TaskState::Merged, "{id}");
    }
    // The epics' tasks were the planners', each in its own epic, after `t0`.
    for (id, epic) in [("a1", "a"), ("b1", "b")] {
        let t = task(&info, id);
        assert_eq!(t.epic.as_deref(), Some(epic), "{id}");
        assert_eq!(t.deps, vec!["t0".to_string()], "{id}");
    }
    // Each epic's integration review ran and approved (decision 37).
    for id in ["a-int1", "b-int1"] {
        assert_eq!(task(&info, id).state, TaskState::Reported, "{id}");
    }
    assert_eq!(info.planners.len(), 2, "{:?}", info.planners);
    for p in &info.planners {
        assert_eq!(p.state, PlannerState::Finished, "{p:?}");
        assert_eq!(p.edits_accepted, 1, "{p:?}");
        assert_eq!(p.edits_rejected, 0, "{p:?}");
    }
    // Both planner windows are removed once retired.
    let ids: Vec<u32> = info.planners.iter().filter_map(|p| p.window_id).collect();
    assert_eq!(ids.len(), 2, "{:?}", info.planners);
    h.wait_windows(
        "both planner windows are removed",
        |ws| ws.iter().all(|w| !ids.contains(&w.id)),
        RETIRE_AFTER + REQUEST_WAIT,
    );
}

#[test]
fn e2e_subplanner_edit_outside_its_area_is_rejected_then_fixed() {
    let h = large_harness();
    planner(
        &h,
        "a",
        &[
            submit_epic_err(vec![epic_task("a1", "src/b/x.rs", &[])]),
            expect_error("src/b/x.rs is outside the area src/a/**"),
            submit_epic(vec![epic_task("a1", "src/a/one.rs", &[])]),
        ],
    );
    let mut steps = vec![prompt(), subplanner("a", &["src/a/**"])];
    steps.extend(until_planners_finished(1));
    steps.extend([
        expect("/planners/0/rejected", json!(1)),
        expect("/planners/0/tasks", json!(1)),
        marker(),
        read(None),
    ]);
    let (run, _) = start(&h, &steps);
    let log = wait_passed(&h, 1);

    // The refusal the planner read names the file and the area.
    let refused: Vec<&Value> = log
        .iter()
        .filter(|l| l["script"] == "planner-a-1" && l["tool"] == "submit_epic")
        .collect();
    assert_eq!(refused.len(), 2, "{refused:?}");
    assert_eq!(refused[0]["ok"], false, "{}", refused[0]);
    assert_eq!(refused[1]["ok"], true, "{}", refused[1]);
    // The orchestrator's digest showed the rejection (decision 33).
    let digest: Value = log
        .iter()
        .rfind(|l| l["script"] == ORCH && l["tool"] == "run_status")
        .and_then(|l| serde_json::from_str(l["result"].as_str()?).ok())
        .expect("a digest");
    let last = digest["planners"][0]["last_rejection"].as_str().unwrap();
    assert!(last.contains("src/b/x.rs"), "{digest}");
    let info = h.run(&run).unwrap();
    let p = &info.planners[0];
    assert_eq!((p.edits_accepted, p.edits_rejected), (1, 1), "{p:?}");
    assert!(
        p.last_rejection
            .as_deref()
            .is_some_and(|r| r.contains("outside the area")),
        "{p:?}"
    );
    assert_eq!(task(&info, "a1").owns, vec!["src/a/one.rs".to_string()]);
    // The rejected batch is in the edit log with the planner's source (decision 40).
    let rejected = info
        .plan_edits
        .iter()
        .find(|e| !e.accepted)
        .expect("the rejected batch is logged");
    assert_eq!(rejected.source, "planner:a");
}

#[test]
fn e2e_new_epic_after_approval_is_held_until_approved() {
    let h = large_harness();
    let go = h.dir.path().join("go");
    planner(
        &h,
        "a",
        &[submit_epic(vec![epic_task("a1", "src/a/one.rs", &[])])],
    );
    planner(
        &h,
        "c",
        &[submit_epic(vec![epic_task("c1", "src/c/one.rs", &[])])],
    );
    h.script(
        "worker-a1-1",
        &[
            wait_file(&go),
            commit("src/a/one.rs", "a1\n"),
            done("added one.rs"),
        ],
    );
    h.script("reviewer-a1-1", &[approve()]);
    green_with_integration(&h, &[("c1", "src/c/one.rs")], &["a", "c"]);
    let mut steps = vec![prompt(), subplanner("a", &["src/a/**"])];
    steps.extend(until_planners_finished(1));
    steps.extend([
        edit_plan(vec![], json!({"submit": true})),
        until("/gate/state", json!("approved"), ORCH_WAIT),
        subplanner("c", &["src/c/**"]),
        expect("/hold", json!("epic:c")),
        until("/planners/1/state", json!("finished"), ORCH_WAIT),
        until("/gate/holds/0/state", json!("approved"), RUN_WAIT * 2),
        expect("/gate/holds/0/id", json!("epic:c")),
        until("/run/complete", json!(true), RUN_WAIT * 3),
        marker(),
        read(None),
    ]);
    let (run, _) = start(&h, &steps);
    approve_plan(&h, &run);

    // Epic c's task waits under its hold; epic a's keeps running.
    let awaiting = |r: &proto::RunInfo| {
        r.holds
            .iter()
            .any(|h| h.id == "epic:c" && h.state == HoldState::Awaiting)
    };
    let info = h.wait_run(&run, awaiting, ORCH_WAIT);
    let c1 = task(&info, "c1");
    assert_eq!(c1.hold.as_deref(), Some("epic:c"));
    assert!(c1.rounds.is_empty(), "{:?}", c1.rounds);
    let info = wait_task(&h, &run, "a1", TaskState::Working);
    assert!(task(&info, "c1").rounds.is_empty());
    std::fs::write(&go, "").unwrap();
    let info = wait_task(&h, &run, "a1", TaskState::Merged);
    let c1 = task(&info, "c1");
    assert!(
        matches!(c1.state, TaskState::Pending | TaskState::Queued),
        "{c1:?}"
    );
    assert!(c1.rounds.is_empty(), "{:?}", c1.rounds);

    ok(&h.anthrex(&["run", "approve", &run, "--hold", "epic:c"]));
    let info = h.wait_run(&run, |r| r.state == RunState::Complete, RUN_WAIT * 3);
    assert_eq!(task(&info, "c1").state, TaskState::Merged);
    let hold = info.holds.iter().find(|h| h.id == "epic:c").unwrap();
    assert_eq!(hold.state, HoldState::Approved);
    assert_eq!(hold.decided_by.as_deref(), Some("user"));
    // The orchestrator's `run_status` saw the hold approved.
    wait_passed(&h, 1);
}
