//! Milestone 8c task 1: the plan-edit log's text and its cap.

use proto::{PlanEdit, PlanTask};

use super::*;
use crate::run::orch::EditSource;
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
            deps: None,
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
        PlanEdit::Message {
            to: proto::MessageTarget::Tasks(vec!["t6".into(), "t7".into()]),
            text: "the schema moved".into(),
            kind: proto::MessageKind::Change,
        },
        PlanEdit::Refresh {
            task_id: "t8".into(),
        },
    ];
    assert_eq!(
        describe(&edits),
        "add t9, split t2, cancel t3, amend t4, dep t4 on t2, answer t5, pause, resume, \
         finish, message t6,t7 (change), refresh t8"
    );
    assert_eq!(describe(&[PlanEdit::Pause]), "pause");
}

#[test]
fn record_keeps_the_last_fifty() {
    let mut run = run_ok(EXAMPLE_PLAN);
    for n in 0..(PLAN_EDITS_KEPT as u64 + 7) {
        record(
            &mut run,
            &[PlanEdit::Pause],
            1_000 + n,
            &EditSource::User,
            EditOutcome::accepted(),
        );
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
    record(
        &mut run,
        &[PlanEdit::Resume],
        2_001,
        &EditSource::User,
        EditOutcome::accepted(),
    );
    assert_eq!(run.plan_edits_since_approval, 1);
    assert_eq!(run.plan_edits.len(), PLAN_EDITS_KEPT);
    assert_eq!(
        run.plan_edits.last(),
        Some(&PlanEditRecord {
            at: 2_001,
            text: "resume".into(),
            source: "user".into(),
            accepted: true,
            error: None,
            recipients: Vec::new(),
        })
    );
}

/// Review M3: a huge batch cannot make one record (and so every snapshot) huge.
#[test]
fn describe_is_capped() {
    let edits: Vec<PlanEdit> = (0..20_000)
        .map(|_| PlanEdit::AmendTask {
            task_id: "t1".into(),
            brief: None,
            acceptance: None,
            route: None,
            test_mode: None,
            test_mode_reason: None,
            priority: Some(1),
            size: None,
            deps: None,
        })
        .collect();
    let text = describe(&edits);
    assert!(
        text.chars().count() <= DESCRIBE_MAX_CHARS + 1,
        "{}",
        text.len()
    );
    assert!(text.starts_with("amend t1, amend t1"), "{text}");
    assert!(text.ends_with('…'), "{text}");
    // A short batch is left whole.
    assert_eq!(describe(&[PlanEdit::Pause]), "pause");
    // Cut on a char boundary: multi-byte ids never split.
    let wide = vec![PlanEdit::CancelTask {
        task_id: "é".repeat(400),
    }];
    let text = describe(&wide);
    assert_eq!(text.chars().count(), DESCRIBE_MAX_CHARS + 1);
    assert!(text.ends_with('…'), "{text}");
    // Exactly at the cap: whole, no `…`. One past it: the first 300 plus `…`.
    let at_cap = format!("{}日", "x".repeat(DESCRIBE_MAX_CHARS - "cancel ".len() - 1));
    let exact = describe(&[PlanEdit::CancelTask {
        task_id: at_cap.clone(),
    }]);
    assert_eq!(exact, format!("cancel {at_cap}"));
    assert_eq!(exact.chars().count(), DESCRIBE_MAX_CHARS);
    let over = describe(&[PlanEdit::CancelTask {
        task_id: format!("{at_cap}日"),
    }]);
    assert_eq!(over, format!("cancel {at_cap}…"));
}

#[test]
fn describe_replaces_control_characters() {
    let edits = vec![PlanEdit::CancelTask {
        task_id: "t\n1\u{1b}[31m\r\t".into(),
    }];
    assert_eq!(describe(&edits), "cancel t 1 [31m  ");
}

/// Decision 40 (task M9.9): a record keeps its source, whether it was accepted, a
/// rejected batch's error and a message's recipients; only accepted batches count as
/// edits after approval.
#[test]
fn record_keeps_source_outcome_and_recipients() {
    let mut run = run_ok(EXAMPLE_PLAN);
    run.approved_at = Some(2_000);
    let planner = EditSource::Planner {
        epic: "mail".into(),
    };
    record(
        &mut run,
        &[PlanEdit::Pause],
        2_001,
        &planner,
        EditOutcome::Rejected {
            error: "task t9: epic: nope".into(),
        },
    );
    record(
        &mut run,
        &[PlanEdit::Resume],
        2_002,
        &EditSource::Orchestrator,
        EditOutcome::Accepted {
            recipients: vec!["t1".into(), "t2".into()],
        },
    );
    assert_eq!(
        run.plan_edits,
        vec![
            PlanEditRecord {
                at: 2_001,
                text: "pause".into(),
                source: "planner:mail".into(),
                accepted: false,
                error: Some("task t9: epic: nope".into()),
                recipients: Vec::new(),
            },
            PlanEditRecord {
                at: 2_002,
                text: "resume".into(),
                source: "orchestrator".into(),
                accepted: true,
                error: None,
                recipients: vec!["t1".into(), "t2".into()],
            },
        ]
    );
    assert_eq!(run.plan_edits_since_approval, 1);
}
