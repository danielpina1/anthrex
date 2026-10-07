//! Milestone 9 decision 25's `amend_task deps`, and a sub-planner's added tasks
//! inheriting its epic (M9.4). Every test goes through `apply_edits`.

use proto::{BlockReason, Effort, PlanEdit, PlanTask, RouteSpec, TaskState};

use super::*;
use crate::run::model_roles::RunModels;
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
            stage: None,
            race: None,
            pair: None,
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

/// Second review, ruling 4: a sub-planner's split of a task that does not exist says
/// so, before any epic rule: the unknown task, not its would-be children's epic.
#[test]
fn a_planners_split_of_an_unknown_task_reports_the_unknown_task() {
    let mut run = run_ok(&plan_with(PROFILE, &[one("a1", "epic = \"auth\"")]));
    for e in ["auth", "billing"] {
        run.orch
            .epics
            .push(EpicRecord::new(e, PlannerPhase::Planning));
    }
    let source = EditSource::Planner {
        epic: "auth".into(),
    };
    let edits = [PlanEdit::SplitTask {
        task_id: "ghost".into(),
        into: vec![spec(&one("a8", "epic = \"billing\"")), spec(&one("a9", ""))],
    }];
    let errors: Vec<String> = apply_edits(&run, &edits, &EditScope::Run, &source, 9)
        .unwrap_err()
        .iter()
        .map(ToString::to_string)
        .collect();
    assert_eq!(errors, ["task ghost: task_id: no such task"]);
}

/// Milestone 9.8 decision 31: the route an orchestrator or a sub-planner sends.
fn sent_route() -> RouteSpec {
    RouteSpec {
        runtime: Some(proto::Runtime::Codex),
        model: Some("gpt-6-sol".into()),
        strength: Some(proto::Strength::Frontier),
        effort: Some(Effort::HIGH),
    }
}

/// `one(id, "")` carrying [`sent_route`].
fn routed(id: &str) -> PlanTask {
    let mut task = spec(&one(id, ""));
    task.route = sent_route();
    task
}

/// Decision 31 for `source`, on `run` (whose `t3` and `t4` it may edit): an added task,
/// a split's children and an amended task each drop the route they were sent and take
/// their row's; each batch notes it once, and all three in one batch once too.
fn routes_are_ignored(run: &Run, source: &EditSource) {
    let check = |edits: Vec<PlanEdit>, ids: &[&str]| {
        let (edited, consequences) = apply_edits(run, &edits, &EditScope::Run, source, 9)
            .unwrap_or_else(|e| panic!("{source:?}: {}", show(&e)));
        for id in ids {
            let t = task(&edited, id);
            assert_eq!(t.spec.route, RouteSpec::default(), "{source:?} {id}");
            let row = edited.limits.models().route(RunModels::task_role(t));
            assert_eq!(t.route, row, "{source:?} {id}");
        }
        assert_eq!(
            consequences,
            vec![EditConsequence::RouteIgnored],
            "{source:?} {ids:?}"
        );
    };
    let add = || PlanEdit::AddTask { task: routed("a1") };
    let split = || PlanEdit::SplitTask {
        task_id: "t4".into(),
        into: vec![routed("a2"), routed("a3")],
    };
    let amended = || {
        amend(
            "t3",
            Amend {
                route: Some(sent_route()),
                priority: Some(4),
                ..Amend::default()
            },
        )
    };
    check(vec![add()], &["a1"]);
    check(vec![split()], &["a2", "a3"]);
    check(vec![amended()], &["t3"]);
    check(vec![add(), split(), amended()], &["a1", "a2", "a3", "t3"]);
    // An amend that names only a route changes nothing and is not refused.
    let only = amend(
        "t3",
        Amend {
            route: Some(sent_route()),
            ..Amend::default()
        },
    );
    check(vec![only], &["t3"]);
}

#[test]
fn an_orchestrators_route_is_ignored_and_noted() {
    routes_are_ignored(&flat(), &EditSource::Orchestrator);
}

#[test]
fn a_planners_route_is_ignored() {
    let mut run = flat();
    run.tasks[2].spec.epic = Some("e1".into());
    run.tasks[3].spec.epic = Some("e1".into());
    run.orch
        .epics
        .push(EpicRecord::new("e1", PlannerPhase::Planning));
    routes_are_ignored(&run, &EditSource::Planner { epic: "e1".into() });
}

/// Decision 31: a user's `amend_task` (`run edit`, the task edit form) keeps its route
/// (decision 10), with nothing noted.
#[test]
fn a_users_amend_keeps_its_route() {
    let route = RouteSpec {
        model: Some("claude-opus-5-5".into()),
        effort: Some(Effort::HIGH),
        ..RouteSpec::default()
    };
    let edit = amend(
        "t3",
        Amend {
            route: Some(route.clone()),
            ..Amend::default()
        },
    );
    let (edited, consequences) = applied(&flat(), vec![edit]);
    let t3 = task(&edited, "t3");
    assert_eq!(t3.spec.route, route);
    assert_eq!(
        (t3.route.model.as_str(), &t3.route.effort),
        ("claude-opus-5-5", &Effort::HIGH)
    );
    assert_eq!(consequences, vec![]);
}
