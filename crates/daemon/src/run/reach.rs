//! The runtimes a run can reach (ruling T22-I1b): every runtime any of its sessions can
//! ever run on, the set decisions 50 and 53 are checked against at `run start` and again
//! when a plan edit would widen it. Pure (design decision 2).
//!
//! Per task, the worker's route and every route decision 39's escalation reaches from
//! it, applied again and again to a fixpoint (rung 2 escalates once; each `run retry`
//! escalates the route it finds, so any number of steps can be taken); and for each of
//! those routes, its reviewer at every review level the task can have: its own, and the
//! level rung 3 re-resolves for each larger size (decision 38 raises the size one step
//! at a time, and an unreviewed `S` task is reviewed once raised). The task's current
//! reviewer route is counted as well, and (milestone 9.5) a racing task's second racer
//! and a paired task's test writer.
//! A run's sessions change routes only by those steps, so the set does not grow
//! while the run goes on; it grows only by a plan edit.
//!
//! Milestone 9.8 (task M9.8.7a): a reviewer, a second racer and a test writer come from
//! the run's role table (decisions 27, 28), so each reviewed route counts the reviewer
//! row's model and its fallback, a racing task its row's fallback, and a paired task the
//! `test_writer` row. Task M9.8.8: escalation is `role_step::escalate` along a row
//! (decision 29), so a route reaches every step of the rows it can escalate along: the
//! task's row at each size from its own up, and the test writer's row for the writer.
//! A larger size's row route is counted too for a task that names no model (the
//! decider's raise re-resolves it, decision 10).

use proto::{Route, Runtime, Size};

use super::model::{ReviewLevel, Run, Task};
use super::model_roles::{RunModels, missing};
use super::role_step;
use super::validate::resolve_task_lenient;
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
        // Milestone 9.8 decision 28: lane b's route (the row's fallback), and the test
        // writer's (its row); each may be the task's own route instead.
        let racer = (t.spec.race).then(|| models.racer_route(RunModels::task_role(t), &t.route));
        let roles = task_roles(t);
        // A raise re-resolves a task that names no model to the larger size's row.
        let raised = (t.spec.route.model.is_none())
            .then(|| roles.iter().skip(1).map(|role| models.route(*role)))
            .into_iter()
            .flatten();
        let starts: Vec<Route> = (std::iter::once(t.route.clone()))
            .chain(racer)
            .chain(raised)
            .collect();
        let writer = (t.spec.pair).then(|| models.route(Role::TestWriter));
        let worker = starts.iter().flat_map(|r| escalations(run, &roles, r));
        let writer = writer
            .iter()
            .flat_map(|r| escalations(run, &[Role::TestWriter], r));
        for route in worker.chain(writer).collect::<Vec<_>>() {
            found.push(route.runtime);
            if reviewed {
                found.extend(reviewers.iter().copied());
            }
        }
    }
    // Milestone 9 decision 26: the orchestrator's runtime and its sub-planners'; and
    // (whole-branch review, item 1) its run scouts'. Milestone 9.8: their rows.
    if let Some(o) = &run.orch.orchestrator {
        found.push(o.route.runtime);
        found.extend(super::orch::launch::planner_route(run).map(|r| r.runtime));
        found.push(super::orch::launch::scout_route(run).runtime);
        // Milestone 9.6 decision 10: a design run's brainstormers and its document
        // reviewer (milestone 9.8: the `brainstorm` row and the `reviewer` row's pick).
        if run.design_mode == proto::DesignMode::Full {
            let picks = super::orch::roles::lists::brainstorm_picks(run);
            found.extend(picks.iter().map(|p| p.route.runtime));
            let caps = crate::decider::caps();
            let doc = super::orch::roles::lists::review_pick(run, &caps);
            found.extend(doc.map(|p| p.route.runtime));
        }
    }
    [Runtime::Claude, Runtime::Codex]
        .into_iter()
        .filter(|r| found.contains(r))
        .collect()
}

/// `route` and every route escalation along `roles`' rows reaches from it
/// (`role_step::steps`, decision 29), past a runtime the run's start found missing, as
/// rung 2 and a fix task step over it (milestone 9.7 decision 16).
fn escalations(run: &Run, roles: &[Role], route: &Route) -> Vec<Route> {
    let models = run.limits.models();
    let steps = (roles.iter()).flat_map(|role| role_step::steps(models, *role, route));
    (std::iter::once(route.clone()).chain(steps))
        .filter(|r| r.runtime == route.runtime || !missing(&run.orch.installed, r.runtime))
        .collect()
}

/// The rows task `t` can take, its own first: its own size's, and each larger size's a
/// raise moves it to (decision 10).
fn task_roles(t: &Task) -> Vec<Role> {
    let mut roles = Vec::new();
    for size in [Size::S, Size::M, Size::L]
        .into_iter()
        .filter(|s| *s >= t.size)
    {
        let role = RunModels::role_of(t.spec.kind, t.hub, size);
        if !roles.contains(&role) {
            roles.push(role);
        }
    }
    roles
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
        let (resolved, _) = resolve_task_lenient(spec, &run.profile, &run.limits);
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
