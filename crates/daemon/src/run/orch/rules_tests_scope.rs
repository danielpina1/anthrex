//! The M9.4 review fixes: decision 23's rules apply to what a batch changes. Evidence
//! and budget (23.1, 23.2) apply to added and split-in tasks and to an `amend_task`
//! that changes `size`; the cap (23.3) refuses only a batch that raised a group's count
//! above it; decision 23.5's fix task after a finished integration review is exempt
//! from its epic's cap. Every test goes through `apply_edits`.

use proto::{PlanEdit, PlanTask, Size, TaskState};

use crate::run::edits::apply_edits;
use crate::run::model::Run;
use crate::run::orch::{EditSource, EpicRecord, PlannerPhase};
use crate::run::plan::parse_plan;
use crate::run::test_support::*;
use crate::run::validate::EditScope;

/// One S code task owning `crates/<id>/src/lib.rs`.
fn one(id: &str, extra: &str) -> String {
    task_toml(id, "S", &format!("[\"crates/{id}/src/lib.rs\"]"), extra)
}

fn spec(table: &str) -> PlanTask {
    let mut plan = parse_plan(&plan_with(PROFILE, &[table.to_string()])).unwrap();
    plan.tasks.pop().unwrap()
}

/// An `amend_task` of `task_id` with the given fields and no others.
fn amend(
    task_id: &str,
    brief: Option<&str>,
    priority: Option<i32>,
    size: Option<Size>,
) -> PlanEdit {
    PlanEdit::AmendTask {
        task_id: task_id.into(),
        brief: brief.map(String::from),
        acceptance: None,
        route: None,
        test_mode: None,
        test_mode_reason: None,
        priority,
        size,
        deps: None,
    }
}

fn amend_deps(task_id: &str, deps: &[&str]) -> PlanEdit {
    let PlanEdit::AmendTask {
        task_id,
        brief,
        acceptance,
        route,
        test_mode,
        test_mode_reason,
        priority,
        size,
        ..
    } = amend(task_id, None, None, None)
    else {
        unreachable!()
    };
    PlanEdit::AmendTask {
        task_id,
        brief,
        acceptance,
        route,
        test_mode,
        test_mode_reason,
        priority,
        size,
        deps: Some(deps.iter().map(|d| d.to_string()).collect()),
    }
}

fn add(table: &str) -> PlanEdit {
    PlanEdit::AddTask { task: spec(table) }
}

fn split(task_id: &str, into: &[&str]) -> PlanEdit {
    PlanEdit::SplitTask {
        task_id: task_id.into(),
        into: into.iter().map(|id| spec(&one(id, ""))).collect(),
    }
}

fn orchestrator(run: &Run, edits: &[PlanEdit]) -> Result<Run, Vec<String>> {
    apply_edits(run, edits, &EditScope::Run, &EditSource::Orchestrator, 7)
        .map(|(run, _)| run)
        .map_err(|errors| errors.iter().map(ToString::to_string).collect())
}

const EVIDENCE: &str =
    "task t1: scout_refs: name the scout reports this task's size rests on (rule 7.1)";

/// An onboarded run whose t1, from the plan file, names no scout report.
fn onboarded() -> Run {
    let mut run = run_ok(&plan_with(
        PROFILE,
        &[
            one("t0", ""),
            one("t1", ""),
            one("t2", "scout_refs = [\"onboarding\"]"),
        ],
    ));
    run.onboarding_report = Some("0123abcd".into());
    run
}

/// Scenario A: amends of `deps`, `priority` or `brief` alone are not ruled.
#[test]
fn amends_that_keep_the_size_are_not_ruled() {
    let run = onboarded();
    let run = orchestrator(&run, &[amend_deps("t1", &["t0"])]).unwrap();
    let run = orchestrator(&run, &[amend("t1", None, Some(5), None)]).unwrap();
    let run = orchestrator(&run, &[amend("t1", Some("a new brief"), None, None)]).unwrap();
    let t1 = run.tasks.iter().find(|t| t.id() == "t1").unwrap();
    assert_eq!(
        (t1.spec.priority, t1.spec.deps.clone()),
        (5, vec!["t0".into()])
    );
    // An amend to the size the task already has changes nothing either.
    orchestrator(&run, &[amend("t1", None, None, Some(Size::S))]).unwrap();
}

#[test]
fn a_size_amend_needs_evidence() {
    let run = onboarded();
    assert_eq!(
        orchestrator(&run, &[amend("t1", None, None, Some(Size::M))]).unwrap_err(),
        [EVIDENCE]
    );
    // The control: a task that names its evidence may be resized.
    orchestrator(&run, &[amend("t2", None, None, Some(Size::M))]).unwrap();
}

/// Scenario B: a group already over the cap (from the user's plan) refuses only a batch
/// that raises its count.
#[test]
fn an_over_cap_group_refuses_only_a_batch_that_grows_it() {
    let mut run = run_ok(&plan_with(
        PROFILE,
        &[one("t1", ""), one("t2", ""), one("t3", "")],
    ));
    run.limits.orch.planner_task_cap = 2;
    orchestrator(&run, &[amend("t1", None, Some(3), None)]).unwrap();
    assert_eq!(
        orchestrator(&run, &[add(&one("t4", ""))]).unwrap_err(),
        [
            "tasks: the orchestrator's plan has 4 tasks, more than planner_task_cap (2); plan the rest through sub-planners (rule 5.1)"
        ]
    );
    // Unchanged: a split into one task, or a cancel with an add.
    orchestrator(&run, &[split("t1", &["t1a"])]).unwrap();
    let cancel = PlanEdit::CancelTask {
        task_id: "t2".into(),
    };
    orchestrator(&run, &[cancel, add(&one("t4", ""))]).unwrap();
    assert_eq!(
        orchestrator(&run, &[split("t1", &["t1a", "t1b"])]).unwrap_err(),
        [
            "tasks: the orchestrator's plan has 4 tasks, more than planner_task_cap (2); plan the rest through sub-planners (rule 5.1)"
        ]
    );
}

/// Decision 23.5 against 23.3: once an epic's integration review has finished, the
/// orchestrator's fix task for it does not meet the epic's cap.
#[test]
fn a_fix_task_after_a_finished_integration_review_is_exempt_from_the_cap() {
    let review = task_toml(
        "auth-int1",
        "M",
        "[]",
        "epic = \"auth\"\nkind = \"review\"\nreview_target = \"main..HEAD\"",
    );
    let mut run = run_ok(&plan_with(
        PROFILE,
        &[
            one("a1", "epic = \"auth\""),
            one("a2", "epic = \"auth\""),
            review,
        ],
    ));
    run.limits.orch.planner_task_cap = 2;
    run.orch
        .epics
        .push(EpicRecord::new("auth", PlannerPhase::Finished));
    run.tasks[2].orch.integration_of = Some("auth".into());
    run.tasks[2].state = TaskState::Reported;
    let fix = add(&one("a3", "epic = \"auth\""));
    orchestrator(&run, std::slice::from_ref(&fix)).unwrap();
    // The control: while the review is unfinished, the cap holds.
    run.tasks[2].state = TaskState::Pending;
    assert_eq!(
        orchestrator(&run, &[fix]).unwrap_err(),
        ["tasks: epic auth has 3 tasks, more than planner_task_cap (2); split the epic (rule 5.1)"]
    );
}
