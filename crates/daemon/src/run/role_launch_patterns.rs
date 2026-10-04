//! Milestone 9.5 decision 25: the test writer's `HeadlessSpec` (the racer's is task
//! M9.5.18's). A test writer is a worker in all but its contract, its role and its
//! route: same tools, sandbox, budget and checkout. Pure (design decision 1).

use proto::{AgentRole, Route};

use super::contract_patterns::TEST_WRITER_CONTRACT;
use super::model::{Run, Task};
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
