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
//! `pick_reviewer` themselves. The task's current reviewer route is counted as well.
//! A run's sessions change routes only by those steps, so the set does not grow
//! while the run goes on; it grows only by a plan edit.

use proto::{Route, Runtime, Size};

use super::model::{ReviewLevel, Run, Task};
use super::roster::{escalate, pick_reviewer};
use super::validate::resolve_task_lenient;

/// Every runtime `run` can launch a session on, in `[Claude, Codex]` order.
pub fn reachable_runtimes(run: &Run) -> Vec<Runtime> {
    let mut found = Vec::new();
    for t in &run.tasks {
        let levels = review_levels(run, t);
        found.extend(t.review_route.as_ref().map(|r| r.runtime));
        for route in escalations(run, &t.route) {
            found.push(route.runtime);
            for &level in &levels {
                found.push(pick_reviewer(&run.roster, &route, level).runtime);
            }
        }
    }
    [Runtime::Claude, Runtime::Codex]
        .into_iter()
        .filter(|r| found.contains(r))
        .collect()
}

/// `route` and every route repeated escalation reaches from it, to the fixpoint. The
/// routes are drawn from a finite set (the roster's entries and the starting route's
/// model, each at three efforts), so the walk ends at a route already seen.
fn escalations(run: &Run, route: &Route) -> Vec<Route> {
    let mut chain = vec![route.clone()];
    loop {
        let next = escalate(&run.roster, chain.last().expect("never empty"));
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
        let (resolved, _) = resolve_task_lenient(
            spec,
            &run.profile,
            &run.limits,
            &run.roster,
            run.limits.default_runtime,
        );
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
