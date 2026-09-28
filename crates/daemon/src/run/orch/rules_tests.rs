//! Decision 23's plan rules for the orchestrator's and sub-planners' batches (M9.4).
//! Pure: every run is built from plan text, then given the milestone 9 state a rule
//! reads (epics, scout reports, integration reviews) by hand.

use std::collections::BTreeSet;

use proto::{PlanEdit, TaskState};

use super::*;
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

fn run_of(tasks: &[String]) -> Run {
    run_ok(&plan_with(PROFILE, tasks))
}

fn touched<'a>(ids: &[&'a str]) -> Vec<&'a str> {
    ids.to_vec()
}

/// [`check`] on `run` as a batch that added the `new` tasks to the run without them.
fn check_new(run: &Run, new: &[&str], source: &EditSource) -> Vec<PlanError> {
    let mut before = run.clone();
    before.tasks.retain(|t| !new.contains(&t.id()));
    check(run, &before, source)
}

fn shown(errors: Vec<PlanError>) -> Vec<String> {
    errors.iter().map(ToString::to_string).collect()
}

fn epic(id: &str, phase: PlannerPhase) -> EpicRecord {
    EpicRecord::new(id, phase)
}

fn planner(e: &str) -> EditSource {
    EditSource::Planner { epic: e.into() }
}

fn with_cap(mut run: Run, cap: u32) -> Run {
    run.limits.orch.planner_task_cap = cap;
    run
}

const BUDGET: &str = "[task.budget]\ntool_calls = 10\nminutes = 5";
const BUDGET_ERROR: &str =
    "task t1: budget: budgets come from the task's size; leave budget out (rule 7.1)";

/// One task added through `apply_edits` from `source`, parsed from a `[[task]]` table.
fn add(run: &Run, table: &str, source: &EditSource) -> Result<Run, Vec<String>> {
    let mut plan = parse_plan(&plan_with(PROFILE, &[table.to_string()])).unwrap();
    let edit = PlanEdit::AddTask {
        task: plan.tasks.pop().unwrap(),
    };
    apply_edits(run, &[edit], &EditScope::Run, source, 7)
        .map(|(run, _)| run)
        .map_err(shown)
}

#[test]
fn planner_budget_is_refused() {
    let run = run_of(&[one("t1", BUDGET)]);
    assert_eq!(
        shown(check_new(
            &run,
            &touched(&["t1"]),
            &EditSource::Orchestrator
        )),
        [BUDGET_ERROR]
    );
    let mut run = run_of(&[one("t1", &format!("epic = \"auth\"\n{BUDGET}"))]);
    run.orch.epics.push(epic("auth", PlannerPhase::Planning));
    assert_eq!(
        shown(check_new(&run, &touched(&["t1"]), &planner("auth"))),
        [BUDGET_ERROR]
    );
    // Through the edit path: the orchestrator's batch is refused whole.
    let base = run_of(&[one("t0", "")]);
    assert_eq!(
        add(&base, &one("t1", BUDGET), &EditSource::Orchestrator).unwrap_err(),
        [BUDGET_ERROR]
    );
}

/// Plan files and the user's `run edit` keep M8a's rules only.
#[test]
fn plan_file_budget_is_still_accepted() {
    let mut run = run_of(&[one("t1", &format!("epic = \"nope\"\n{BUDGET}"))]);
    run.scout_reports = vec!["s1".into()];
    run = with_cap(run, 1);
    assert_eq!(run.tasks[0].budget.tool_calls, 10);
    assert!(check_new(&run, &touched(&["t1"]), &EditSource::User).is_empty());
    let added = add(&run, &one("t2", BUDGET), &EditSource::User).unwrap();
    assert_eq!(added.tasks[1].budget.minutes, 5);
    assert!(
        !added.tasks[1].notes.iter().any(|n| n == UNBACKED_NOTE),
        "a user's task gets no evidence note"
    );
}

#[test]
fn code_task_without_scout_refs_is_refused_when_reports_exist() {
    let error = "task t1: scout_refs: name the scout reports this task's size rests on (rule 7.1)";
    let tasks = [
        one("t1", ""),
        task_toml("r1", "S", "[]", "kind = \"research\""),
    ];
    let mut run = run_of(&tasks);
    run.scout_reports = vec!["s1".into()];
    assert_eq!(
        shown(check_new(
            &run,
            &touched(&["t1", "r1"]),
            &EditSource::Orchestrator
        )),
        [error]
    );
    // The onboarding report counts as a finished report too.
    let mut run = run_of(&tasks);
    run.onboarding_report = Some("0123abcd".into());
    assert_eq!(
        shown(check_new(
            &run,
            &touched(&["t1", "r1"]),
            &EditSource::Orchestrator
        )),
        [error]
    );
    // A docs task is held to it; an untouched task is not.
    let mut run = run_of(&[
        one("t1", "kind = \"docs\"\ntest_mode_reason = \"prose\""),
        one("t2", ""),
    ]);
    run.scout_reports = vec!["s1".into()];
    assert_eq!(
        shown(check_new(
            &run,
            &touched(&["t1"]),
            &EditSource::Orchestrator
        )),
        [error]
    );
}

#[test]
fn unknown_scout_ref_is_refused() {
    let mut run = run_of(&[one("t1", "scout_refs = [\"s1\", \"s9\", \"onboarding\"]")]);
    run.scout_reports = vec!["s1".into()];
    assert_eq!(
        shown(check_new(
            &run,
            &touched(&["t1"]),
            &EditSource::Orchestrator
        )),
        [
            "task t1: scout_refs: s9 is not a finished scout report of this run",
            "task t1: scout_refs: onboarding is not a finished scout report of this run",
        ]
    );
}

#[test]
fn no_reports_gives_a_note_not_an_error() {
    let run = run_of(&[one("t0", "")]);
    assert!(check_new(&run, &touched(&["t0"]), &EditSource::Orchestrator).is_empty());
    let added = add(&run, &one("t1", ""), &EditSource::Orchestrator).unwrap();
    assert!(
        added.tasks[1].notes.iter().any(|n| n == UNBACKED_NOTE),
        "{:?}",
        added.tasks[1].notes
    );
    assert_eq!(UNBACKED_NOTE, "size not backed by a scout report");
    // A task that names its evidence gets no note once a report exists.
    let mut run = run_of(&[one("t0", "")]);
    run.scout_reports = vec!["s1".into()];
    let added = add(
        &run,
        &one("t1", "scout_refs = [\"s1\"]"),
        &EditSource::Orchestrator,
    )
    .unwrap();
    assert!(!added.tasks[1].notes.iter().any(|n| n == UNBACKED_NOTE));
}

#[test]
fn onboarding_ref_is_accepted() {
    let mut run = run_of(&[one("t1", "scout_refs = [\"onboarding\"]")]);
    run.onboarding_report = Some("0123abcd".into());
    assert!(check_new(&run, &touched(&["t1"]), &EditSource::Orchestrator).is_empty());
    // The control: with no onboarding report the alias names nothing.
    run.onboarding_report = None;
    assert_eq!(
        shown(check_new(
            &run,
            &touched(&["t1"]),
            &EditSource::Orchestrator
        )),
        ["task t1: scout_refs: onboarding is not a finished scout report of this run"]
    );
}

#[test]
fn orchestrator_cap_counts_tasks_without_an_epic() {
    let tasks = [
        one("t1", ""),
        one("t2", ""),
        one("t3", ""),
        one("a1", "epic = \"auth\""),
    ];
    let mut run = with_cap(run_of(&tasks), 2);
    run.orch.epics.push(epic("auth", PlannerPhase::Finished));
    assert_eq!(
        shown(check_new(
            &run,
            &touched(&["t3"]),
            &EditSource::Orchestrator
        )),
        [
            "tasks: the orchestrator's plan has 3 tasks, more than planner_task_cap (2); plan the rest through sub-planners (rule 5.1)"
        ]
    );
    let run = with_cap(run, 3);
    assert!(check_new(&run, &touched(&["t3"]), &EditSource::Orchestrator).is_empty());
}

#[test]
fn epic_cap_counts_the_epics_tasks() {
    let tasks = [
        one("t1", ""),
        one("a1", "epic = \"auth\""),
        one("a2", "epic = \"auth\""),
        one("a3", "epic = \"auth\""),
    ];
    let mut run = with_cap(run_of(&tasks), 2);
    run.orch.epics.push(epic("auth", PlannerPhase::Planning));
    run.tasks[3].state = TaskState::Merged;
    assert_eq!(
        shown(check_new(&run, &touched(&["a1", "a2"]), &planner("auth"))),
        ["tasks: epic auth has 3 tasks, more than planner_task_cap (2); split the epic (rule 5.1)"]
    );
}

#[test]
fn cancelled_and_integration_review_tasks_do_not_count() {
    let tasks = [
        one("t1", ""),
        one("t2", ""),
        one("t3", ""),
        one("a1", "epic = \"auth\""),
        one("a2", "epic = \"auth\""),
        task_toml(
            "auth-int1",
            "M",
            "[]",
            "epic = \"auth\"\nkind = \"review\"\nreview_target = \"main..HEAD\"",
        ),
    ];
    let mut run = with_cap(run_of(&tasks), 2);
    run.orch.epics.push(epic("auth", PlannerPhase::Finished));
    run.tasks[0].state = TaskState::Cancelled;
    run.tasks[5].orch.integration_of = Some("auth".into());
    let ids = touched(&["t3", "a2"]);
    assert_eq!(
        shown(check_new(&run, &ids, &EditSource::Orchestrator)),
        [""; 0]
    );
    // The controls: counted, each group is over the cap.
    let mut counted = run.clone();
    counted.tasks[0].state = TaskState::Pending;
    counted.tasks[5].orch.integration_of = None;
    assert_eq!(
        shown(check_new(&counted, &ids, &EditSource::Orchestrator)),
        [
            "tasks: the orchestrator's plan has 3 tasks, more than planner_task_cap (2); plan the rest through sub-planners (rule 5.1)",
            "tasks: epic auth has 3 tasks, more than planner_task_cap (2); split the epic (rule 5.1)",
        ]
    );
}

#[test]
fn task_naming_an_unknown_epic_is_refused() {
    let mut run = run_of(&[one("t1", "epic = \"nope\"")]);
    run.orch.epics.push(epic("auth", PlannerPhase::Finished));
    assert_eq!(
        shown(check_new(
            &run,
            &touched(&["t1"]),
            &EditSource::Orchestrator
        )),
        ["task t1: epic: nope is not an epic of this run; create it with spawn_subplanner"]
    );
}

#[test]
fn task_for_a_live_planners_epic_from_the_orchestrator_is_refused() {
    for phase in [PlannerPhase::Queued, PlannerPhase::Planning] {
        let mut run = run_of(&[one("t1", "epic = \"auth\"")]);
        run.orch.epics.push(epic("auth", phase.clone()));
        assert_eq!(
            shown(check_new(
                &run,
                &touched(&["t1"]),
                &EditSource::Orchestrator
            )),
            ["task t1: epic: epic auth is being planned by its sub-planner"],
            "{phase:?}"
        );
        // The epic's own planner may.
        assert!(check_new(&run, &touched(&["t1"]), &planner("auth")).is_empty());
    }
}

#[test]
fn fix_task_for_a_finished_epic_is_accepted() {
    for phase in [
        PlannerPhase::Finished,
        PlannerPhase::Failed {
            reason: "timeout".into(),
        },
    ] {
        let mut run = run_of(&[one("t1", "epic = \"auth\"")]);
        run.orch.epics.push(epic("auth", phase));
        assert!(check_new(&run, &touched(&["t1"]), &EditSource::Orchestrator).is_empty());
    }
}

#[test]
fn integration_review_ids_are_reserved() {
    let ids = ["auth-int1", "x-int12", "a-int", "a-intx", "int1", "a-int1b"];
    let run = run_of(&ids.map(|id| one(id, "")));
    assert_eq!(
        shown(check_new(&run, &touched(&ids), &EditSource::Orchestrator)),
        [
            "task auth-int1: id: ids ending in -int<n> are reserved for integration reviews",
            "task x-int12: id: ids ending in -int<n> are reserved for integration reviews",
        ]
    );
}

/// Every rule id this file produces is one of decision 23's.
#[test]
fn rule_ids_are_the_documented_ones() {
    let tasks = [
        one("t1", BUDGET),
        one("t2", "scout_refs = [\"s9\"]"),
        one("t3", ""),
        one("t4", "epic = \"nope\""),
        one("t5", "epic = \"auth\""),
        one("t6-int1", ""),
        one("a1", "epic = \"done\""),
        one("a2", "epic = \"done\""),
    ];
    let mut run = with_cap(run_of(&tasks), 1);
    run.scout_reports = vec!["s1".into()];
    run.orch.epics.push(epic("auth", PlannerPhase::Planning));
    run.orch.epics.push(epic("done", PlannerPhase::Finished));
    let ids = touched(&["t1", "t2", "t3", "t4", "t5", "t6-int1", "a1", "a2"]);
    let rules: BTreeSet<String> = check_new(&run, &ids, &EditSource::Orchestrator)
        .into_iter()
        .map(|e| e.rule)
        .collect();
    let documented: BTreeSet<String> = [
        "7.1.budget",
        "7.1.evidence",
        "5.1.cap",
        "2.epic",
        "5.3.reserved",
    ]
    .map(String::from)
    .into();
    assert_eq!(rules, documented);
}

/// Decision 37's own review task is exempt from the reserved-id rule (M9.4 review
/// fixes, ruling 5).
#[test]
fn an_integration_review_keeps_its_reserved_id() {
    let review = task_toml(
        "auth-int1",
        "M",
        "[]",
        "epic = \"auth\"\nkind = \"review\"\nreview_target = \"main..HEAD\"",
    );
    let mut run = run_of(&[one("a1", "epic = \"auth\""), review]);
    run.orch.epics.push(epic("auth", PlannerPhase::Finished));
    run.tasks[1].orch.integration_of = Some("auth".into());
    assert_eq!(
        shown(check_new(&run, &["auth-int1"], &EditSource::Orchestrator)),
        [""; 0]
    );
    // The control: without the mark, the id is reserved.
    run.tasks[1].orch.integration_of = None;
    assert_eq!(
        shown(check_new(&run, &["auth-int1"], &EditSource::Orchestrator)),
        ["task auth-int1: id: ids ending in -int<n> are reserved for integration reviews"]
    );
}
