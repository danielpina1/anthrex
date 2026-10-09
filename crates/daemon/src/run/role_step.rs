//! Milestone 9.8 decision 29 (MR §3.5, D2): escalation along the role table. A
//! struggling role first raises its effort one step along its model's discovered list
//! (`RunModels.efforts`); at the top, or with no list, it switches to the row's
//! fallback at the fallback's default effort; on the fallback it climbs the fallback's
//! list; then nothing is left. A route that failed in the task (ruling RL-1) is
//! stepped over. Never a model the row (or the route itself) does not name. Task
//! M9.8.13 moved here (preflight ruling F13) the run-bound steps that use it: rung 2's
//! and `run retry`'s, the test writer's and an engine fix task's, with their
//! every-route-failed lines. Pure (design decision 1).

use proto::models::{ModelRef, Role};
use proto::{Effort, Route, Runtime};

use super::model::Run;
use super::model_roles::{Mover, RunModels, failed_in, failed_routes, missing, runtime_open};

/// Decision 29: the next route for `role` from `current`, skipping `failed` (by runtime
/// and model, `model_roles::failed_in`); `None` when there is none.
///
/// On the row's fallback, `current` climbs the fallback's list. On any other model (the
/// row's own, or a route a user set, decision 10) it climbs that model's list, then
/// takes the row's fallback at `DEFAULT`, then climbs the fallback's list.
pub fn escalate(
    models: &RunModels,
    role: Role,
    current: &Route,
    failed: &[Route],
) -> Option<Route> {
    ladder(models, role, current)
        .into_iter()
        .find(|route| !failed_in(models, failed, route))
}

/// Every route escalation can reach from `from` in order, with nothing skipped: the
/// steps [`escalate`] takes one by one. The routing record's escalation pool (ruling
/// F16).
pub fn steps(models: &RunModels, role: Role, from: &Route) -> Vec<Route> {
    ladder(models, role, from)
}

/// The routes above `current`, in escalation order.
fn ladder(models: &RunModels, role: Role, current: &Route) -> Vec<Route> {
    let own = RunModels::model_of(current);
    let same = |a: &ModelRef, b: &ModelRef| models.same_model_ref(a, b);
    let fallback = (models.choice(role).fallback.clone()).filter(|f| !same(f, &own));
    let on_fallback = (models.choice(role).fallback.as_ref()).is_some_and(|f| same(f, &own))
        && !same(&models.choice(role).model, &own);
    let mut out = above(models, &own, &current.effort);
    if !on_fallback && let Some(f) = fallback {
        out.push(RunModels::route_of(&f, None));
        out.extend(above(models, &f, &Effort::DEFAULT));
    }
    out
}

/// `model` at each effort of its list above `effort`, on [`Effort::LADDER`] only (the
/// real-CLI manual check fix, decision 4: Codex's `ultra` is chosen by hand, never
/// climbed into). `DEFAULT` sits at the model's default effort (at the top when that
/// default is off the ladder), else below the first: Claude reports no default (D6), so
/// a Claude row with no effort climbs from the bottom, as before. An effort the list
/// does not hold (a user's, which decision 21 never validated, or one off the ladder)
/// counts as its top.
fn above(models: &RunModels, model: &ModelRef, effort: &Effort) -> Vec<Route> {
    let Some(known) = models.efforts_of(model) else {
        return Vec::new();
    };
    let mut list: Vec<&str> = (known.efforts.iter().map(String::as_str))
        .filter(|e| Effort::ladder_rank(e).is_some())
        .collect();
    list.sort_by_key(|e| Effort::ladder_rank(e));
    let find = |name: &str| list.iter().position(|e| *e == name);
    let next = match (effort.is_default(), known.default.as_deref()) {
        (true, Some(d)) if Effort::ladder_rank(d).is_none() => list.len(),
        (true, d) => d.and_then(find).map_or(0, |k| k + 1),
        (false, _) => find(effort.as_str()).map_or(list.len(), |k| k + 1),
    };
    (list.iter().skip(next))
        .map(|e| RunModels::route_of(model, Some(e)))
        .collect()
}

/// The route rung 2 and `run retry` give task `i` (milestone 9.8 decision 29, ruling
/// RL-1): `role_step::escalate` along its row ([`RunModels::task_role`]) from its route,
/// stepping over a route that failed in this task and a runtime [`runtime_open`] holds
/// the worker off (ruling T10a-6); with nothing left, the task's own route
/// ([`every_route_failed`] says so when it failed).
pub fn rung2_route(run: &Run, i: usize) -> Route {
    let task = &run.tasks[i];
    let role = RunModels::task_role(task);
    let open = |runtime| runtime_open(run, i, runtime, Mover::Worker);
    let next = step(run, i, role, &task.route, open);
    next.unwrap_or_else(|| task.route.clone())
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
    skip.extend((steps(models, role, current).into_iter()).filter(|r| !open(r.runtime)));
    escalate(models, role, current, &skip)
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
    if !failed_in(run.limits.models(), &failed_routes(task), route) {
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

#[cfg(test)]
#[path = "role_step_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "role_step_run_tests.rs"]
mod run_tests;
