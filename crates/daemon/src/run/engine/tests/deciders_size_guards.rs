//! M8b.13 review fixes: the size cross-check's guards. An L answer blocks only a task
//! still waiting to run (review I1); a stale answer is ignored and a dispatched task
//! is never re-checked (I2); requests hold at most 50 tasks (m2); the deciders off
//! write no history line (m4).

use proto::{BlockReason, PlanEdit, Size, TaskState};

use super::control::retry;
use super::deciders_size::{done_info, evidenced, prepared, size_check_op, verdicts};
use super::dispatch::{edit, replies};
use super::fixture::*;
use super::liveness::assert_alive;
use crate::run::engine::OpResult;
use crate::run::model::SizeCheckState;

fn amend_brief(task_id: &str, brief: &str) -> PlanEdit {
    PlanEdit::AmendTask {
        task_id: task_id.into(),
        brief: Some(brief.into()),
        acceptance: None,
        route: None,
        test_mode: None,
        test_mode_reason: None,
        priority: None,
        size: None,
        deps: None,
    }
}

/// Review I1, scenario A: a task blocked `dep_cancelled` while its check is in flight
/// keeps that block when the check answers L, so `run retry` still refuses it.
#[test]
fn an_l_answer_keeps_a_dep_cancelled_block_and_retry_stays_refused() {
    let tasks = [
        task("t1", "S", "a", ""),
        task("t2", "S", "b", "deps = [\"t1\"]"),
    ];
    let mut fx = evidenced(&tasks, true, Default::default());
    let (op, task_ids, _) = size_check_op(&fx);
    assert_eq!(task_ids, vec!["t1".to_string(), "t2".to_string()]);
    let effects = edit(
        &mut fx,
        vec![PlanEdit::CancelTask {
            task_id: "t1".into(),
        }],
    );
    assert!(replies(&effects)[0].is_ok(), "{effects:#?}");
    let block = fx.task("t2").block.clone().expect("blocked");
    assert_eq!(block.reason, BlockReason::DepCancelled);
    assert!(matches!(
        fx.task("t2").size_check,
        Some(SizeCheckState::Pending { .. })
    ));

    fx.decided(
        op,
        verdicts(&[("t1", Size::S, "one"), ("t2", Size::L, "three modules")]),
    );
    let t2 = fx.task("t2");
    assert_eq!(t2.state, TaskState::Blocked);
    assert_eq!(t2.block, Some(block), "the existing block is kept");
    assert_eq!(
        (t2.size, t2.raised_size),
        (Size::S, None),
        "an L answer never raises"
    );
    assert_eq!(
        done_info(&fx, "t2").decided,
        Some(Size::L),
        "still recorded"
    );

    let effects = retry(&mut fx, "t2");
    let reply = replies(&effects)[0].clone();
    assert!(
        reply
            .as_ref()
            .is_err_and(|e| e.contains("retry cannot bring back a cancelled dependency")),
        "{reply:?}"
    );
    assert_eq!(fx.task("t2").state, TaskState::Blocked);
}

/// Review I1, scenario B: a task pre-warmed at the gate whose setup failed keeps its
/// `environment` block, with the setup output, when its check answers L.
#[test]
fn an_l_answer_keeps_an_environment_block_and_its_setup_output() {
    let mut fx = evidenced(&[task("t1", "S", "a", "")], false, Default::default());
    let (op, _) = fx.op("PrepareWorktree");
    fx.done(
        op,
        OpResult::SetupFailed {
            output: "boom".into(),
        },
    );
    let block = fx.task("t1").block.clone().expect("blocked");
    assert_eq!(block.reason, BlockReason::Environment);
    assert!(block.text.contains("boom"), "{}", block.text);

    fx.approve();
    let (op, ..) = size_check_op(&fx);
    fx.decided(op, verdicts(&[("t1", Size::L, "too big")]));
    let t1 = fx.task("t1");
    assert_eq!(t1.state, TaskState::Blocked);
    assert_eq!(t1.block, Some(block), "the setup output is kept");
    assert_eq!(
        (t1.size, t1.raised_size),
        (Size::S, None),
        "an L answer never raises"
    );
    assert_eq!(done_info(&fx, "t1").decided, Some(Size::L));
}

/// Review I2: an answer to a check an amend superseded is ignored; only the check the
/// task waits for now applies.
#[test]
fn a_stale_answer_after_an_amend_is_ignored() {
    let mut fx = evidenced(&[task("t1", "S", "a", "")], true, Default::default());
    let (stale, ..) = size_check_op(&fx);
    let effects = edit(&mut fx, vec![amend_brief("t1", "A new brief")]);
    assert!(replies(&effects)[0].is_ok(), "{effects:#?}");
    let current = match fx.task("t1").size_check {
        Some(SizeCheckState::Pending { decider_id }) => decider_id,
        ref other => panic!("{other:?}"),
    };

    let effects = fx.decided(stale, verdicts(&[("t1", Size::L, "old brief")]));
    let t1 = fx.task("t1");
    assert_eq!(t1.state, TaskState::Queued);
    assert_eq!(t1.block, None);
    assert_eq!(
        t1.size_check,
        Some(SizeCheckState::Pending {
            decider_id: current
        })
    );
    assert!(prepared(&effects).is_empty());
    assert_alive(&fx);

    let (op, task_ids, input) = size_check_op(&fx);
    assert_eq!(task_ids, vec!["t1".to_string()]);
    assert_eq!(input.tasks[0].brief, "A new brief");
    let effects = fx.decided(op, verdicts(&[("t1", Size::S, "one file")]));
    assert_eq!(prepared(&effects), vec!["t1"]);
}

/// Review I2: amending a working task's brief queues no check, so no answer can block
/// or re-route a live worker.
#[test]
fn amending_a_working_task_queues_no_size_check() {
    let mut fx = evidenced(&[task("t1", "S", "a", "")], true, Default::default());
    let (op, ..) = size_check_op(&fx);
    fx.decided(op, verdicts(&[("t1", Size::S, "one file")]));
    fx.launch_all();
    assert_eq!(fx.task("t1").state, TaskState::Working);
    let before = fx.task("t1").size_check.clone();
    let decides = fx.ops("Decide").len();

    let effects = edit(&mut fx, vec![amend_brief("t1", "A new brief")]);
    assert!(replies(&effects)[0].is_ok(), "{effects:#?}");
    assert!(ops_in(&effects, "Decide").is_empty(), "{effects:#?}");
    assert_eq!(fx.ops("Decide").len(), decides);
    assert!(fx.queued_deciders().is_empty());
    assert_eq!(fx.task("t1").size_check, before);
    assert_eq!(fx.task("t1").state, TaskState::Working);
}

/// Review I1 ruling: amending a blocked task queues no check either; `run retry` is
/// the user's override.
#[test]
fn amending_a_blocked_task_queues_no_size_check() {
    let mut fx = evidenced(&[task("t1", "S", "a", "")], true, Default::default());
    let (op, ..) = size_check_op(&fx);
    fx.decided(op, verdicts(&[("t1", Size::L, "three modules")]));
    assert_eq!(fx.task("t1").state, TaskState::Blocked);
    let decides = fx.ops("Decide").len();
    let effects = edit(&mut fx, vec![amend_brief("t1", "A smaller brief")]);
    assert!(replies(&effects)[0].is_ok(), "{effects:#?}");
    assert_eq!(fx.ops("Decide").len(), decides);
    assert!(fx.queued_deciders().is_empty());
    assert_eq!(done_info(&fx, "t1").decided, Some(Size::L));
}

/// Review m2: the schema's `maxItems` is 50, so 51 evidenced tasks make two requests.
#[test]
fn a_size_check_is_split_into_batches_of_fifty() {
    let tasks: Vec<String> = (1..=51)
        .map(|n| task(&format!("t{n}"), "S", &format!("m{n}"), ""))
        .collect();
    let config = config::Orchestrator {
        max_tasks: 51,
        max_readers: 4,
        ..Default::default()
    };
    let fx = evidenced(&tasks, true, config);
    let sizes: Vec<usize> = fx
        .ops("Decide")
        .into_iter()
        .map(|(_, kind)| match kind {
            crate::run::engine::OpKind::Decide {
                task_ids,
                request: crate::decider::DeciderRequest::SizeCheck(input),
                ..
            } => {
                assert_eq!(task_ids.len(), input.tasks.len());
                task_ids.len()
            }
            other => panic!("{other:?}"),
        })
        .collect();
    assert_eq!(sizes, vec![50, 1]);
    assert_eq!(fx.run().tasks.len(), 51);
}

/// Review m4: with the deciders off an evidenced task records the fallback with no
/// history line and no note.
#[test]
fn deciders_off_write_no_size_check_history() {
    let mut config = config::Orchestrator::default();
    config.deciders.mode = proto::DeciderMode::Off;
    let fx = evidenced(&[task("t1", "S", "a", "")], true, config);
    assert!(matches!(
        fx.task("t1").size_check,
        Some(SizeCheckState::Done(_))
    ));
    let t1 = fx.task("t1");
    assert!(
        t1.history.iter().all(|e| !e.text.contains("cross-check")),
        "{:?}",
        t1.history
    );
    assert!(t1.notes.iter().all(|n| !n.contains("cross-check")));
}
