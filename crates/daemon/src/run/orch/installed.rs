//! Decision 26's start check that the planning agents' runtimes are installed (M9.17
//! fix round; the decision cited a "binary present" check the code did not have). Pure:
//! the driver stats the binaries (`driver/build.rs::installed`) and passes what it found.

use proto::{OrchestratorChoice, Runtime};

use super::launch::{Resolved, orchestrator_route};
use crate::run::model::Run;
use crate::run::model_roles::RunModels;

/// The orchestrator's missing-runtime refusal's way out (MR §7; fix round 1, M1): the
/// table that chose the runtime, or a per-run flag.
pub const ORCHESTRATOR_HINT: &str =
    "choose another model for the orchestrator in C-b S, or another runtime with --orchestrator";

/// A runtime's configured binary when that binary is not installed, `None` when it is.
pub type Missing<'a> = &'a dyn Fn(Runtime) -> Option<String>;

/// The orchestrator's route ([`orchestrator_route`]) when its runtime is installed.
/// Milestone 9.8 (D2, MR §7): no other model is ever taken in its place; a runtime that
/// is not installed refuses the start, naming the runtime and its binary.
pub fn resolve_installed(
    choice: Option<&OrchestratorChoice>,
    models: &RunModels,
    missing: Missing<'_>,
) -> Result<Resolved, String> {
    let resolved = orchestrator_route(choice, models);
    let runtime = resolved.route.runtime;
    match not_installed("the orchestrator's", runtime, missing, ORCHESTRATOR_HINT) {
        Some(refused) => Err(refused),
        None => Ok(resolved),
    }
}

/// Milestone 9.5 rulings RH-5 and RL-3, milestone 9.8: [`resolve_installed`] for
/// planned `run` from its frozen role table, below an explicit `choice`, over `missing`
/// (what the orchestrator's window finds installed).
pub fn resolve_planned(
    run: &Run,
    choice: Option<&OrchestratorChoice>,
    missing: Missing<'_>,
) -> Result<Resolved, String> {
    resolve_installed(choice, run.limits.models(), missing)
}

/// Rulings RH-5 and RL-3, milestone 9.8: decision 29's route for `run`'s promotion
/// ([`orchestrator_route`] on the run's frozen table, no fallback), below an explicit
/// `choice`.
pub fn resolve_promoted(run: &Run, choice: Option<&OrchestratorChoice>) -> Resolved {
    orchestrator_route(choice, run.limits.models())
}

/// The start's refusal when `who`'s `runtime` is not installed; `hint` says what else
/// the user may do.
pub fn not_installed(
    who: &str,
    runtime: Runtime,
    missing: Missing<'_>,
    hint: &str,
) -> Option<String> {
    let bin = missing(runtime)?;
    Some(format!(
        "{who} runtime {} is not installed ({bin} is not an executable file); install it, or {hint}",
        runtime.label()
    ))
}

#[cfg(test)]
#[path = "installed_tests.rs"]
mod tests;
