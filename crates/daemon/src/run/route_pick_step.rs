//! Rung 2's and `run retry`'s step for a task (milestone 9.8 decision 29, rulings
//! RL-1, T10a-6): the role table's escalation (`role_step::escalate`), skipping a route
//! that failed in this task. Milestone 9.5's list step ([`next_candidate`]) is kept for
//! its tests until M9.8.13; nothing on a route-changing rung calls it. Pure (design
//! decision 1).

use proto::{Route, RoutingCandidate};

use super::*;

/// Decision 9a's rung 2 (and `run retry`'s) for task `i` whose list has two or more
/// candidates: the next unskipped candidate after the task's current one, cycling. The
/// candidates' routes keep the plan's effort ([`route_for`], as [`pick`] gave them), and
/// the current one is found by its exact route, else by runtime and model (ruling
/// T10a-1). Skipped: a candidate identical to the current route, one not installed, one
/// that failed in this task (RL-1), one on the other runtime while an unfinished task on
/// the current runtime overlaps the task's `owns`, and one below the current route —
/// unless the current route failed in this task, when the step is a substitution and
/// any strength will do (ruling T10a-3). `None` (a list of one, a current route the
/// list does not hold, or nothing left) leaves rung 2 to the roster ([`rung2_route`]).
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
    let held = alongside(tasks, i, Mover::Worker)
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
