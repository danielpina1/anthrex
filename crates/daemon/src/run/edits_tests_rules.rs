//! Plan edits, decision 13: dependencies, answers, atomicity, the L rule, the edit
//! area and the run-level edits.

use super::*;

#[test]
fn add_dep_creating_a_cycle_is_rejected() {
    let run = chain();
    assert_eq!(
        rejected(&run, vec![add_dep("t1", "t3")]),
        vec!["deps: cycle t1 -> t3 -> t2 -> t1"]
    );

    // A dependency that closes no cycle applies.
    let (edited, _) = applied(&run, vec![add_dep("t4", "t3")]);
    assert_eq!(task(&edited, "t4").spec.deps, vec!["t3"]);

    // And only on a task that has not started.
    let mut working = chain();
    set_state(&mut working, "t4", TaskState::Working, None);
    assert_eq!(
        rejected(&working, vec![add_dep("t4", "t1")]),
        vec![
            "task t4 is working; dependencies can be added only on pending, queued or blocked tasks"
        ]
    );
}

#[test]
fn dep_on_a_cancelled_task_is_rejected() {
    let mut run = flat();
    set_state(&mut run, "t2", TaskState::Cancelled, None);
    assert_eq!(
        rejected(&run, vec![add_dep("t3", "t2")]),
        vec!["task t3: deps: t2 is cancelled"]
    );
    let (edited, _) = applied(&run, vec![add_dep("t3", "t4")]);
    assert_eq!(task(&edited, "t3").spec.deps, vec!["t4"]);
}

#[test]
fn add_dep_returns_a_queued_task_to_pending() {
    let mut run = flat();
    set_state(&mut run, "t1", TaskState::Queued, None);
    set_state(&mut run, "t3", TaskState::Queued, None);
    set_state(&mut run, "t4", TaskState::Merged, None);
    // t1 now waits for an unmerged task; t3's new dependency is already merged.
    let (edited, _) = applied(&run, vec![add_dep("t1", "t2"), add_dep("t3", "t4")]);
    assert_eq!(task(&edited, "t1").state, TaskState::Pending);
    assert_eq!(task(&edited, "t3").state, TaskState::Queued);
}

#[test]
fn answer_only_on_blocked_question_or_working() {
    let mut run = flat();
    set_state(
        &mut run,
        "t1",
        TaskState::Blocked,
        Some(BlockReason::Question),
    );
    set_state(&mut run, "t2", TaskState::Working, None);
    set_state(
        &mut run,
        "t4",
        TaskState::Blocked,
        Some(BlockReason::Conflict),
    );

    let (edited, consequences) = applied(
        &run,
        vec![
            answer("t1", "use the token module"),
            answer("t2", "keep the old API"),
        ],
    );
    assert_eq!(
        answer_message("use the token module"),
        "[anthrex] Answer to your question: use the token module"
    );
    assert_eq!(
        consequences,
        vec![
            EditConsequence::Deliver {
                task_id: "t1".to_string(),
                text: answer_message("use the token module"),
            },
            EditConsequence::Deliver {
                task_id: "t2".to_string(),
                text: answer_message("keep the old API"),
            },
        ]
    );
    assert_eq!(task(&edited, "t1").state, TaskState::Working);
    assert_eq!(task(&edited, "t1").block, None);
    assert_eq!(task(&edited, "t2").state, TaskState::Working);

    assert_eq!(
        rejected(&run, vec![answer("t3", "x"), answer("t4", "y")]),
        vec![
            "task t3 is pending; only blocked(question) or working tasks can be answered",
            "task t4 is blocked(conflict); only blocked(question) or working tasks can be answered",
        ]
    );
}

#[test]
fn a_batch_is_atomic() {
    let run = chain();
    let before = run.clone();
    let add = PlanEdit::AddTask {
        task: spec(&one("t9", "deps = [\"t4\"]")),
    };
    // The add alone is valid.
    assert!(apply(&run, vec![add.clone()]).is_ok());

    let errors = rejected(&run, vec![add.clone(), add_dep("t1", "t2")]);
    assert_eq!(errors, vec!["deps: cycle t1 -> t2 -> t1"]);
    assert_eq!(run, before);

    // Every error of a batch is listed: a refusal and a validation error together.
    let mut working = chain();
    set_state(&mut working, "t4", TaskState::Working, None);
    assert_eq!(
        rejected(
            &working,
            vec![add, add_dep("t4", "t1"), add_dep("t1", "t2"), cancel("t0")]
        ),
        vec![
            "task t4 is working; dependencies can be added only on pending, queued or blocked tasks",
            "task t0: task_id: no such task",
            "deps: cycle t1 -> t2 -> t1",
        ]
    );
}

#[test]
fn edit_leaving_an_l_task_is_rejected() {
    let run = flat();
    assert_eq!(
        rejected(&run, vec![amend_size("t1", Size::L)]),
        vec!["task t1: size: L tasks are never executed; split the task (rule 7.2.4)"]
    );
    let (edited, _) = applied(&run, vec![amend_size("t2", Size::M)]);
    assert_eq!(task(&edited, "t2").size, Size::M);
    assert_eq!(task(&edited, "t2").route.effort, Effort::Medium);
}

#[test]
fn an_untouched_rung3_l_task_does_not_block_other_edits() {
    let mut run = flat();
    // Rung 3 raised t1 to L and blocked it as mis-sized.
    set_state(
        &mut run,
        "t1",
        TaskState::Blocked,
        Some(BlockReason::MisSized),
    );
    run.tasks[0].size = Size::L;
    run.tasks[0].rung = 3;
    run.tasks[0].raised_size = Some(Size::L);
    set_state(&mut run, "t3", TaskState::Working, None);

    let (_, consequences) = applied(&run, vec![answer("t3", "carry on")]);
    assert_eq!(
        consequences,
        vec![EditConsequence::Deliver {
            task_id: "t3".to_string(),
            text: answer_message("carry on"),
        }]
    );

    // An edit that touches the L task ends its exemption.
    assert_eq!(
        rejected(&run, vec![amend_brief("t1", "Smaller now", &["e"])]),
        vec!["task t1: size: L tasks are never executed; split the task (rule 7.2.4)"]
    );
}

#[test]
fn edit_outside_its_area_is_rejected() {
    let run = flat();
    let area = EditScope::Area {
        globs: vec!["crates/daemon/**".to_string()],
    };
    let outside = PlanEdit::AddTask {
        task: spec(&task_toml("t5", "S", "[\"crates/tui/**\"]", "")),
    };
    let errors: Vec<String> = match apply_edits(&run, &[outside], &area, 5_000) {
        Err(errors) => errors.iter().map(ToString::to_string).collect(),
        Ok(_) => panic!("an add outside the area must be rejected"),
    };
    assert_eq!(
        errors,
        vec!["task t5: owns: crates/tui/** is outside the area crates/daemon/**"]
    );

    let inside = PlanEdit::AddTask {
        task: spec(&task_toml("t6", "S", "[\"crates/daemon/src/x.rs\"]", "")),
    };
    let (edited, _) = apply_edits(&run, &[inside], &area, 5_000)
        .unwrap_or_else(|e| panic!("inside the area must apply: {}", show(&e)));
    assert_eq!(ids(&edited), vec!["t1", "t2", "t3", "t4", "t6"]);
}

#[test]
fn pause_resume_finish_are_consequences() {
    let run = chain();
    let (edited, consequences) = applied(
        &run,
        vec![PlanEdit::Finish, PlanEdit::Pause, PlanEdit::Resume],
    );
    assert_eq!(
        consequences,
        vec![
            EditConsequence::Finish,
            EditConsequence::Pause,
            EditConsequence::Resume
        ]
    );
    assert_eq!(edited, run);
}
