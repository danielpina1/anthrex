//! Milestone 9.16 task M9.16.5, the readers' half (decision 11, rulings C4 and C6):
//! sub-planners, run scouts and research tasks wait on a braked runtime, inside the
//! orchestrator tool handlers' own scheduler passes too, and a refused first candidate
//! never spins the reader loop.

use std::sync::mpsc;
use std::time::Duration;

use proto::brakes::QuotaWindow;
use proto::models::{ModelRef, Role, RoleChoice};
use proto::{Runtime, TaskState};

use super::brakes::{attention, claude_held, reading, trip_claude};
use super::fixture::*;
use super::orch::{answer, launched};
use super::planners::spawn;
use super::run_scouts::scout;
use crate::run::orch::{PlannerPhase, RunScoutState};

/// The hang guard of [`a_refused_first_candidate_does_not_spin`]: one pure reducer step
/// takes milliseconds; a loop that spins never returns (docs/timing-budgets.md).
const STEP_WAIT: Duration = Duration::from_secs(10);

/// [`launched`], planning, its `research` row (run scouts) on Codex.
fn planning_with_codex_scouts() -> Fixture {
    let mut fx = launched(false);
    let models = fx
        .run_mut()
        .limits
        .models
        .as_mut()
        .expect("frozen at start");
    let codex = RoleChoice {
        model: ModelRef::parse("codex:default").expect("a model"),
        effort: None,
        fallback: None,
    };
    models.rows.insert(Role::Research, codex);
    fx
}

fn starts(fx: &Fixture, name: &str) -> usize {
    ops_in(&fx.log, name).len()
}

#[test]
fn planners_design_agents_and_scouts_skip_a_braked_runtime() {
    let mut fx = planning_with_codex_scouts();
    let until = trip_claude(&mut fx);
    // The sub-planner (the `planner` row: Claude) is queued by the tool's own pass.
    assert_eq!(spawn(&mut fx, "mail")["state"], "queued");
    let epic = &fx.run().orch.epics[0];
    assert_eq!(
        (epic.phase.clone(), epic.route.runtime),
        (PlannerPhase::Queued, Runtime::Claude)
    );
    assert_eq!(starts(&fx, "StartPlanner"), 0);
    assert!(attention(&fx).contains(&claude_held(until)));
    // The Codex scout behind it starts.
    let (ok, value) = answer(&scout(&mut fx, "s1"));
    assert!(ok, "{value}");
    assert_eq!(value["state"], "starting");
    assert_eq!(starts(&fx, "StartScout"), 1);
    assert_eq!(fx.run().orch.epics[0].phase, PlannerPhase::Queued);
    // A lower reading of the window, newer than the trip, releases it.
    fx.next(reading(Runtime::Claude, QuotaWindow::SevenDay, 500, until));
    assert_eq!(fx.run().orch.epics[0].phase, PlannerPhase::Planning);
    assert_eq!(starts(&fx, "StartPlanner"), 1);
}

#[test]
fn a_refused_first_candidate_does_not_spin() {
    let mut fx = planning_with_codex_scouts();
    trip_claude(&mut fx);
    let (send, recv) = mpsc::channel();
    // Both tool calls run their handler's scheduler pass with a braked queued
    // sub-planner first in line; a loop that re-picks it never returns.
    std::thread::spawn(move || {
        spawn(&mut fx, "mail");
        let effects = scout(&mut fx, "s1");
        let _ = send.send((fx, effects));
    });
    let (fx, effects) = recv
        .recv_timeout(STEP_WAIT)
        .expect("the step returned: the reader loop does not spin");
    assert_eq!(answer(&effects).1["state"], "starting");
    let run = fx.run();
    assert_eq!(run.orch.epics[0].phase, PlannerPhase::Queued);
    assert_eq!(run.orch.run_scouts[0].state, RunScoutState::Running);
}

#[test]
fn a_research_task_on_a_braked_runtime_stays_queued() {
    // The `research` row is Claude (Haiku) by default.
    let plan = plan_with(PROFILE, &[super::kinds::research("r1", "")]);
    let mut fx = Fixture::new(&plan);
    let until = trip_claude(&mut fx);
    fx.ready(true);
    fx.tick();
    assert_eq!(fx.task("r1").state, TaskState::Queued);
    assert_eq!(starts(&fx, "CreateWindow"), 0);
    assert!(attention(&fx).contains(&claude_held(until)));
    // The window resets: its session starts.
    fx.send(until, crate::run::engine::EventKind::Tick);
    assert_eq!(starts(&fx, "CreateWindow"), 1);
    assert_eq!(fx.task("r1").state, TaskState::Working);
}
