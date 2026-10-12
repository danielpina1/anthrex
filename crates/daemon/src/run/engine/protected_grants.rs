//! Milestone 9.13 decision 6 (NA §2 A6): a protected path an `owns` entry names
//! literally is a grant, and a grant is the user's to approve. Past the plan gate, an
//! orchestrator's or sub-planner's batch may add one only under a hold the user has not
//! yet decided ([`unheld`]); a hold whose tasks carry one is approved by the user only
//! ([`of_hold`]). Computed when asked, never stored. Pure (design decision 2).

use super::gate_holds;
use crate::run::globs::{ProtectedMatcher, literal_protected};
use crate::run::model::Run;

/// The grants `owns` carries against the run's `protected` list.
pub(super) fn literal(owns: &[String], protected: &[String]) -> Vec<String> {
    match ProtectedMatcher::new(protected) {
        Ok(matcher) => literal_protected(owns, &matcher),
        Err(_) => Vec::new(),
    }
}

/// The batch's refusal: the first grant `edited` gained over `before` on a task that
/// does not end the batch under a hold still undecided; `None` before the plan gate
/// (ruling C4: the draft's grants are approved with the plan) and when there is none.
/// `added` are the batch's new tasks, which had no grant before it.
pub(super) fn unheld(edited: &Run, before: &Run, added: &[String]) -> Option<String> {
    if !gate_holds::past_gate(edited) {
        return None;
    }
    let protected = &edited.profile.protected;
    edited
        .tasks
        .iter()
        .filter(|task| gate_holds::released(edited, task))
        .find_map(|task| {
            let had = match added.iter().any(|id| id == task.id()) {
                true => Vec::new(),
                false => (before.task(task.id()))
                    .map_or_else(Vec::new, |t| literal(&t.spec.owns, protected)),
            };
            literal(&task.spec.owns, protected)
                .into_iter()
                .find(|grant| !had.contains(grant))
        })
        .map(|grant| format!("owns names protected {grant}; ask the user (ask_user)"))
}

/// The grants hold `hold`'s live tasks carry, in task order, each once.
pub(super) fn of_hold(run: &Run, hold: &str) -> Vec<String> {
    let mut grants: Vec<String> = Vec::new();
    let held = (run.tasks.iter())
        .filter(|t| t.orch.gate_hold.as_deref() == Some(hold))
        .filter(|t| !t.state.is_finished());
    for task in held {
        for grant in literal(&task.spec.owns, &run.profile.protected) {
            if !grants.contains(&grant) {
                grants.push(grant);
            }
        }
    }
    grants
}
