//! Rung 2's and `run retry`'s step for a task (decision 9a, rulings RL-1, T10a-1,
//! T10a-3): the list's next candidate, else the roster's escalation, each skipping a
//! route that failed in this task. Pure (design decision 1).

use proto::{Route, RoutingCandidate};

use super::*;
use crate::run::roster::escalate_skipping;

/// Decision 9a's rung 2 (and `run retry`'s) for task `i` whose list has two or more
/// candidates: the next unskipped candidate after the task's current one, cycling. The
/// candidates' routes keep the plan's effort ([`route_for`], as [`pick`] gave them), and
/// the current one is found by its exact route, else by runtime and model (ruling
/// T10a-1). Skipped: a candidate identical to the current route, one not installed, one
/// that failed in this task (RL-1), one on the other runtime while an unfinished task on
/// the current runtime overlaps the task's `owns`, and one below the current route —
/// unless the current route failed in this task, when the step is a substitution and
/// any strength will do (ruling T10a-3). `None` (a list of one, a current route the list does not hold, or nothing left)
/// leaves rung 2 to `roster::escalate`.
pub fn next_candidate(
    limits: &RunLimits,
    tasks: &[Task],
    i: usize,
    installed: &Installed,
) -> Option<(Route, ListPick)> {
    let task = &tasks[i];
    let list = task_list(&limits.route_lists, task);
    if list.candidates.len() < 2 {
        return None;
    }
    let current = &task.route;
    let failed = failed_routes(task);
    let substitute = failed_in(&failed, current);
    let routes: Vec<Route> = (0..list.candidates.len())
        .map(|k| route_for(limits, task, list, k))
        .collect();
    let same = |r: &Route| r.runtime == current.runtime && r.model == current.model;
    let at = (routes.iter().position(|r| r == current)).or_else(|| routes.iter().position(same))?;
    let held = (tasks.iter().enumerate())
        .filter(|(j, t)| *j != i && !t.state.is_finished())
        .any(|(_, t)| t.route.runtime == current.runtime && overlap(t, task));
    let reasons: Vec<Option<&str>> = (routes.iter())
        .map(|route| {
            if route == current {
                Some(CURRENT_ROUTE)
            } else if missing(installed, route.runtime) {
                Some(NOT_INSTALLED)
            } else if failed_in(&failed, route) {
                Some(FAILED_IN_TASK)
            } else if route.runtime != current.runtime && held {
                Some(OVERLAPPING_OWNS)
            } else if !substitute && below(route, current) {
                Some(BELOW_CURRENT)
            } else {
                None
            }
        })
        .collect();
    let n = routes.len();
    let k = (1..n)
        .map(|d| (at + d) % n)
        .find(|&k| reasons[k].is_none())?;
    let candidates = (routes.iter().zip(&reasons))
        .map(|(route, reason)| RoutingCandidate {
            route: route.clone(),
            skipped_reason: reason.map(String::from),
        })
        .collect();
    let slot = task.list_pick.as_ref().and_then(|p| p.slot);
    let step = ListPick {
        candidates,
        chosen: Some(k as u32),
        pick: policy(list, task),
        slot,
    };
    Some((routes[k].clone(), step))
}

/// Ruling T10a-1: `to` is a weaker strength than `from`, or a lower effort at the same.
fn below(to: &Route, from: &Route) -> bool {
    to.strength < from.strength || (to.strength == from.strength && to.effort < from.effort)
}

/// The route rung 2 and `run retry` give task `i` (decision 9a, ruling RL-1): its
/// list's [`next_candidate`], else `roster::escalate`, each skipping a route that failed
/// in this task; with the list's step for the routing history. Ruling T10a-3: when
/// nothing is left but a failed route, the task's own route is retried
/// ([`every_route_failed`] says so).
pub fn rung2_route(run: &Run, i: usize) -> (Route, Option<ListPick>) {
    let (task, installed) = (&run.tasks[i], &run.orch.installed);
    if let Some((route, step)) = next_candidate(&run.limits, &run.tasks, i, installed) {
        return (route, Some(step));
    }
    let failed = failed_routes(task);
    let route = escalate_skipping(&run.roster, &task.route, &failed);
    if failed_in(&failed, &route) {
        return (task.route.clone(), None);
    }
    (route, None)
}

/// Ruling T10a-3: the run-log line when task `i`'s next route, `route` (from
/// [`rung2_route`]), failed in this task too: every route did, and the original is
/// retried.
pub fn every_route_failed(run: &Run, i: usize, route: &Route) -> Option<String> {
    let task = &run.tasks[i];
    failed_in(&failed_routes(task), route).then(|| {
        format!(
            "every route for task {} failed in this task; retrying {}/{}",
            task.id(),
            route.runtime.label(),
            route.model
        )
    })
}
