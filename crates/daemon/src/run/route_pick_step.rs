//! Rung 2's and `run retry`'s step for a task (decision 9a, rulings RL-1, T10a-1,
//! T10a-3): the list's next candidate, else the roster's escalation, each skipping a
//! route that failed in this task. Pure (design decision 1).

use proto::{Route, RoutingCandidate};

use super::*;
use crate::run::model_roles::runtime_open;
use crate::run::roster::{escalate_skipping, peer};

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

/// The route rung 2 and `run retry` give task `i` (decision 9a, ruling RL-1): its
/// list's [`next_candidate`], else `roster::escalate`, each skipping a route that failed
/// in this task; with the list's step for the routing history. Ruling T10a-5: when the
/// current route failed in this task and the list has no step, the roster substitutes
/// for it ([`roster_substitute`]); with nothing left, the task's own route is retried
/// ([`every_route_failed`] says so). Ruling T10a-6: both roster paths take only the
/// entries of [`open_roster`].
pub fn rung2_route(run: &Run, i: usize) -> (Route, Option<ListPick>) {
    let (task, installed) = (&run.tasks[i], &run.orch.installed);
    if let Some((route, step)) = next_candidate(&run.limits, &run.tasks, i, installed) {
        return (route, Some(step));
    }
    let failed = failed_routes(task);
    let roster = open_roster(run, i, Some(Mover::Worker));
    if failed_in(&failed, &task.route) {
        let route = roster_substitute(&roster, &task.route, &failed);
        return (route.unwrap_or_else(|| task.route.clone()), None);
    }
    (escalate_skipping(&roster, &task.route, &failed), None)
}

/// Ruling T16-2: rung 2's and `run retry`'s step for a test writer on `current`: the
/// roster half of [`rung2_route`] from the writer's route (a model list picks the
/// task's route, which the implementer keeps): a substitute when `current` failed in
/// this task, else `escalate_skipping`, both over [`open_roster`].
pub fn writer_step(run: &Run, i: usize, current: &Route) -> Route {
    let failed = failed_routes(&run.tasks[i]);
    let roster = open_roster(run, i, Some(Mover::Transient));
    if failed_in(&failed, current) {
        return roster_substitute(&roster, current, &failed).unwrap_or_else(|| current.clone());
    }
    escalate_skipping(&roster, current, &failed)
}

/// Milestone 9.7 decision 16 (DH §4.2, BR-15): the route an engine-made fix task for
/// culprit task `culprit` (its index) takes one rung up from `current`: the roster's
/// escalation over the entries [`open_roster`] leaves open for `mover` (installed, and
/// for a worker rule 9's overlap skip), skipping the routes that failed in the
/// culprit's task, as its worker's rung 2 would ([`rung2_route`]).
pub(crate) fn escalate_for(run: &Run, culprit: usize, current: &Route, mover: Mover) -> Route {
    let roster = open_roster(run, culprit, Some(mover));
    escalate_skipping(&roster, current, &failed_routes(&run.tasks[culprit]))
}

/// Ruling T10a-6: the roster entries task `i` may move to: those on its own runtime,
/// and those on a runtime installed for the run that no unfinished task on another
/// runtime holds by overlapping the task's `owns` (M8a decision 11, contract rule 22: as
/// the lists' `overlapping owns` skip). Which tasks hold is `rule`'s mover's
/// ([`alongside`], rulings FW-1 and FW-5); `None` leaves the overlap rule out. With none
/// on another runtime, rung 2 escalates within its own.
fn open_roster(run: &Run, i: usize, rule: Option<Mover>) -> Vec<ModelEntry> {
    let task = &run.tasks[i];
    // Ruling F40: the one copy of the predicate, `model_roles::runtime_open`; without a
    // mover only the installed skip applies.
    let open = |runtime: Runtime| match rule {
        Some(mover) => runtime_open(run, i, runtime, mover),
        None => runtime == task.route.runtime || !missing(&run.orch.installed, runtime),
    };
    (run.roster.iter())
        .filter(|e| open(e.runtime))
        .cloned()
        .collect()
}

/// Ruling T10a-5: a substitute for `current`, which failed in this task, from the
/// `roster` entries ([`open_roster`]'s) that have not failed in it: the peer runtime's
/// first at the same strength, else the strongest left (the peer runtime's first, else
/// the roster's first, on a tie). At `high` effort, as `roster::escalate_skipping`
/// steps on from a failed route. `None` when every one has failed.
fn roster_substitute(roster: &[ModelEntry], current: &Route, failed: &[Route]) -> Option<Route> {
    let route = |e: &ModelEntry| Route {
        runtime: e.runtime,
        model: e.model.clone(),
        strength: e.strength,
        effort: Effort::HIGH,
    };
    let left: Vec<&ModelEntry> = (roster.iter())
        .filter(|e| !failed_in(failed, &route(e)))
        .collect();
    let peer = peer(current.runtime);
    let strongest = left.iter().map(|e| e.strength).max()?;
    let at = |runtime: Option<Runtime>, strength: Strength| {
        (left.iter()).find(|e| e.strength == strength && runtime.is_none_or(|r| e.runtime == r))
    };
    let entry = (at(Some(peer), current.strength))
        .or_else(|| at(Some(peer), strongest))
        .or_else(|| at(None, strongest))?;
    Some(route(entry))
}

/// Rulings T10a-3, T10a-5, T10a-6: the run-log line when task `i`'s next route, `route`
/// (from [`rung2_route`]), failed in this task too, and the original is retried: every
/// list and installed roster route failed, or only a route the overlap rule holds
/// task `i` off has not.
pub fn every_route_failed(run: &Run, i: usize, route: &Route) -> Option<String> {
    route_failed_line(run, i, &run.tasks[i].route, route)
}

/// [`every_route_failed`] for a test writer stepping from `from`, its own route (ruling
/// T16-7, N3), as the worker's steps from the task's route.
pub fn writer_route_failed(run: &Run, i: usize, from: &Route, route: &Route) -> Option<String> {
    route_failed_line(run, i, from, route)
}

fn route_failed_line(run: &Run, i: usize, from: &Route, route: &Route) -> Option<String> {
    let task = &run.tasks[i];
    let failed = failed_routes(task);
    if !failed_in(&failed, route) {
        return None;
    }
    let (id, runtime, model) = (task.id(), route.runtime.label(), &route.model);
    let held = roster_substitute(&open_roster(run, i, None), from, &failed).is_some();
    Some(if held {
        format!(
            "no route for task {id} keeps the overlap rule and has not failed in this task; \
             retrying {runtime}/{model}"
        )
    } else {
        format!("every route for task {id} failed in this task; retrying {runtime}/{model}")
    })
}
