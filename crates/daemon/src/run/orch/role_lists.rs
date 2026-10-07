//! The design flow's agents' picks (milestone 9.6 decision 10) and the orchestrator
//! record's sources. Milestone 9.8: the brainstormers are the run's `brainstorm` row and
//! the document reviewer the `reviewer` row's pick against the orchestrator (decision
//! 27); milestone 9.5's role lists no longer choose a route. Part of `roles.rs`. Pure.

use proto::models::ModelRef;
use proto::{Route, Runtime};

use crate::decider::DeciderCaps;
use crate::run::model::Run;
use crate::run::model_roles::RunModels;

/// An orchestrator record's source when the user chose its route (`--orchestrator`).
pub const EXPLICIT_SOURCE: &str = "explicit_choice";

/// An orchestrator record's source when a continued chain kept its route (ruling RH-5
/// ranks it on its own, below an explicit choice).
pub const CHAIN_SOURCE: &str = "continued_chain";

/// Ruling WB-A-W2: the halt when no installed runtime can run a brainstormer unsaved.
pub const UNSAVED_BRAINSTORMER: &str =
    "design flow: no installed runtime can run a brainstormer without saving its session";

/// Ruling WB-A-W2: the halt when no installed runtime can run a document reviewer
/// unsaved.
pub const UNSAVED_REVIEWER: &str =
    "design flow: no installed runtime can run a document reviewer without saving its session";

/// Milestone 9.6 decision 10: one brainstormer's label and route.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BrainstormPick {
    /// `claude` or `codex` on two runtimes, the lens `A` or `B` on one.
    pub label: String,
    pub route: Route,
    /// The run's `brainstorm` list chose the route (milestone 9.5); never since
    /// milestone 9.8, whose row chooses.
    pub listed: bool,
}

/// Decision 10, milestone 9.8 (MR §3.1): the two brainstormers' picks, the run's
/// `brainstorm` row ([`brainstorm_picks_with`] by the decider's caps). Empty when no
/// installed runtime can run a brainstormer unsaved (ruling WB-A-W2).
pub fn brainstorm_picks(run: &Run) -> Vec<BrainstormPick> {
    brainstorm_picks_with(run, &crate::decider::caps()).unwrap_or_default()
}

/// Ruling T8-4: whether `runtime`'s CLI runs a session without saving it (`claude
/// --no-session-persistence`, `codex exec --ephemeral`), by the decider's caps.
pub fn runs_unsaved(runtime: Runtime, caps: &DeciderCaps) -> bool {
    match runtime {
        Runtime::Claude => caps.claude_no_session_persistence,
        Runtime::Codex => caps.codex_ephemeral,
        Runtime::Shell => false,
    }
}

/// Ruling T8-4: the installed runtimes of the `brainstorm` row that cannot brainstorm,
/// their CLI unable to run without saving the session; their picks fall back to the
/// row's other model ([`brainstorm_picks_with`]).
pub fn unsaved_missing(run: &Run, caps: &DeciderCaps) -> Vec<Runtime> {
    let installed = |runtime: Runtime| run.orch.installed.get(runtime.label()) != Some(&false);
    let row = &run.limits.models().brainstorm;
    let in_row = |runtime: Runtime| row.first.runtime == runtime || row.second.runtime == runtime;
    [Runtime::Claude, Runtime::Codex]
        .into_iter()
        .filter(|&runtime| in_row(runtime) && installed(runtime) && !runs_unsaved(runtime, caps))
        .collect()
}

/// [`brainstorm_picks`] by `caps`: the `brainstorm` row's two models at its effort. A
/// model whose runtime the start found missing, or whose CLI cannot run without saving
/// the session (ruling T8-4), is replaced by the row's other model; `None` when neither
/// can run (ruling WB-A-W2: there is no fallback to one that saves its session). Labels:
/// the runtimes' names on two runtimes, the lenses `A` and `B` on one.
pub fn brainstorm_picks_with(run: &Run, caps: &DeciderCaps) -> Option<Vec<BrainstormPick>> {
    let usable = |runtime: Runtime| {
        run.orch.installed.get(runtime.label()) != Some(&false) && runs_unsaved(runtime, caps)
    };
    let row = &run.limits.models().brainstorm;
    let route = |m: &ModelRef| RunModels::route_of(m, row.effort.as_deref());
    let (first, second) = (route(&row.first), route(&row.second));
    let (a, b) = match (usable(first.runtime), usable(second.runtime)) {
        (true, true) => (first, second),
        (true, false) => (first.clone(), first),
        (false, true) => (second.clone(), second),
        (false, false) => return None,
    };
    let labels = match a.runtime == b.runtime {
        true => ("A".to_string(), "B".to_string()),
        false => (a.runtime.label().to_string(), b.runtime.label().to_string()),
    };
    let pick = |label, route| BrainstormPick {
        label,
        route,
        listed: false,
    };
    Some(vec![pick(labels.0, a), pick(labels.1, b)])
}

/// Milestone 9.6 decision 10 (task M9.6.10, DF §4.2), milestone 9.8 (decision 27): the
/// document reviewer's route, the `reviewer` row's pick against the orchestrator's
/// route (its author), and, when it is instead the orchestrator's own route, why (fix
/// round 1, m1): the pick's runtime is not installed, or its CLI cannot run a session
/// without saving it (ruling T8-4). `None` when the orchestrator's own runtime cannot
/// run one unsaved either: there is no fallback to a runtime that saves its session
/// (ruling WB-A-W2, ruling WB-B m4).
pub fn review_pick(run: &Run, caps: &DeciderCaps) -> Option<(Route, Option<String>)> {
    let own = (run.orch.orchestrator.as_ref()).map_or_else(
        || crate::run::orch::launch::scout_route(run),
        |o| o.route.clone(),
    );
    let (pick, _) = run.limits.models().reviewer_route(&own);
    let name = pick.runtime.label();
    let installed = &run.orch.installed;
    let why = if installed.get(name) == Some(&false) {
        format!("the {name} runtime is not installed")
    } else if !runs_unsaved(pick.runtime, caps) {
        format!("the {name} CLI cannot run a session without saving it")
    } else {
        return Some((pick, None));
    };
    let own_installed = installed.get(own.runtime.label()) != Some(&false);
    (own_installed && runs_unsaved(own.runtime, caps)).then_some((own, Some(why)))
}

#[cfg(test)]
#[path = "role_lists_tests.rs"]
mod tests;
