//! Milestone 9.5 decision 9a for a run's roles (task M9.5.10b): the run scouts' and the
//! sub-planners' picks from the run's frozen `scout` and `planner` lists, rotating per
//! run in start order; the orchestrator's below an explicit choice (rulings RH-5,
//! RL-3); and how a role record made over a list says so. Part of `roles.rs`. Pure.

use proto::{OrchestratorChoice, RoleRoutingDecision, Route, RoutingCandidate};

use crate::run::model::{FrozenList, ListPolicy, Run};
use crate::run::orch::launch::{Resolved, scout_routing};
use crate::run::route_pick::{Installed, LIST_POLICY, RolePick, role};

/// A role record's source when the role's list chose its route.
pub const LIST_SOURCE: &str = "configured_list";

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

#[cfg(test)]
#[path = "role_lists_tests.rs"]
mod tests;
