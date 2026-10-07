//! Milestone 9.5 ruling RL-1 (task M9.5.10b): [`pick_reviewer`] over the roster less
//! the routes that failed in this task (the escalation's half left in M9.8.8). A child
//! of `roster.rs`. Pure.

use proto::{Effort, ModelEntry, Route};

use super::{pick_reviewer, route_from};
use crate::run::model::ReviewLevel;
use crate::run::model_roles::failed_in;

/// Milestone 9.5 ruling RL-1: [`pick_reviewer`] with no roster entry that failed in
/// this task (`failed`, by runtime and model).
pub fn pick_reviewer_skipping(
    roster: &[ModelEntry],
    author: &Route,
    level: ReviewLevel,
    failed: &[Route],
) -> Route {
    pick_reviewer(&without(roster, failed), author, level)
}

fn without(roster: &[ModelEntry], failed: &[Route]) -> Vec<ModelEntry> {
    let kept = |e: &&ModelEntry| !failed_in(failed, &route_from(e, Effort::LOW));
    roster.iter().filter(kept).cloned().collect()
}
