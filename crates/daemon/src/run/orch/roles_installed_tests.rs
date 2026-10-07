//! M9.17 fix round 3, milestone 9.8 (ruling F16): a sub-planner's and a run scout's
//! record are their rows' (`planner`, `research`): source `role_table`, policy
//! `m9.8-roles-v1`, the row's model then its fallback, a candidate on a runtime the
//! run's start found not installed recorded `not installed` (decision 43: a factual
//! reason). Their routes are the rows' whatever is installed (D2: never a model the
//! row does not name).

use proto::Runtime;
use proto::models::Role;

use super::*;
use crate::run::orch::test_support::{orchestrator, run_of, scout};
use crate::run::orch::{EpicRecord, PlannerPhase, RunScoutState};
use crate::run::test_support::set_row;

fn reasons(d: &RoleRoutingDecision) -> Vec<(Runtime, String, Option<String>)> {
    (d.candidates.iter())
        .map(|c| {
            let r = &c.route;
            (r.runtime, r.model.clone(), c.skipped_reason.clone())
        })
        .collect()
}

/// A run with an orchestrator, Claude not installed, rows `planner` = Opus falling back
/// to the Codex default and `research` = Haiku falling back to `gpt-6-luna`, one epic on
/// the planner row's route, and run scout `s1`.
fn run_on() -> Run {
    let mut run = run_of(1);
    run.orch.orchestrator = Some(orchestrator());
    run.orch.installed = [("claude".to_string(), false), ("codex".to_string(), true)].into();
    let opus = "claude:claude-opus-5-5";
    set_row(
        &mut run,
        Role::Planner,
        opus,
        Some("high"),
        Some("codex:default"),
    );
    let haiku = "claude:claude-haiku-4-5";
    set_row(
        &mut run,
        Role::Research,
        haiku,
        Some("low"),
        Some("codex:gpt-6-luna"),
    );
    let mut epic = EpicRecord::new("api", PlannerPhase::Planning);
    epic.route = crate::run::orch::launch::planner_route(&run).expect("an orchestrator");
    run.orch.epics.push(epic);
    let s1 = scout("s1", RunScoutState::Running, &["crates/api/**"]);
    run.orch.run_scouts.push(s1);
    run
}

#[test]
fn a_planners_record_is_its_row() {
    let run = run_on();
    let d = planner_record(&run, 0, 1, 100);
    assert_eq!(
        (d.source.as_str(), d.policy_version.as_str()),
        ("role_table", "m9.8-roles-v1")
    );
    let later = Some(EARLIER_TAKEN.to_string());
    assert_eq!(
        reasons(&d),
        [
            (Runtime::Claude, "claude-opus-5-5".into(), None),
            (Runtime::Codex, String::new(), later),
        ]
    );
    assert_eq!(
        d.selected_index, 0,
        "the row's route, whatever is installed"
    );
    assert_eq!(d.chosen.model, "claude-opus-5-5");
}

#[test]
fn a_run_scouts_record_is_the_research_row() {
    let run = run_on();
    let d = scout_record(&run, "s1", 100);
    assert_eq!(
        (d.source.as_str(), d.policy_version.as_str()),
        ("role_table", "m9.8-roles-v1")
    );
    let later = Some(EARLIER_TAKEN.to_string());
    assert_eq!(
        reasons(&d),
        [
            (Runtime::Claude, "claude-haiku-4-5".into(), None),
            (Runtime::Codex, "gpt-6-luna".into(), later),
        ]
    );
    // An unchosen candidate on a runtime the start found missing says so.
    let mut codexless = run.clone();
    codexless.orch.installed = [("codex".to_string(), false)].into();
    let d = scout_record(&codexless, "s1", 100);
    let not = Some(NOT_INSTALLED.to_string());
    assert_eq!(reasons(&d)[1], (Runtime::Codex, "gpt-6-luna".into(), not));
    assert_eq!(d.chosen, crate::run::orch::launch::scout_route(&run));
}
