//! Milestone 9.5 decisions 19 and 25: the racer's and the test writer's `HeadlessSpec`.
//! A test writer is a worker in all but its contract, its role and its route: same
//! tools, sandbox, budget and checkout. A racer is a worker in all but its role, its
//! lane and its checkout, the lane's (its prompt is task M9.5.18's). Pure (design
//! decision 1).

use proto::{AgentRole, Route};

use super::contract_patterns::TEST_WRITER_CONTRACT;
use super::model::{Lane, Run, Task};
use super::role_launch::worker_spec;
use crate::headless::HeadlessSpec;

/// Task `task`'s test writer on `route`, in the task's own checkout, with the task's
/// current session number.
pub fn test_writer_spec(run: &Run, task: &Task, route: &Route) -> HeadlessSpec {
    let mut on_route = task.clone();
    on_route.route = route.clone();
    let mut spec = worker_spec(run, &on_route);
    spec.instructions = TEST_WRITER_CONTRACT.to_string();
    if let Some(mcp) = spec.mcp.as_mut() {
        mcp.role = AgentRole::TestWriter;
    }
    if let Some(run_ref) = spec.run_ref.as_mut() {
        run_ref.role = AgentRole::TestWriter;
    }
    spec
}

/// Decision 19: lane `lane`'s racer of task `task`, a worker's session in the lane's
/// checkout (`<task>.<lane>`: its `cwd`, `TMPDIR`, sandbox roots and filter log) on the
/// lane's route, with role `Racer` and the lane on its MCP target and run reference.
pub fn racer_spec(run: &Run, task: &Task, lane: &Lane) -> HeadlessSpec {
    let mut on_lane = task.clone();
    on_lane.worktree = run.task_path(&lane.checkout);
    on_lane.route = lane.route.clone();
    on_lane.lane_view = Some(lane.lane);
    let mut spec = worker_spec(run, &on_lane);
    if let Some(mcp) = spec.mcp.as_mut() {
        mcp.role = AgentRole::Racer;
        mcp.lane = Some(lane.lane);
    }
    if let Some(run_ref) = spec.run_ref.as_mut() {
        run_ref.role = AgentRole::Racer;
        run_ref.lane = Some(lane.lane);
    }
    spec
}
