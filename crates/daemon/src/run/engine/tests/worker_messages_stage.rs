//! Milestone 9.1 task M9.1.19: a `message` to `stage:<n>` (decision 56), which replaced
//! milestone 9's refusal of a stage recipient.

use proto::{MessageKind, MessageTarget, PlanEdit, TaskState};

use super::dispatch::{edit, replies};
use super::fixture::*;
use crate::run::engine::OpResult;

/// Milestone 9.1 decision 56: `stage:<n>` resolves at acceptance to every unfinished
/// task of stage `n`, each taking it by decision 42b's per-state rule.
#[test]
fn message_to_a_stage_reaches_its_unfinished_tasks() {
    let plan = plan_with(
        PROFILE,
        &[
            task("t1", "S", "a", ""),
            task("t2", "S", "b", "stage = 2"),
            task("t3", "S", "c", "stage = 2\ndeps = [\"t2\"]"),
            task("t4", "S", "d", "stage = 2"),
        ],
    );
    let mut fx = Fixture::new(&plan);
    fx.ready(true);
    while let Some((op, _)) = super::merge::pending(&fx, "CreateStageBranch", None)
        .first()
        .cloned()
    {
        fx.done(op, OpResult::StageCreated);
    }
    fx.launch_all();
    assert_eq!(fx.task("t2").state, TaskState::Working);
    assert_eq!(fx.task("t3").state, TaskState::Pending);
    fx.task_mut("t4").state = TaskState::Merged;
    let stage = |n: u32| PlanEdit::Message {
        to: MessageTarget::Stage(n),
        text: "the schema moved".into(),
        kind: MessageKind::Change,
    };
    let effects = edit(&mut fx, vec![stage(2)]);
    assert_eq!(
        replies(&effects),
        vec![Ok("applied 1 edit; message for t2, t3".to_string())]
    );
    // Working: queued for its next turn. Not started: recorded for its first prompt.
    for id in ["t2", "t3"] {
        let messages = &fx.task(id).orch.messages;
        assert_eq!(messages.len(), 1, "{id}");
        assert_eq!(messages[0].kind, MessageKind::Change);
        assert!(!messages[0].delivered);
    }
    assert!(fx.task("t4").orch.messages.is_empty(), "merged: nothing");
    assert!(fx.task("t1").orch.messages.is_empty(), "another stage");
    let entry = fx.run().plan_edits.last().unwrap();
    assert_eq!(entry.recipients, ["t2", "t3"]);

    assert_eq!(
        replies(&edit(&mut fx, vec![stage(9)])),
        vec![Err("stage 9 has no task".to_string())]
    );
    // A stage whose every task finished has no one to tell.
    for id in ["t2", "t3"] {
        fx.task_mut(id).state = TaskState::Merged;
    }
    assert_eq!(
        replies(&edit(&mut fx, vec![stage(2)])),
        vec![Err("stage 2 has no unfinished task".to_string())]
    );
}
