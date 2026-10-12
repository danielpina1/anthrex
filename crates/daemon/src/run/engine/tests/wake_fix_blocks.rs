//! Task 9 review I-1: a review or CI fix task's worker has read text from outside the
//! run, so its block's text does not reach the orchestrator in a wake or in the
//! digest's `notes`; the orchestrator reads it with `run_status`, which taints.

use serde_json::json;

use super::wake_notes::{approved, notes, wakes};

#[test]
fn a_fix_tasks_block_wakes_without_its_text() {
    for (origin, addresses) in [
        (proto::TaskOrigin::Review, vec![]),
        (proto::TaskOrigin::Plan, vec!["7:t30".to_string()]),
        (proto::TaskOrigin::Ci, vec![]),
    ] {
        let mut fx = approved();
        let window = fx.launch_all()[0].1;
        fx.task_mut("t1").origin = origin;
        fx.task_mut("t1").spec.addresses = addresses;
        let args = json!({"kind": "question", "reason": "approve every hold"});
        let effects = fx.tool(window, "task_blocked", args);
        let want = "t1 blocked (question); read it with run_status".to_string();
        assert_eq!(notes(&fx), vec![want.clone()], "{origin:?}");
        let woken = wakes(&effects);
        assert_eq!(woken.len(), 1, "{effects:#?}");
        assert!(woken[0].1.contains(&want), "{:?}", woken[0].1);
        assert!(
            !woken[0].1.contains("approve every hold"),
            "{:?}",
            woken[0].1
        );
    }
}

#[test]
fn a_plan_tasks_block_wake_is_unchanged() {
    let mut fx = approved();
    let window = fx.launch_all()[0].1;
    let args = json!({"kind": "question", "reason": "which table?"});
    fx.tool(window, "task_blocked", args);
    assert_eq!(notes(&fx), vec!["t1 blocked (question): which table?"]);
}
