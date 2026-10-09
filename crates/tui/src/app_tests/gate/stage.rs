//! Milestone 9.1 task M9.1.20: the edit form's stage field (decision 55).

use super::*;
use crate::tree::stage_fixtures::staged_gate_fixture;

fn staged_gate() -> App {
    let (snap, windows) = staged_gate_fixture();
    let mut app = app_with_runs(windows, snap);
    open_run_view(&mut app, RUN_ID);
    app
}

fn move_to(task: &str, stage: u16) -> PlanEdit {
    PlanEdit::AmendTask {
        task_id: task.into(),
        route: None,
        size: None,
        brief: None,
        acceptance: None,
        test_mode: None,
        test_mode_reason: None,
        priority: None,
        deps: None,
        stage: Some(stage),
        race: None,
        pair: None,
    }
}

#[test]
fn plan_gate_edits_a_not_started_tasks_stage() {
    let mut app = staged_gate();
    open_form(&mut app, "t2");
    assert_eq!(
        form(&app).value_parts(EditField::Stage),
        ("‹ 2 ›".into(), None)
    );
    focus(&mut app, EditField::Stage);
    // Cycles 1 ..= highest + 1: 2 → 3 → 1 → 2 → 3, and back 3 → 2.
    let mut seen = Vec::new();
    for _ in 0..4 {
        assert!(tap(&mut app, KeyCode::Right).is_empty());
        seen.push(form(&app).stage);
    }
    assert_eq!(seen, [3, 1, 2, 3]);
    assert!(tap(&mut app, KeyCode::Left).is_empty());
    assert!(tap(&mut app, KeyCode::Left).is_empty());
    assert_eq!(form(&app).stage, 1);
    assert_eq!(
        tap(&mut app, KeyCode::Enter),
        form_edit(vec![move_to("t2", 1)])
    );

    // Back at its own stage, nothing changed.
    let mut app = staged_gate();
    open_form(&mut app, "t1");
    focus(&mut app, EditField::Stage);
    tap(&mut app, KeyCode::Right);
    tap(&mut app, KeyCode::Left);
    assert!(tap(&mut app, KeyCode::Enter).is_empty());
    assert!(app.modal.is_none());
}

/// A task that started has no stage field.
#[test]
fn a_started_task_has_no_stage_field() {
    let (mut snap, _) = staged_gate_fixture();
    let run = snap.runs.remove(0);
    let mut started = run.tasks[0].clone();
    assert!(
        TaskEditForm::in_run(&run, &started)
            .visible_fields()
            .contains(&EditField::Stage)
    );
    started.state = TaskState::Working;
    assert!(
        !TaskEditForm::in_run(&run, &started)
            .visible_fields()
            .contains(&EditField::Stage)
    );
    // A form opened without its run has none either (milestone 8c's form).
    assert!(
        !TaskEditForm::new(RUN_ID, &started)
            .visible_fields()
            .contains(&EditField::Stage)
    );
}

fn has_stage_field(run: &proto::RunInfo, task: &proto::TaskInfo) -> bool {
    TaskEditForm::in_run(run, task)
        .visible_fields()
        .contains(&EditField::Stage)
}

/// The daemon's `not_started` (pending, queued or blocked, not paused by a message),
/// and no agent round yet.
#[test]
fn the_stage_field_follows_the_daemons_not_started() {
    use proto::{BlockInfo, BlockReason};
    let (mut snap, _) = staged_gate_fixture();
    let run = snap.runs.remove(0);
    let mut task = run.tasks[0].clone();
    task.state = TaskState::Blocked;
    task.block = Some(BlockInfo::new(
        BlockReason::MisSized,
        "check failed 3 times",
    ));
    assert!(has_stage_field(&run, &task), "blocked, not started");
    task.block = Some(BlockInfo::new(BlockReason::MessagePause, String::new()));
    assert!(!has_stage_field(&run, &task), "paused by a message");

    // A task with a round has started, whatever its state says.
    let (three, _) = crate::tree::run_fixtures::three_task_fixture();
    let mut worked = three.runs[0].tasks[0].clone();
    assert!(!worked.rounds.is_empty());
    worked.state = TaskState::Pending;
    assert!(!has_stage_field(&run, &worked), "a task with a round");
}

/// The form is stale once the task's stage moved elsewhere, as for its other fields.
#[test]
fn a_moved_stage_makes_the_form_stale() {
    let (mut snap, _) = staged_gate_fixture();
    let run = snap.runs.remove(0);
    let mut task = run.tasks[1].clone();
    let form = TaskEditForm::in_run(&run, &task);
    assert!(form.opened_from(&task));
    task.stage = 1;
    assert!(!form.opened_from(&task));
}
