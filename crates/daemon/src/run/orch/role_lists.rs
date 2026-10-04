//! Milestone 9.5 decision 9a for a run's roles (task M9.5.10b): the run scouts' and the
//! sub-planners' picks from the run's frozen `scout` and `planner` lists, rotating per
//! run in start order; the orchestrator's below an explicit choice (rulings RH-5,
//! RL-3); and how a role record made over a list says so. Part of `roles.rs`. Pure.

use proto::{Effort, OrchestratorChoice, RoleRoutingDecision, Route, RoutingCandidate, Runtime};

use crate::decider::{DECIDER_CAPS, DeciderCaps};
use crate::run::model::{FrozenList, ListPolicy, Run};
use crate::run::orch::launch::{Resolved, scout_routing};
use crate::run::roster::strongest_of;
use crate::run::route_pick::{Installed, LIST_POLICY, RolePick, role};

/// A role record's source when the role's list chose its route.
pub const LIST_SOURCE: &str = "configured_list";

/// An orchestrator record's source when the user chose its route (`--orchestrator`).
pub const EXPLICIT_SOURCE: &str = "explicit_choice";

/// An orchestrator record's source when a continued chain kept its route (ruling RH-5
/// ranks it on its own, below an explicit choice).
pub const CHAIN_SOURCE: &str = "continued_chain";

/// Run scout `scout_id`'s pick from the run's `scout` list: its rotation is its place
/// among the run's scouts (start order), at `[orchestrator.scouts] effort` where a
/// candidate names none, over the run's installed runtimes. `None` with no list.
pub fn scout_pick(run: &Run, scout_id: &str) -> Option<RolePick> {
    let scouts = &run.orch.run_scouts;
    let rotation = (scouts.iter().position(|s| s.id == scout_id)).unwrap_or(scouts.len());
    let (list, effort) = (&run.limits.route_lists.scout, scout_routing(run).effort);
    role(list, rotation as u32, effort, &run.orch.installed)
}

/// Epic `k`'s sub-planner pick from the run's `planner` list (rotation `k`), at
/// `[orchestrator.planners] effort` where a candidate names none.
pub fn planner_pick(run: &Run, k: usize) -> Option<RolePick> {
    let (list, effort) = (
        &run.limits.route_lists.planner,
        run.limits.orch.planners.effort,
    );
    role(list, k as u32, effort, &run.orch.installed)
}

/// Rulings RH-5 and RL-3: the orchestrator's route, highest first: an explicit
/// `choice` (the goal form, `run promote --orchestrator`, and a continued chain, whose
/// route its continuation passes as the choice); the `orchestrator` list's first
/// candidate `installed` (the window's map at a start) does not rule out; `today`
/// (decision 6's resolution from `[orchestrator.agent]`). Skipped list candidates lead
/// `today`'s snapshot.
pub fn orchestrator(
    choice: Option<&OrchestratorChoice>,
    list: &FrozenList,
    effort: proto::Effort,
    installed: &Installed,
    today: impl FnOnce() -> Result<Resolved, String>,
) -> Result<Resolved, String> {
    let pick = role(list, 0, effort, installed).filter(|_| choice.is_none());
    match pick {
        None => today(),
        Some(RolePick {
            route: Some(route),
            candidates,
            ..
        }) => Ok(Resolved {
            route,
            source: LIST_SOURCE.to_string(),
            candidates,
        }),
        Some(pick) => {
            let mut resolved = today()?;
            resolved.candidates = candidates(&pick, resolved.candidates);
            Ok(resolved)
        }
    }
}

/// The candidates of a role record made over `pick`: the list's snapshot, followed by
/// `today`'s when every candidate was skipped.
pub fn candidates(pick: &RolePick, today: Vec<RoutingCandidate>) -> Vec<RoutingCandidate> {
    let mut out = pick.candidates.clone();
    if pick.route.is_none() {
        out.extend(today);
    }
    out
}

/// Marks a role record made over a list: its `pick_policy`, a `spread` list's rotation
/// and, when the list chose the record's route, source [`LIST_SOURCE`] and policy
/// `m9.5-list-v1`.
pub fn mark(decision: &mut RoleRoutingDecision, pick: (ListPolicy, u32), chosen: Option<&Route>) {
    let (policy, rotation) = pick;
    decision.pick_policy = Some(policy.label().to_string());
    decision.rotation = (policy == ListPolicy::Spread).then_some(rotation);
    if chosen == Some(&decision.chosen) {
        decision.source = LIST_SOURCE.to_string();
        decision.policy_version = LIST_POLICY.to_string();
    }
}

/// Milestone 9.6 decision 10: one brainstormer's label and route.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BrainstormPick {
    /// `claude` or `codex` on two runtimes, the lens `A` or `B` on one.
    pub label: String,
    pub route: Route,
    /// The run's `brainstorm` list chose the route.
    pub listed: bool,
}

/// Decision 10: the two brainstormers' picks. With a `brainstorm` list: its first two
/// entries on different runtimes, else its first two (one usable entry runs twice); a
/// candidate on a runtime the run's start found missing is passed over, as every
/// role's list does. Without one (or with none usable): the strongest model of each
/// installed runtime (`roster::strongest_of`), Claude's first; with one runtime, its
/// strongest twice. A candidate without an effort, and every default, takes the
/// orchestrator's effort (no `[orchestrator.design]` key names one). Labels: the
/// runtimes' names on two runtimes, the lenses `A` and `B` on one.
pub fn brainstorm_picks(run: &Run) -> Vec<BrainstormPick> {
    brainstorm_picks_with(run, &DECIDER_CAPS)
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

/// Ruling T8-4: the installed runtimes that cannot brainstorm, their CLI unable to run
/// without saving the session; their picks fall back as an uninstalled runtime's.
pub fn unsaved_missing(run: &Run, caps: &DeciderCaps) -> Vec<Runtime> {
    let installed = |runtime: Runtime| run.orch.installed.get(runtime.label()) != Some(&false);
    [Runtime::Claude, Runtime::Codex]
        .into_iter()
        .filter(|&runtime| installed(runtime) && !runs_unsaved(runtime, caps))
        .collect()
}

/// [`brainstorm_picks`] by `caps` (ruling T8-4).
pub fn brainstorm_picks_with(run: &Run, caps: &DeciderCaps) -> Vec<BrainstormPick> {
    let effort = (run.orch.orchestrator.as_ref()).map_or(Effort::High, |o| o.route.effort);
    let usable = |runtime: Runtime| {
        run.orch.installed.get(runtime.label()) != Some(&false) && runs_unsaved(runtime, caps)
    };
    let list = &run.limits.route_lists.brainstorm;
    let listed: Vec<Route> = (list.candidates.iter())
        .filter(|c| usable(c.runtime))
        .map(|c| c.route(effort))
        .collect();
    let (routes, from_list) = match listed.first() {
        Some(first) => {
            let other = listed.iter().skip(1).find(|r| r.runtime != first.runtime);
            let second = other.or(listed.get(1)).unwrap_or(first);
            ((first.clone(), second.clone()), true)
        }
        None => (default_pair(run, effort, &usable), false),
    };
    let (a, b) = routes;
    let labels = match a.runtime == b.runtime {
        true => ("A".to_string(), "B".to_string()),
        false => (a.runtime.label().to_string(), b.runtime.label().to_string()),
    };
    vec![
        BrainstormPick {
            label: labels.0,
            route: a,
            listed: from_list,
        },
        BrainstormPick {
            label: labels.1,
            route: b,
            listed: from_list,
        },
    ]
}

/// Decision 10's defaults: the strongest model of each installed runtime, else (no
/// roster entry on any) the orchestrator's route twice.
fn default_pair(run: &Run, effort: Effort, usable: &dyn Fn(Runtime) -> bool) -> (Route, Route) {
    let strongest: Vec<Route> = [Runtime::Claude, Runtime::Codex]
        .into_iter()
        .filter(|&runtime| usable(runtime))
        .filter_map(|runtime| strongest_of(&run.roster, runtime))
        .map(|e| Route {
            runtime: e.runtime,
            model: e.model.clone(),
            strength: e.strength,
            effort,
        })
        .collect();
    match &strongest[..] {
        [a, b, ..] => (a.clone(), b.clone()),
        [a] => (a.clone(), a.clone()),
        [] => {
            let route = (run.orch.orchestrator.as_ref()).map_or_else(
                || crate::run::orch::launch::frozen_scout_route(run),
                |o| o.route.clone(),
            );
            (route.clone(), route)
        }
    }
}

#[cfg(test)]
#[path = "role_lists_tests.rs"]
mod tests;
