//! Milestone 9.1 decisions 43 and 44 on an edited run (task M9.1.11): `amend_task`'s
//! `stage`, allowed only before a task starts and refused as a whole otherwise, and the
//! stage rules re-run over every task after every batch. Every test goes through
//! `apply_edits`.

use proto::{BlockReason, PlanEdit, TaskState};

use super::*;

/// An `amend_task` of `stage`, with a `priority` when given.
fn amend_stage(task_id: &str, stage: u16, priority: Option<i32>) -> PlanEdit {
    PlanEdit::AmendTask {
        task_id: task_id.to_string(),
        brief: None,
        acceptance: None,
        route: None,
        test_mode: None,
        test_mode_reason: None,
        priority,
        size: None,
        deps: None,
        stage: Some(stage),
    }
}

fn add(table: &str) -> PlanEdit {
    PlanEdit::AddTask { task: spec(table) }
}

fn stage_of(run: &Run, id: &str) -> u16 {
    run.tasks.iter().find(|t| t.id() == id).unwrap().spec.stage
}

#[test]
fn amend_stage_only_before_start() {
    // t1; t2 depends on t1; t3 depends on t2; t4 stands alone.
    let mut working = chain();
    set_state(&mut working, "t4", TaskState::Working, None);
    let refusal =
        "task t4 is working; stage can be amended only on pending, queued or blocked tasks";
    assert_eq!(
        rejected(&working, vec![amend_stage("t4", 2, None)]),
        [refusal]
    );
    // Review of M9.1.3: a stage with another field is refused as a whole, so the
    // priority it carries is not applied (ruling C-14 (f): the batch itself is
    // rejected), while the priority alone is.
    let combined = apply(&working, vec![amend_stage("t4", 2, Some(5))]);
    let errors: Vec<String> = combined
        .expect_err("the combined amend is rejected")
        .iter()
        .map(ToString::to_string)
        .collect();
    assert_eq!(errors, [refusal]);
    let (edited, _) = applied(
        &working,
        vec![amend(
            "t4",
            Amend {
                priority: Some(5),
                ..Amend::default()
            },
        )],
    );
    assert_eq!(edited.task("t4").unwrap().spec.priority, 5);
    // A paused task has started (milestone 9 decision 42c).
    let mut paused = chain();
    set_state(
        &mut paused,
        "t4",
        TaskState::Blocked,
        Some(BlockReason::MessagePause),
    );
    assert_eq!(
        rejected(&paused, vec![amend_stage("t4", 2, None)]),
        [format!(
            "task t4 is {}; stage can be amended only on pending, queued or blocked tasks",
            state_label(paused.task("t4").unwrap())
        )]
    );

    // Not started: applied, with the other field, and logged.
    for state in [TaskState::Pending, TaskState::Queued, TaskState::Blocked] {
        let mut run = chain();
        set_state(&mut run, "t4", state, None);
        let (edited, _) = applied(&run, vec![amend_stage("t4", 2, Some(7))]);
        let t4 = edited.task("t4").unwrap();
        assert_eq!(t4.spec.stage, 2, "{state:?}");
        assert_eq!(t4.spec.priority, 7, "{state:?}");
        assert_eq!(t4.history.last().unwrap().text, "amended: priority, stage");
    }

    // Re-validated: t3 depends on t2, so t2 cannot move above it.
    assert_eq!(
        rejected(&chain(), vec![amend_stage("t2", 2, None)]),
        ["task t3: stage: stage 1 cannot depend on t2 in stage 2"]
    );
    // Moving t3 and then t2 in one batch keeps the order legal.
    let (edited, _) = applied(
        &chain(),
        vec![amend_stage("t3", 2, None), amend_stage("t2", 2, None)],
    );
    assert_eq!(stage_of(&edited, "t2"), 2);
    assert_eq!(stage_of(&edited, "t3"), 2);
    // A gap is refused: stage 2 would have no task.
    assert_eq!(
        rejected(&chain(), vec![amend_stage("t4", 3, None)]),
        ["stage: stages must be numbered from 1 without gaps: stage 2 has no task"]
    );
    // Out of range.
    assert_eq!(
        rejected(&chain(), vec![amend_stage("t4", 33, None)]),
        ["task t4: stage: must be between 1 and 32"]
    );
}

/// Decision 44: a stage whose tasks were all cancelled still exists, and an atomic
/// task counts until it is cancelled.
#[test]
fn stage_rules_count_cancelled_and_merged_tasks_as_decided() {
    let (run, _) = applied(&chain(), vec![amend_stage("t4", 2, None)]);
    let (run, _) = applied(&run, vec![cancel("t4")]);
    let (run, _) = applied(&run, vec![add(&one("t5", "stage = 3"))]);
    assert_eq!(stage_of(&run, "t5"), 3);

    let atomic = "atomic = true\natomic_reason = \"protocol bump\"";
    let (mut run, _) = applied(&flat(), vec![add(&one("t5", atomic))]);
    set_state(&mut run, "t5", TaskState::Merged, None);
    assert_eq!(
        rejected(&run, vec![add(&one("t6", atomic))]),
        ["task t6: atomic: stage 1 already has the atomic task t5"]
    );
    set_state(&mut run, "t5", TaskState::Cancelled, None);
    applied(&run, vec![add(&one("t6", atomic))]);
}

/// Decision 39: the `fix<n>` ids are reserved for new tasks; a fix task the engine
/// added can still be amended.
#[test]
fn an_existing_fix_task_can_still_be_amended() {
    let mut run = flat();
    run.tasks[3].spec.id = "fix1".into();
    let (edited, _) = applied(&run, vec![amend_stage("fix1", 1, Some(100))]);
    assert_eq!(edited.task("fix1").unwrap().spec.priority, 100);
    assert_eq!(
        rejected(&run, vec![add(&one("fix2", ""))]),
        ["task fix2: id: fix<n> ids are reserved for fix tasks the engine adds"]
    );
}

/// Ruling C-14 (d): a blocked task with a worktree has started, so its stage stays; the
/// per-state text of `not_started` does not fit a blocked task, so it gets its own.
#[test]
fn a_blocked_task_with_a_start_commit_keeps_its_stage() {
    for reason in [BlockReason::Human, BlockReason::Conflict] {
        let mut run = chain();
        set_state(&mut run, "t4", TaskState::Blocked, Some(reason));
        run.tasks[3].start_commit = Some("c".repeat(40));
        assert_eq!(
            rejected(&run, vec![amend_stage("t4", 2, Some(5))]),
            ["task t4 has started: its stage cannot change"],
            "{reason:?}"
        );
        // Without a start commit it has not started, and moves.
        run.tasks[3].start_commit = None;
        let (edited, _) = applied(&run, vec![amend_stage("t4", 2, None)]);
        assert_eq!(stage_of(&edited, "t4"), 2, "{reason:?}");
    }
}

/// Ruling C-14 (a): split children take their parent's stage; a child naming another
/// stage is refused. `stage` defaults to 1 in serde, so a child naming 1 cannot be told
/// from one naming none, and takes the parent's stage too.
#[test]
fn split_children_take_their_parents_stage() {
    // t4 in stage 2 (t1..t3 stay in stage 1).
    let (run, _) = applied(&chain(), vec![amend_stage("t4", 2, None)]);
    let split = |children: Vec<String>| PlanEdit::SplitTask {
        task_id: "t4".into(),
        into: children.iter().map(|c| spec(c)).collect(),
    };
    let (edited, _) = applied(
        &run,
        vec![split(vec![one("t5", ""), one("t6", "stage = 1")])],
    );
    assert_eq!(stage_of(&edited, "t5"), 2);
    assert_eq!(stage_of(&edited, "t6"), 2);
    assert_eq!(
        rejected(&run, vec![split(vec![one("t5", "stage = 3")])]),
        ["task t5: split_task: children stay in their parent's stage"]
    );
}

/// Ruling C-14 (e): `stage-<n>` is reserved for new tasks only, as `fix<n>` is, so a
/// task an M9 run already holds under that name can still be amended.
#[test]
fn an_existing_stage_named_task_can_still_be_amended() {
    let mut run = flat();
    run.tasks[3].spec.id = "stage-3".into();
    let (edited, _) = applied(&run, vec![amend_stage("stage-3", 1, Some(9))]);
    assert_eq!(edited.task("stage-3").unwrap().spec.priority, 9);
    assert_eq!(
        rejected(&run, vec![add(&one("stage-4", ""))]),
        ["task stage-4: id: stage-4 is reserved for stage branches"]
    );
}
