//! The runtimes a run can reach (ruling T22-I1b): every runtime any of its sessions can
//! ever run on, the set decisions 50 and 53 are checked against at `run start` and again
//! when a plan edit would widen it. Pure (design decision 2).
//!
//! Per task, the worker's route and every route decision 39's escalation reaches from
//! it, applied again and again to a fixpoint (rung 2 escalates once; each `run retry`
//! escalates the route it finds, so any number of steps can be taken); and for each of
//! those routes, its reviewer at every review level the task can have: its own, and the
//! level rung 3 re-resolves for each larger size (decision 38 raises the size one step
//! at a time, and an unreviewed `S` task is reviewed once raised). The roster's
//! fallbacks (the peer runtime, else the same runtime) are those of `escalate` and
//! `pick_reviewer` themselves. The task's current reviewer route is counted as well, and
//! (milestone 9.5) a racing task's second racer and a paired task's test writer, whose
//! peer route escalation reaches too, so naming them changes no reach today.
//! A run's sessions change routes only by those steps, so the set does not grow
//! while the run goes on; it grows only by a plan edit.
//!
//! Milestone 9.8 (task M9.8.7a): a reviewer, a second racer and a test writer come from
//! the run's role table (decisions 27, 28), so each reviewed route counts the reviewer
//! row's model and its fallback, a racing task its row's fallback, and a paired task the
//! `test_writer` row; escalation stays the roster's until M9.8.8.

use proto::{Route, Runtime, Size};

use super::model::{ReviewLevel, Run, Task};
use super::model_roles::RunModels;
use super::model_roles::installed_roster;
use super::roster::escalate;
use super::route_pick::task_list;
use super::validate::resolve_task_lenient;
use super::validate_patterns::peer_route;
use proto::models::Role;

/// Whether `edits` can widen [`reachable_runtimes`] (T22-P2, F4): only a task added,
/// split or amended can; a pause, resume, finish, cancel, answer or dependency cannot,
/// so `run edit` probes git for those batches no more, and a git error cannot refuse
/// them.
pub fn edits_may_widen(edits: &[proto::PlanEdit]) -> bool {
    use proto::PlanEdit;
    edits.iter().any(|edit| {
        matches!(
            edit,
            PlanEdit::AddTask { .. } | PlanEdit::SplitTask { .. } | PlanEdit::AmendTask { .. }
        )
    })
}

/// Every runtime `run` can launch a session on, in `[Claude, Codex]` order.
pub fn reachable_runtimes(run: &Run) -> Vec<Runtime> {
    let mut found = Vec::new();
    let models = run.limits.models();
    // Milestone 9.8 decision 27: a reviewer is the reviewer row's model, or its
    // fallback (against the author's model, or past a route that failed, RL-1).
    let reviewer = models.choice(Role::Reviewer);
    let reviewers: Vec<Runtime> = (std::iter::once(&reviewer.model).chain(&reviewer.fallback))
        .map(|m| m.runtime)
        .collect();
    for t in &run.tasks {
        let reviewed = !review_levels(run, t).is_empty();
        found.extend(t.review_route.as_ref().map(|r| r.runtime));
        // Milestone 9.5 decision 9a: rung 2 can take any candidate of its list.
        let lists = &run.limits.route_lists;
        let listed =
            (task_list(lists, t).candidates.iter()).map(|c| c.route(t.route.effort.clone()));
        // Milestone 9.8 decision 28: lane b's route (the row's fallback), and the test
        // writer's (its row); each may be the task's own route instead.
        let racer = (t.spec.race).then(|| models.racer_route(RunModels::task_role(t), &t.route));
        let writer = (t.spec.pair).then(|| models.route(Role::TestWriter));
        let starts: Vec<Route> = (std::iter::once(t.route.clone()).chain(listed))
            .chain(racer)
            .chain(writer)
            .collect();
        for route in starts.iter().flat_map(|r| escalations(run, r)) {
            found.push(route.runtime);
            if reviewed {
                found.extend(reviewers.iter().copied());
            }
        }
    }
    // Milestone 9 decision 26: the orchestrator's runtime and its sub-planners'; and
    // (whole-branch review, item 1) its run scouts', on the keys the run froze.
    if let Some(o) = &run.orch.orchestrator {
        found.push(o.route.runtime);
        found.extend(super::orch::launch::planner_route(run).map(|r| r.runtime));
        found.push(super::orch::launch::frozen_scout_route(run).runtime);
        // Milestone 9.5 decision 9a: a role list's every candidate can be taken.
        let lists = &run.limits.route_lists;
        found.extend((lists.scout.candidates.iter()).map(|c| c.runtime));
        found.extend((lists.planner.candidates.iter()).map(|c| c.runtime));
        // Milestone 9.6 decision 10: a design run's brainstormers (every `brainstorm`
        // candidate, else the strongest of each installed runtime) and its document
        // reviewer (the orchestrator's peer, else its own runtime).
        if run.design_mode == proto::DesignMode::Full {
            found.extend((lists.brainstorm.candidates.iter()).map(|c| c.runtime));
            let picks = super::orch::roles::lists::brainstorm_picks(run);
            found.extend(picks.iter().map(|p| p.route.runtime));
            let peer = peer_route(&run.roster, &o.route, &run.orch.installed);
            found.extend(peer.map(|r| r.runtime));
        }
    }
    [Runtime::Claude, Runtime::Codex]
        .into_iter()
        .filter(|r| found.contains(r))
        .collect()
}

/// `route` and every route repeated escalation reaches from it, to the fixpoint. The
/// routes are drawn from a finite set (the roster's entries and the starting route's
/// model, each at three efforts), so the walk ends at a route already seen. Milestone
/// 9.7 decision 16: over the installed roster only, as rung 2 and a fix task escalate.
fn escalations(run: &Run, route: &Route) -> Vec<Route> {
    let roster = installed_roster(&run.roster, &run.orch.installed);
    let mut chain = vec![route.clone()];
    loop {
        let next = escalate(&roster, chain.last().expect("never empty"));
        if chain.contains(&next) {
            return chain;
        }
        chain.push(next);
    }
}

/// The review levels task `t` can be reviewed at: its own, and the one rung 3's
/// re-resolution (`ladder::reresolve`) gives it at each size from its own up.
fn review_levels(run: &Run, t: &Task) -> Vec<ReviewLevel> {
    let mut levels: Vec<ReviewLevel> = t.review_level.into_iter().collect();
    for size in [Size::S, Size::M, Size::L] {
        if size < t.size {
            continue;
        }
        let mut spec = t.spec.clone();
        spec.size = spec.size.max(size);
        let (resolved, _) = resolve_task_lenient(spec, &run.profile, &run.limits, &run.roster);
        if let Some(level) = resolved.review_level
            && !levels.contains(&level)
        {
            levels.push(level);
        }
    }
    levels
}

#[cfg(test)]
#[path = "reach_tests.rs"]
mod tests;
