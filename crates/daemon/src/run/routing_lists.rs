//! Milestone 9.5 decision 9a's routing decisions over a model list: the list's
//! snapshot (every candidate, in order, each skipped one with its reason), the policy
//! `m9.5-list-v1` and the list's `pick_policy`. Part of `routing.rs` (split for the
//! 600-line rule). Pure.

use proto::{AgentRole, Route, RoutingCandidate, RoutingDecision};

use super::{CLASS_DEFAULT, ESCALATE_POLICY, REVIEW_POLICY, Raw, WORKER_POLICY, decision, push};
use crate::run::model::{ListPick, Run, Task};
use crate::run::route_pick::{FIRST_QUALIFYING, LIST_POLICY};

fn raw(candidates: Vec<RoutingCandidate>) -> Vec<Raw> {
    (candidates.into_iter())
        .map(|c| (c.route, c.skipped_reason))
        .collect()
}

/// A worker decision over a class list's snapshot: `configured_list` when the list
/// chose `chosen`; otherwise the source that did, `chosen` appended when absent.
fn over_list(
    run: &Run,
    task: &Task,
    (id, trigger): ((AgentRole, u32, Option<u32>), &str),
    pick: &ListPick,
    chosen: &Route,
    now: u64,
) -> RoutingDecision {
    let ours = pick.chosen_route() == Some(chosen);
    let explicit = task.spec.route.model.is_some() || task.spec.route.strength.is_some();
    let (source, policy) = match (ours, trigger) {
        (true, _) => ("configured_list", LIST_POLICY),
        (false, "escalation") => ("escalation_policy", ESCALATE_POLICY),
        (false, _) if explicit => ("explicit_task", LIST_POLICY),
        (false, _) => (CLASS_DEFAULT, WORKER_POLICY),
    };
    let pool = raw(pick.candidates.clone());
    let mut d = decision(run, task, id, (trigger, source, policy), chosen, pool, now);
    d.pick_policy = Some(pick.pick.label().to_string());
    d
}

/// The worker decision `routing::record_worker` records for a task its class list
/// routed: rung 2's list step (`escalation`), or the plan's pick on its first session
/// (`initial`); `None` leaves it to milestone 8b's selectors.
pub(super) fn worker(
    run: &Run,
    task: &Task,
    id: (AgentRole, u32, Option<u32>),
    (escalated, step, first): (bool, Option<ListPick>, bool),
    chosen: &Route,
    now: u64,
) -> Option<RoutingDecision> {
    match (escalated, step) {
        (true, Some(step)) => Some(over_list(run, task, (id, "escalation"), &step, chosen, now)),
        (false, _) if first => (task.list_pick.as_ref())
            .map(|pick| over_list(run, task, (id, "initial"), pick, chosen, now)),
        _ => None,
    }
}

/// Milestone 9.5 ruling T16-7 (N3): a test writer's first decision when it runs on the
/// task's route and a class list chose that route: the list's snapshot, `configured_list`.
pub(super) fn test_writer(
    run: &Run,
    task: &Task,
    id: (AgentRole, u32, Option<u32>),
    chosen: &Route,
    now: u64,
) -> Option<RoutingDecision> {
    let pick = (task.list_pick.as_ref()).filter(|p| p.chosen_route() == Some(chosen))?;
    Some(over_list(run, task, (id, "test_writer"), pick, chosen, now))
}

/// Review round `round` of task `i` is being launched on `chosen`, from the `review`
/// list's snapshot `list`: the list's first qualifying candidate (`configured_list`),
/// or, with none qualifying, `pick_reviewer`'s route appended (`review_policy`).
/// Nothing for a run without history.
pub fn record_listed_reviewer(
    run: &mut Run,
    i: usize,
    list: Vec<RoutingCandidate>,
    chosen: &Route,
    round: u32,
    now: u64,
) {
    if !run.history {
        return;
    }
    let ours = (list.iter()).any(|c| c.skipped_reason.is_none() && c.route == *chosen);
    let source = match ours {
        true => ("review", "configured_list", LIST_POLICY),
        false => ("review", "review_policy", REVIEW_POLICY),
    };
    let task = &run.tasks[i];
    let id = (AgentRole::Reviewer, round, Some(round));
    let mut d = decision(run, task, id, source, chosen, raw(list), now);
    d.pick_policy = Some(FIRST_QUALIFYING.to_string());
    push(&mut run.tasks[i], d);
}
