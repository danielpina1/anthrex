//! Milestone 9.8 decision 29 on a run (task M9.8.8): rung 2 and `run retry`
//! (`role_step::rung2_route`), the test writer's step (`writer_step`) and an engine fix
//! task's step up (`escalate_for`) go through `role_step::escalate` along the row,
//! stepping over a model that failed in the task (ruling RL-1) and a runtime not
//! installed or held off by the overlap rule (ruling T10a-6).

use proto::models::{ModelRef, ModelTable, Role, RoleChoice};
use proto::{AgentRole, Route, TaskState};

use crate::run::model::{AgentRound, Run};
use crate::run::model_roles::{ModelEfforts, Mover, RunModels};
use crate::run::role_step::{escalate_for, every_route_failed, rung2_route, writer_step};
use crate::run::test_support::{PROFILE, build_with_models, plan_with, set_row, show, task_toml};

const SONNET: &str = "claude:claude-sonnet-5";
const SOL: &str = "codex:gpt-6-sol";
const OPUS: &str = "claude:claude-opus-5-5";

fn m(s: &str) -> ModelRef {
    ModelRef::parse(s).unwrap()
}

fn at(model: &str, effort: &str) -> Route {
    RunModels::route_of(&m(model), (!effort.is_empty()).then_some(effort))
}

/// M tasks `(id, owns, extra)` on the `implementer.medium` row Sonnet at `medium`,
/// falling back to `gpt-6-sol`; Sonnet's catalog list `low`, `medium`, `high`, Sol's
/// none.
fn built(tasks: &[(&str, &str, &str)]) -> Run {
    let row = RoleChoice {
        model: m(SONNET),
        effort: Some("medium".into()),
        fallback: Some(m(SOL)),
    };
    let table = ModelTable {
        rows: [(Role::ImplementerMedium, row)].into(),
        brainstorm: None,
    };
    let mut models = RunModels::resolve(&table, None);
    let efforts = ModelEfforts {
        efforts: vec!["low".into(), "medium".into(), "high".into()],
        default: None,
    };
    models.efforts.insert(m(SONNET), efforts);
    let tables: Vec<String> = (tasks.iter())
        .map(|(id, owns, extra)| task_toml(id, "M", owns, extra))
        .collect();
    let text = plan_with(PROFILE, &tables);
    let config = config::Orchestrator::default();
    build_with_models(&text, &config, models, Vec::new()).unwrap_or_else(|e| panic!("{}", show(&e)))
}

fn failed(route: Route) -> AgentRound {
    let mut r = crate::run::orch::test_support::round(1, 0, Default::default());
    r.role = AgentRole::Worker;
    r.route = route;
    r.ended = true;
    r.environment_failed = true;
    r
}

#[test]
fn rung_2_climbs_the_rows_efforts_then_takes_its_fallback_then_stays() {
    let mut run = built(&[("t1", "[\"crates/a/**\"]", "")]);
    assert_eq!(run.tasks[0].route, at(SONNET, "medium"));
    assert_eq!(rung2_route(&run, 0), at(SONNET, "high"));
    run.tasks[0].route = at(SONNET, "high");
    assert_eq!(rung2_route(&run, 0), at(SOL, ""));
    // On the fallback, which reports no efforts: nothing left, the route stays.
    run.tasks[0].route = at(SOL, "");
    let next = rung2_route(&run, 0);
    assert_eq!(next, at(SOL, ""));
    assert_eq!(every_route_failed(&run, 0, &next), None);
}

#[test]
fn rung_2_steps_over_a_model_that_failed_in_the_task() {
    let mut run = built(&[("t1", "[\"crates/a/**\"]", "")]);
    run.tasks[0].rounds = vec![failed(at(SONNET, "medium"))];
    let next = rung2_route(&run, 0);
    assert_eq!(next, at(SOL, ""));
    assert_eq!(every_route_failed(&run, 0, &next), None);
    // Every model of the row failed: the original is retried, and the log says so.
    run.tasks[0].rounds.push(failed(at(SOL, "")));
    let next = rung2_route(&run, 0);
    assert_eq!(next, at(SONNET, "medium"));
    assert_eq!(
        every_route_failed(&run, 0, &next).as_deref(),
        Some("every route for task t1 failed in this task; retrying claude/claude-sonnet-5")
    );
}

#[test]
fn rung_2_keeps_to_installed_and_overlap_free_runtimes() {
    let mut run = built(&[("t1", "[\"crates/a/**\"]", "")]);
    run.tasks[0].route = at(SONNET, "high");
    run.orch.installed = [("claude".into(), true), ("codex".into(), false)].into();
    assert_eq!(rung2_route(&run, 0), at(SONNET, "high"));
    // Codex installed, but `t2`, unfinished on Claude, overlaps `t1`'s owns; ruling
    // FW-5: for a worker, even though `t2` waits on `t1`.
    let tasks = [
        ("t1", "[\"crates/a/**\"]", ""),
        ("t2", "[\"crates/a/src/**\"]", "deps = [\"t1\"]"),
    ];
    let mut run = built(&tasks);
    run.tasks[0].route = at(SONNET, "high");
    assert_eq!(rung2_route(&run, 0), at(SONNET, "high"));
    run.tasks[0].rounds = vec![failed(at(SONNET, "high"))];
    let next = rung2_route(&run, 0);
    assert_eq!(next, at(SONNET, "high"));
    assert_eq!(
        every_route_failed(&run, 0, &next).as_deref(),
        Some(
            "no route for task t1 keeps the overlap rule and has not failed in this task; \
             retrying claude/claude-sonnet-5"
        )
    );
    // `t2` merged: the overlap no longer holds `t1` to Claude.
    run.tasks[1].state = TaskState::Merged;
    assert_eq!(rung2_route(&run, 0), at(SOL, ""));
}

#[test]
fn the_test_writer_steps_along_its_own_row() {
    let mut run = built(&[("t1", "[\"crates/a/**\"]", "")]);
    set_row(&mut run, Role::TestWriter, SOL, Some("low"), Some(OPUS));
    let efforts = ModelEfforts {
        efforts: vec!["low".into(), "high".into()],
        default: None,
    };
    let models = run.limits.models.as_mut().expect("frozen");
    models.efforts.insert(m(SOL), efforts);
    assert_eq!(writer_step(&run, 0, &at(SOL, "low")), at(SOL, "high"));
    assert_eq!(writer_step(&run, 0, &at(SOL, "high")), at(OPUS, ""));
    // Opus reports no efforts: nothing left, the writer's route stays.
    assert_eq!(writer_step(&run, 0, &at(OPUS, "")), at(OPUS, ""));
}

#[test]
fn a_fix_task_steps_up_along_the_culprits_row() {
    let run = built(&[("t1", "[\"crates/a/**\"]", "")]);
    let up = escalate_for(&run, 0, &at(SONNET, "medium"), Mover::Worker);
    assert_eq!(up, at(SONNET, "high"));
    let up = escalate_for(&run, 0, &at(SONNET, "high"), Mover::Worker);
    assert_eq!(up, at(SOL, ""));
}
