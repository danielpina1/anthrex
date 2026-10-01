//! Steering by typing (decision 5), and the orchestrator's reactions to blocked work
//! after a wake-up (decisions 25, 39 and 40).

use proto::{RunState, TaskState};
use serde_json::json;

use crate::common::*;
use crate::support::orch_script::*;
use crate::support::run_harness::RUN_WAIT;
use crate::support::run_orch::{ORCH_WAIT, framed};
use crate::support::run_plans::{approve, commit, done, done_expecting_error};

#[test]
fn e2e_typed_steering_becomes_a_plan_edit() {
    let h = harness("");
    let go = h.dir.path().join("go");
    h.script(
        "worker-t1-1",
        &[wait_file(&go), commit("a.txt", "t1\n"), done("added a.txt")],
    );
    h.script("reviewer-t1-1", &[approve()]);
    let t2 = plan_task("t2", &["b.txt"], json!({"deps": ["t1"]}));
    let steps = [
        prompt(),
        edit_plan(
            vec![add(plan_task("t1", &["a.txt"], json!({}))), add(t2)],
            json!({"submit": true}),
        ),
        until("/gate/state", json!("approved"), ORCH_WAIT),
        read(Some("skip t2")),
        edit_plan(
            vec![json!({"op": "cancel_task", "task_id": "t2"})],
            json!({}),
        ),
        marker(),
        read(None),
    ];
    let (run, window) = start(&h, &steps);
    approve_plan(&h, &run);
    h.wait_orchestrator_idle(window);
    h.type_into(window, b"skip t2\r");
    let info = wait_task(&h, &run, "t2", TaskState::Cancelled);
    wait_passed(&h, 1);
    let entry = info
        .plan_edits
        .iter()
        .find(|e| e.text == "cancel t2")
        .unwrap_or_else(|| panic!("{:#?}", info.plan_edits));
    assert_eq!(entry.source, "orchestrator");
    assert!(entry.accepted);
    // What the orchestrator read was the typed line, and nothing else.
    let read = h.read_messages(ORCH);
    assert_eq!(read.len(), 1, "{read:?}");
    assert_eq!(read[0]["text"], "skip t2");

    std::fs::write(&go, "").unwrap();
    let info = h.wait_run(&run, |r| r.state == RunState::Complete, RUN_WAIT);
    assert_eq!(task(&info, "t1").state, TaskState::Merged);
}

#[test]
fn e2e_mis_sized_task_is_split_by_the_orchestrator() {
    let h = harness("");
    // Three failed done checks (no commit): rung 1, rung 2's fresh session, rung 3.
    h.script(
        "worker-t1-1",
        &[done_expecting_error(), done_expecting_error(), read(None)],
    );
    h.script("worker-t1-2", &[done_expecting_error(), read(None)]);
    green(&h, "t1a", "a.txt");
    green(&h, "t1b", "b.txt");
    let into = json!([
        plan_task("t1a", &["a.txt"], json!({})),
        plan_task("t1b", &["b.txt"], json!({})),
    ]);
    let steps = [
        prompt(),
        edit_plan(
            vec![add(plan_task("t1", &["a.txt", "b.txt"], json!({})))],
            json!({"submit": true}),
        ),
        until("/gate/state", json!("approved"), ORCH_WAIT),
        read(Some("t1 blocked (mis_sized)")),
        edit_plan(
            vec![json!({"op": "split_task", "task_id": "t1", "into": into})],
            json!({}),
        ),
        marker(),
        read(None),
    ];
    let (run, _) = start(&h, &steps);
    approve_plan(&h, &run);
    let info = h.wait_run(&run, |r| r.state == RunState::Complete, RUN_WAIT * 3);
    wait_passed(&h, 1);
    assert_eq!(task(&info, "t1").state, TaskState::Cancelled);
    for id in ["t1a", "t1b"] {
        assert_eq!(task(&info, id).state, TaskState::Merged, "{id}");
    }
    let json = h.run_json(&run);
    let t1 = json["tasks"]
        .as_array()
        .unwrap()
        .iter()
        .find(|t| t["spec"]["id"] == "t1")
        .unwrap();
    assert_eq!(t1["rung"], 3, "{t1}");
}

#[test]
fn e2e_blocked_question_is_answered_after_a_wake() {
    let h = harness("");
    let go = h.dir.path().join("go");
    // The worker blocks only once the orchestrator polls no more: a poll that answered
    // after the block would read its note away, and no wake-up would follow.
    h.script(
        "worker-t1-1",
        &[
            wait_file(&go),
            call(
                "task_blocked",
                json!({"kind": "question", "reason": "tabs or spaces?"}),
            ),
            read(Some("use tabs")),
            commit("a.txt", "\tt1\n"),
            done("indented with tabs"),
        ],
    );
    h.script("reviewer-t1-1", &[approve()]);
    let steps = [
        prompt(),
        edit_plan(
            vec![add(plan_task("t1", &["a.txt"], json!({})))],
            json!({"submit": true}),
        ),
        until("/gate/state", json!("approved"), ORCH_WAIT),
        read(Some("t1 blocked (question)")),
        edit_plan(
            vec![json!({"op": "answer", "task_id": "t1", "text": "use tabs"})],
            json!({}),
        ),
        marker(),
        read(None),
    ];
    let (run, _) = start(&h, &steps);
    approve_plan(&h, &run);
    wait_saw_approval(&h);
    std::fs::write(&go, "").unwrap();
    let info = h.wait_run(&run, |r| r.state == RunState::Complete, RUN_WAIT);
    wait_passed(&h, 1);
    assert_eq!(task(&info, "t1").state, TaskState::Merged);
    // The wake-up came as decision 39's paste, submitted with its own `\r`.
    let read = h.read_messages(ORCH);
    let wake = read
        .iter()
        .find(|m| {
            m["text"]
                .as_str()
                .unwrap()
                .contains("t1 blocked (question)")
        })
        .unwrap_or_else(|| panic!("{read:?}"));
    let text = wake["text"].as_str().unwrap();
    assert!(
        text.starts_with(&format!("[anthrex] Run {run} changed: ")),
        "{text}"
    );
    assert_eq!(wake["raw"].as_str().unwrap(), framed(text));
}
