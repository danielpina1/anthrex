//! Milestone 9.8 decision 29 (MR §3.5, D2): escalation along the role table. A
//! struggling role first raises its effort one step along its model's discovered list
//! (`RunModels.efforts`); at the top, or with no list, it switches to the row's
//! fallback at the fallback's default effort; on the fallback it climbs the fallback's
//! list; then nothing is left. A route that failed in the task (ruling RL-1) is
//! stepped over. Never a model the row (or the route itself) does not name. Pure
//! (design decision 1).

use proto::models::{ModelRef, Role};
use proto::{Effort, Route};

use super::model_roles::{RunModels, failed_in};

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
        .find(|route| !failed_in(failed, route))
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

/// `model` at each effort of its list above `effort`. `DEFAULT` sits at the model's
/// default effort, else below the first; an effort the list does not hold (a user's,
/// which decision 21 never validated) counts as its top.
fn above(models: &RunModels, model: &ModelRef, effort: &Effort) -> Vec<Route> {
    let Some(known) = models.efforts.get(model) else {
        return Vec::new();
    };
    let list = &known.efforts;
    let find = |name: &str| list.iter().position(|e| e == name);
    let next = match effort.is_default() {
        true => known.default.as_deref().and_then(find).map_or(0, |k| k + 1),
        false => find(effort.as_str()).map_or(list.len(), |k| k + 1),
    };
    (list.iter().skip(next))
        .map(|e| RunModels::route_of(model, Some(e)))
        .collect()
}

/// Whether `a` and `b` name the same model (runtime and id).
fn same(a: &ModelRef, b: &ModelRef) -> bool {
    a.runtime == b.runtime && a.route_model() == b.route_model()
}

#[cfg(test)]
#[path = "role_step_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "role_step_run_tests.rs"]
mod run_tests;
