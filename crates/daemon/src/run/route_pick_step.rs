//! Rung 2's and `run retry`'s step for a task (milestone 9.8 decision 29, rulings
//! RL-1, T10a-6): the role table's escalation (`role_step::escalate`), skipping a route
//! that failed in this task. Milestone 9.5's list step ([`next_candidate`]) is kept for
//! its tests until M9.8.13; nothing on a route-changing rung calls it. Pure (design
//! decision 1).

use proto::models::Role;
use proto::{Route, RoutingCandidate};

use super::*;
use crate::run::model_roles::{RunModels, runtime_open};
use crate::run::role_step;

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

/// The route rung 2 and `run retry` give task `i` (milestone 9.8 decision 29, ruling
/// RL-1): `role_step::escalate` along its row ([`RunModels::task_role`]) from its route,
/// stepping over a route that failed in this task and a runtime [`runtime_open`] holds
/// the worker off (ruling T10a-6); with nothing left, the task's own route
/// ([`every_route_failed`] says so when it failed). No list step since M9.8.8: the
/// `ListPick` is always `None` (the list code goes in M9.8.13).
pub fn rung2_route(run: &Run, i: usize) -> (Route, Option<ListPick>) {
    let task = &run.tasks[i];
    let role = RunModels::task_role(task);
    let open = |runtime| runtime_open(run, i, runtime, Mover::Worker);
    let next = step(run, i, role, &task.route, open);
    (next.unwrap_or_else(|| task.route.clone()), None)
}

/// Ruling T16-2: rung 2's and `run retry`'s step for a test writer on `current`: the
/// `test_writer` row's escalation (decision 29), held to the overlap rule as a
/// transient lane; with nothing left, `current`.
pub fn writer_step(run: &Run, i: usize, current: &Route) -> Route {
    let open = |runtime| runtime_open(run, i, runtime, Mover::Transient);
    step(run, i, Role::TestWriter, current, open).unwrap_or_else(|| current.clone())
}

/// Milestone 9.7 decision 16 (DH §4.2, BR-15): the route an engine-made fix task for
/// culprit task `culprit` (its index) takes one rung up from `current`: the culprit's
/// row's escalation (decision 29), past the runtimes [`runtime_open`] holds off for
/// `mover` and the routes that failed in the culprit's task, as its worker's rung 2
/// would ([`rung2_route`]); with nothing left, `current`.
pub(crate) fn escalate_for(run: &Run, culprit: usize, current: &Route, mover: Mover) -> Route {
    let role = RunModels::task_role(&run.tasks[culprit]);
    let open = |runtime| runtime_open(run, culprit, runtime, mover);
    step(run, culprit, role, current, open).unwrap_or_else(|| current.clone())
}

/// `role_step::escalate` for task `i`'s `role` from `current`, skipping the routes that
/// failed in the task (RL-1) and every step on a runtime `open` refuses.
fn step(
    run: &Run,
    i: usize,
    role: Role,
    current: &Route,
    open: impl Fn(Runtime) -> bool,
) -> Option<Route> {
    let models = run.limits.models();
    let mut skip = failed_routes(&run.tasks[i]);
    skip.extend((role_step::steps(models, role, current).into_iter()).filter(|r| !open(r.runtime)));
    role_step::escalate(models, role, current, &skip)
}

/// Rulings T10a-3, T10a-5, T10a-6: the run-log line when task `i`'s next route, `route`
/// (from [`rung2_route`]), failed in this task too, and the original is retried: every
/// route of its row failed, or only a route the overlap rule holds task `i` off has not.
pub fn every_route_failed(run: &Run, i: usize, route: &Route) -> Option<String> {
    let role = RunModels::task_role(&run.tasks[i]);
    route_failed_line(run, i, role, &run.tasks[i].route, route)
}

/// [`every_route_failed`] for a test writer stepping from `from`, its own route (ruling
/// T16-7, N3), as the worker's steps from the task's route.
pub fn writer_route_failed(run: &Run, i: usize, from: &Route, route: &Route) -> Option<String> {
    route_failed_line(run, i, Role::TestWriter, from, route)
}

fn route_failed_line(
    run: &Run,
    i: usize,
    role: Role,
    from: &Route,
    route: &Route,
) -> Option<String> {
    let task = &run.tasks[i];
    if !failed_in(&failed_routes(task), route) {
        return None;
    }
    let (id, runtime, model) = (task.id(), route.runtime.label(), &route.model);
    // Only the installed skip: a step left is one the overlap rule held off.
    let installed = |r: Runtime| r == task.route.runtime || !missing(&run.orch.installed, r);
    let held = step(run, i, role, from, installed).is_some();
    Some(if held {
        format!(
            "no route for task {id} keeps the overlap rule and has not failed in this task; \
             retrying {runtime}/{model}"
        )
    } else {
        format!("every route for task {id} failed in this task; retrying {runtime}/{model}")
    })
}
