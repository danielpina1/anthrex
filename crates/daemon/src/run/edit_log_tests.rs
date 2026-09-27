//! Milestone 8c task 1: the plan-edit log's text and its cap.

use proto::{PlanEdit, PlanTask};

use super::*;
use crate::run::test_support::{EXAMPLE_PLAN, run_ok};

fn a_task(id: &str) -> PlanTask {
    let mut task = run_ok(EXAMPLE_PLAN).tasks[0].spec.clone();
    task.id = id.to_string();
    task
}

#[test]
fn describe_names_every_edit_op() {
    let edits = vec![
        PlanEdit::AddTask { task: a_task("t9") },
        PlanEdit::SplitTask {
            task_id: "t2".into(),
            into: vec![a_task("t2a"), a_task("t2b")],
        },
        PlanEdit::CancelTask {
            task_id: "t3".into(),
        },
        PlanEdit::AmendTask {
            task_id: "t4".into(),
            brief: Some("new brief".into()),
            acceptance: None,
            route: None,
            test_mode: None,
            test_mode_reason: None,
            priority: None,
            size: None,
        },
        PlanEdit::AddDep {
            task_id: "t4".into(),
            dep: "t2".into(),
        },
        PlanEdit::Answer {
            task_id: "t5".into(),
            text: "use the v2 API".into(),
        },
        PlanEdit::Pause,
        PlanEdit::Resume,
        PlanEdit::Finish,
    ];
    assert_eq!(
        describe(&edits),
        "add t9, split t2, cancel t3, amend t4, dep t4 on t2, answer t5, pause, resume, finish"
    );
    assert_eq!(describe(&[PlanEdit::Pause]), "pause");
}

#[test]
fn record_keeps_the_last_fifty() {
    let mut run = run_ok(EXAMPLE_PLAN);
    for n in 0..(PLAN_EDITS_KEPT as u64 + 7) {
        record(&mut run, &[PlanEdit::Pause], 1_000 + n);
    }
    assert_eq!(run.plan_edits.len(), PLAN_EDITS_KEPT);
    assert_eq!(PLAN_EDITS_KEPT, 50);
    // The oldest seven went; the rest stay oldest first.
    assert_eq!(run.plan_edits[0].at, 1_007);
    assert_eq!(run.plan_edits.last().map(|r| r.at), Some(1_056));
    assert_eq!(run.plan_edits[0].text, "pause");
    // Not approved: nothing counts as an edit after approval.
    assert_eq!(run.plan_edits_since_approval, 0);

    run.approved_at = Some(2_000);
    record(&mut run, &[PlanEdit::Resume], 2_001);
    assert_eq!(run.plan_edits_since_approval, 1);
    assert_eq!(run.plan_edits.len(), PLAN_EDITS_KEPT);
    assert_eq!(
        run.plan_edits.last(),
        Some(&PlanEditRecord {
            at: 2_001,
            text: "resume".into()
        })
    );
}
