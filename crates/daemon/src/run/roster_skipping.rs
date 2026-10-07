//! Milestone 9.5 ruling RL-1 (task M9.5.10b): [`pick_reviewer`] and [`escalate`] over
//! the roster less the routes that failed in this task. A child of `roster.rs`. Pure.

use proto::{Effort, ModelEntry, Route};

use super::{escalate, pick_reviewer, route_from};
use crate::run::model::ReviewLevel;
use crate::run::model_roles::failed_in;

/// Milestone 9.5 ruling RL-1: [`pick_reviewer`] and [`escalate`] with no roster entry
/// that failed in this task (`failed`, by runtime and model). A route that failed
/// itself is not given more effort: it steps on as a `high` one.
pub fn pick_reviewer_skipping(
    roster: &[ModelEntry],
    author: &Route,
    level: ReviewLevel,
    failed: &[Route],
) -> Route {
    pick_reviewer(&without(roster, failed), author, level)
}

/// See [`pick_reviewer_skipping`].
pub fn escalate_skipping(roster: &[ModelEntry], route: &Route, failed: &[Route]) -> Route {
    let mut from = route.clone();
    if failed_in(failed, route) {
        from.effort = Effort::HIGH;
    }
    escalate(&without(roster, failed), &from)
}

fn without(roster: &[ModelEntry], failed: &[Route]) -> Vec<ModelEntry> {
    let kept = |e: &&ModelEntry| !failed_in(failed, &route_from(e, Effort::LOW));
    roster.iter().filter(kept).cloned().collect()
}
