//! Milestone 9 decision 25's `amend_task deps`, and a sub-planner's added tasks
//! inheriting its epic (M9.4). Every test goes through `apply_edits`.

use proto::{BlockReason, PlanEdit, TaskState};

use super::*;
use crate::run::orch::{EditSource, EpicRecord, PlannerPhase};

fn amend_deps(task_id: &str, deps: &[&str]) -> PlanEdit {
    match amend(task_id, Amend::default()) {
        PlanEdit::AmendTask {
            task_id,
            brief,
            acceptance,
            route,
            test_mode,
            test_mode_reason,
            priority,
            size,
            ..
        } => PlanEdit::AmendTask {
            task_id,
            brief,
            acceptance,
            route,
            test_mode,
            test_mode_reason,
            priority,
            size,
            deps: Some(deps.iter().map(|d| d.to_string()).collect()),
        },
        other => panic!("an amend: {other:?}"),
    }
}

#[test]
fn amend_deps_unblocks_dep_cancelled() {
    // t1; t2 depends on t1; t3 depends on t2; t4 stands alone.
    let (run, _) = applied(&chain(), vec![cancel("t1")]);
    let t2 = run.tasks.iter().find(|t| t.id() == "t2").unwrap();
    assert_eq!(t2.state, TaskState::Blocked);
    assert_eq!(t2.block.as_ref().unwrap().reason, BlockReason::DepCancelled);

    let (edited, consequences) = applied(&run, vec![amend_deps("t2", &["t4"])]);
    assert!(consequences.is_empty(), "{consequences:?}");
    let t2 = edited.tasks.iter().find(|t| t.id() == "t2").unwrap();
    assert_eq!(t2.spec.deps, ["t4"]);
    assert_eq!(t2.state, TaskState::Pending);
    assert_eq!(t2.block, None);
    assert_eq!(t2.history.last().unwrap().text, "amended: deps");

    // An empty list replaces the dependencies too.
    let (edited, _) = applied(&run, vec![amend_deps("t2", &[])]);
    let t2 = edited.tasks.iter().find(|t| t.id() == "t2").unwrap();
    assert!(t2.spec.deps.is_empty());
    assert_eq!(t2.state, TaskState::Pending);

    // A task blocked for another reason stays blocked.
    let mut other = chain();
    set_state(
        &mut other,
        "t2",
        TaskState::Blocked,
        Some(BlockReason::MisSized),
    );
    let (edited, _) = applied(&other, vec![amend_deps("t2", &["t4"])]);
    let t2 = edited.tasks.iter().find(|t| t.id() == "t2").unwrap();
    assert_eq!(t2.state, TaskState::Blocked);
    assert_eq!(t2.block.as_ref().unwrap().reason, BlockReason::MisSized);
}

#[test]
fn amend_deps_rejects_a_cycle() {
    let run = chain();
    let before = run.clone();
    assert_eq!(
        rejected(&run, vec![amend_deps("t1", &["t3"])]),
        ["deps: cycle t1 -> t3 -> t2 -> t1"]
    );
    assert_eq!(run, before);
}

#[test]
fn amend_deps_rejects_a_cancelled_dep() {
    let (run, _) = applied(&chain(), vec![cancel("t1")]);
    // Keeping the cancelled dependency is refused: the whole list is new.
    assert_eq!(
        rejected(&run, vec![amend_deps("t2", &["t1", "t4"])]),
        ["task t2: deps: t1 is cancelled"]
    );
    assert_eq!(
        rejected(&run, vec![amend_deps("t2", &["t9"])]),
        ["task t2: deps: t9 is not a task"]
    );
}

#[test]
fn amend_deps_only_on_tasks_that_have_not_started() {
    let mut run = chain();
    set_state(&mut run, "t2", TaskState::Working, None);
    assert_eq!(
        rejected(&run, vec![amend_deps("t2", &["t4"])]),
        ["task t2 is working; deps can be amended only on pending, queued or blocked tasks"]
    );
    for state in [TaskState::Pending, TaskState::Queued] {
        let mut run = chain();
        set_state(&mut run, "t3", state, None);
        let (edited, _) = applied(&run, vec![amend_deps("t3", &["t4"])]);
        let t3 = edited.tasks.iter().find(|t| t.id() == "t3").unwrap();
        assert_eq!(t3.spec.deps, ["t4"], "{state:?}");
        // A queued task whose new dependency is not merged waits again.
        assert_eq!(t3.state, TaskState::Pending, "{state:?}");
    }
}

#[test]
fn a_planners_added_task_inherits_its_epic() {
    let mut run = flat();
    // A sub-planner splits only its own epic's tasks (ruling 4).
    run.tasks[3].spec.epic = Some("auth".into());
    run.orch
        .epics
        .push(EpicRecord::new("auth", PlannerPhase::Planning));
    let source = EditSource::Planner {
        epic: "auth".into(),
    };
    let edits = [
        PlanEdit::AddTask {
            task: spec(&one("a1", "")),
        },
        PlanEdit::SplitTask {
            task_id: "t4".into(),
            into: vec![spec(&one("a2", "")), spec(&one("a3", ""))],
        },
    ];
    let (edited, _) = apply_edits(&run, &edits, &EditScope::Run, &source, 9).unwrap();
    for id in ["a1", "a2", "a3"] {
        let task = edited.tasks.iter().find(|t| t.id() == id).unwrap();
        assert_eq!(task.spec.epic.as_deref(), Some("auth"), "{id}");
    }
    // The orchestrator's and the user's tasks keep what they name.
    for source in [EditSource::Orchestrator, EditSource::User] {
        let (edited, _) = apply_edits(&run, &edits[..1], &EditScope::Run, &source, 9).unwrap();
        assert_eq!(edited.tasks.last().unwrap().spec.epic, None, "{source:?}");
    }
}

/// Decision 22 (M9.4 review fixes, ruling 4): a sub-planner's added task naming another
/// epic is refused, never rewritten; one naming none is given the planner's epic.
#[test]
fn a_planners_task_for_another_epic_is_refused() {
    let mut run = flat();
    run.tasks[3].spec.epic = Some("auth".into());
    for e in ["auth", "billing"] {
        run.orch
            .epics
            .push(EpicRecord::new(e, PlannerPhase::Planning));
    }
    let source = EditSource::Planner {
        epic: "auth".into(),
    };
    let edits = [
        PlanEdit::AddTask {
            task: spec(&one("a1", "epic = \"billing\"")),
        },
        PlanEdit::SplitTask {
            task_id: "t4".into(),
            into: vec![spec(&one("a2", "epic = \"billing\"")), spec(&one("a3", ""))],
        },
    ];
    let errors: Vec<String> = apply_edits(&run, &edits, &EditScope::Run, &source, 9)
        .unwrap_err()
        .iter()
        .map(ToString::to_string)
        .collect();
    assert_eq!(
        errors,
        [
            "task a1: epic: a sub-planner adds tasks only to its own epic auth",
            "task a2: epic: a sub-planner adds tasks only to its own epic auth",
        ]
    );
    // Its own epic, named or not, is accepted.
    let own = [PlanEdit::AddTask {
        task: spec(&one("a1", "epic = \"auth\"")),
    }];
    let (edited, _) = apply_edits(&run, &own, &EditScope::Run, &source, 9).unwrap();
    assert_eq!(
        edited.tasks.last().unwrap().spec.epic.as_deref(),
        Some("auth")
    );
}

/// Ruling 4: a sub-planner splits only its own epic's tasks.
#[test]
fn a_planners_split_outside_its_epic_is_refused() {
    let mut run = run_ok(&plan_with(
        PROFILE,
        &[
            one("t1", ""),
            one("a1", "epic = \"auth\""),
            one("b1", "epic = \"billing\""),
        ],
    ));
    for e in ["auth", "billing"] {
        run.orch
            .epics
            .push(EpicRecord::new(e, PlannerPhase::Planning));
    }
    let source = EditSource::Planner {
        epic: "auth".into(),
    };
    for outside in ["t1", "b1"] {
        let edits = [PlanEdit::SplitTask {
            task_id: outside.into(),
            into: vec![spec(&one("a8", "")), spec(&one("a9", ""))],
        }];
        let errors: Vec<String> = apply_edits(&run, &edits, &EditScope::Run, &source, 9)
            .unwrap_err()
            .iter()
            .map(ToString::to_string)
            .collect();
        assert_eq!(
            errors,
            [format!(
                "task {outside}: epic: a sub-planner splits only tasks of its own epic auth"
            )],
            "{outside}"
        );
    }
    // The control: its own epic's task splits.
    let edits = [PlanEdit::SplitTask {
        task_id: "a1".into(),
        into: vec![spec(&one("a8", "")), spec(&one("a9", ""))],
    }];
    let (edited, _) = apply_edits(&run, &edits, &EditScope::Run, &source, 9).unwrap();
    let a9 = edited.tasks.iter().find(|t| t.id() == "a9").unwrap();
    assert_eq!(a9.spec.epic.as_deref(), Some("auth"));
}
