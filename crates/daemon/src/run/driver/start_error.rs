//! Milestone 9.8 (MR §7): a session whose program is not found fails its launch with
//! the role and the way out, `reviewer: codex not found; choose another model in C-b
//! S`, not the bare spawn error: the role table chose the runtime, so the table is
//! where the user changes it. Any other start error is recorded as before.

use proto::models::Role;
use proto::{AgentRole, RunRef, Runtime};

use super::RunService;
use crate::run::model::Run;
use crate::run::model_roles::RunModels;

/// The text a failed session start records: [`not_found`]'s when the spawn found no
/// program, else `error` as before.
pub(super) fn named(
    service: &RunService,
    (run_ref, runtime): &(Option<RunRef>, Runtime),
    error: &anyhow::Error,
) -> String {
    let Some(run_ref) = run_ref.as_ref().filter(|_| missing(error)) else {
        return error.to_string();
    };
    // A short read of the engine's state; nothing else is done under the lock.
    let role = {
        let state = crate::lock(&service.state);
        role_label(state.runs.get(&run_ref.run_id), run_ref)
    };
    not_found(&role, *runtime)
}

/// [`named`] for a session whose row's label the caller knows (`research` for a run
/// scout, `planner`, `brainstorm`).
pub(super) fn named_as(role: &str, runtime: Runtime, error: &anyhow::Error) -> String {
    match missing(error) {
        true => not_found(role, runtime),
        false => error.to_string(),
    }
}

/// Whether the spawn found no program (`ENOENT` anywhere in the error's chain).
fn missing(error: &anyhow::Error) -> bool {
    error.chain().any(|e| {
        e.downcast_ref::<std::io::Error>()
            .is_some_and(|io| io.kind() == std::io::ErrorKind::NotFound)
    })
}

/// MR §7's text, exactly.
pub(super) fn not_found(role: &str, runtime: Runtime) -> String {
    format!(
        "{role}: {} not found; choose another model in C-b S",
        runtime.label()
    )
}

/// The role-table row whose session `run_ref` is (`Role::label`): a worker's or a
/// racer's is its task's row (decision 10), a research task's scout `research`, a
/// brainstormer's `brainstorm`.
pub(super) fn role_label(run: Option<&Run>, run_ref: &RunRef) -> String {
    let task = || {
        let id = run_ref.task_id.as_deref()?;
        run?.tasks.iter().find(|t| t.id() == id)
    };
    let role = match run_ref.role {
        AgentRole::Orchestrator => Role::Orchestrator,
        AgentRole::Planner => Role::Planner,
        AgentRole::Scout => Role::Research,
        AgentRole::Reviewer | AgentRole::DocReviewer => Role::Reviewer,
        AgentRole::TestWriter => Role::TestWriter,
        AgentRole::Decider => Role::Helpers,
        AgentRole::Brainstormer => return "brainstorm".to_string(),
        AgentRole::Worker | AgentRole::Racer => {
            task().map_or(Role::ImplementerMedium, RunModels::task_role)
        }
    };
    role.label()
}

#[cfg(test)]
#[path = "start_error_tests.rs"]
mod tests;
