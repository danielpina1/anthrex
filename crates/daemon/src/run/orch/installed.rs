//! Decision 26's start check that the planning agents' runtimes are installed (M9.17
//! fix round; the decision cited a "binary present" check the code did not have). Pure:
//! the driver stats the binaries (`driver/build.rs::installed`) and passes what it found.

use proto::{ModelEntry, OrchestratorChoice, Route, RoutingCandidate, Runtime};

use super::launch::{Resolved, resolve_orchestrator};
use super::roles::NOT_INSTALLED;
use crate::run::roster::peer;

/// A runtime's configured binary when that binary is not installed, `None` when it is.
pub type Missing<'a> = &'a dyn Fn(Runtime) -> Option<String>;

/// Decision 6's resolution over the installed runtimes only. The runtime decision 6
/// picks is kept when it is installed. When it is not, and nothing pinned it (no
/// `--orchestrator` choice, no `[orchestrator.agent]` runtime or model), the peer runtime
/// is resolved instead, and the skipped runtime's roster entries lead the candidate
/// snapshot, each `not installed` (decision 43: a factual reason, never a failure).
/// Otherwise the start is refused, naming the runtime and its binary.
pub fn resolve_installed(
    choice: Option<&OrchestratorChoice>,
    agent: &config::AgentConfig,
    default_runtime: Runtime,
    roster: &[ModelEntry],
    missing: Missing<'_>,
) -> Result<Resolved, String> {
    let runtime = choice
        .map(|c| c.runtime)
        .or(agent.runtime)
        .unwrap_or(default_runtime);
    let refusal = || {
        not_installed(
            "the orchestrator's",
            runtime,
            missing,
            "choose another runtime with --orchestrator",
        )
    };
    let Some(refused) = refusal() else {
        return resolve_orchestrator(choice, agent, default_runtime, roster);
    };
    let pinned = choice.is_some() || agent.runtime.is_some() || !agent.model.is_empty();
    let peer = peer(runtime);
    if pinned || missing(peer).is_some() {
        return Err(refused);
    }
    let mut resolved = resolve_orchestrator(None, agent, peer, roster)?;
    let mut candidates: Vec<RoutingCandidate> = roster
        .iter()
        .filter(|e| e.runtime == runtime)
        .map(|e| RoutingCandidate {
            route: Route {
                runtime: e.runtime,
                model: e.model.clone(),
                strength: e.strength,
                effort: agent.effort,
            },
            skipped_reason: Some(NOT_INSTALLED.to_string()),
        })
        .collect();
    if candidates.is_empty() {
        // Item 6: a skipped runtime with no roster entry is still recorded, as the one
        // candidate it would have had, the CLI's default model.
        candidates.push(RoutingCandidate {
            route: Route {
                runtime,
                model: String::new(),
                strength: proto::Strength::Standard,
                effort: agent.effort,
            },
            skipped_reason: Some(NOT_INSTALLED.to_string()),
        });
    }
    candidates.append(&mut resolved.candidates);
    resolved.candidates = candidates;
    Ok(resolved)
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
