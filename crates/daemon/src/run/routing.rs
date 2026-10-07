//! M8b decision 33a (spec §15): the routing decision behind every task-bound agent
//! session, recorded in the task before its session-start op is emitted. A decision
//! snapshots the candidates the selector could use from the run's frozen roster, in
//! the selector's own order and each skipped one with its reason, and the task as it
//! stood at dispatch. Nothing here changes a route: the engine's selectors
//! (`validate::resolve_route`, `role_step::escalate`, `roster::pick_reviewer`) choose, and
//! this module only records. Pure (M8b decision 1). Milestone 9.8 (preflight ruling
//! F16): a first worker's, a racer's, a test writer's and a reviewer's decision records
//! the role table's row (source `role_table`, policy `m9.8-roles-v1`): the row's model,
//! then its fallback; an escalation, M9.8.8's `role_step` steps along the row.

use proto::{
    AgentRole, Effort, ModelEntry, Route, RoutingCandidate, RoutingDecision, RoutingInput, Strength,
};

use super::model::{ReviewLevel, Run, Task};
use super::model_roles::{NOT_INSTALLED, OVERLAPPING_OWNS, RunModels, missing};
use super::role_step;
use super::roster::peer;
use proto::models::Role;

#[path = "routing_lists.rs"]
mod lists;
pub use lists::record_listed_reviewer;

/// Decision 33a's policy versions: the selectors milestone 8a shipped.
pub const WORKER_POLICY: &str = "m8a-worker-v1";
pub const REVIEW_POLICY: &str = "m8a-review-v1";
pub const ESCALATE_POLICY: &str = "m8a-escalate-v1";
/// A decision's `source` when the task took its class's default route (decision 33a).
pub const CLASS_DEFAULT: &str = "class_default";
/// Milestone 9.8 (preflight ruling F16): a worker's, racer's, test writer's or
/// reviewer's decision from the run's role table, its `source` and policy version.
pub const ROLE_TABLE: &str = "role_table";
pub const ROLES_POLICY: &str = "m9.8-roles-v1";

/// The reason given to a selectable candidate after the chosen one.
pub const RANKED_AFTER: &str = "ranked after the selected route";
/// The reason given to a selectable candidate before the chosen one (the chosen route
/// was not the selector's first pick, say an explicit route).
pub const PASSED_OVER: &str = "passed over for the selected route";

/// One candidate before the choice: its route, and why the selector could not take it
/// (`None`: it could).
type Raw = (Route, Option<String>);

fn route_of(entry: &ModelEntry, effort: Effort) -> Route {
    Route {
        runtime: entry.runtime,
        model: entry.model.clone(),
        strength: entry.strength,
        effort,
    }
}

fn strength_label(strength: Strength) -> String {
    format!("{strength:?}").to_lowercase()
}

/// Ruling F16: a row's candidates, its model at its effort then its fallback at its
/// default effort, each with the reason `skip` gives it.
fn row_pool(run: &Run, role: Role, skip: impl Fn(&Route) -> Option<String>) -> Vec<Raw> {
    let models = run.limits.models();
    let fallback = (models.choice(role).fallback.as_ref()).map(|f| RunModels::route_of(f, None));
    (std::iter::once(models.route(role)).chain(fallback))
        .map(|route| {
            let reason = skip(&route);
            (route, reason)
        })
        .collect()
}

/// The task as it stands now: decision 33a's input, frozen into the decision.
pub fn input(run: &Run, task: &Task) -> RoutingInput {
    RoutingInput {
        title: task.spec.title.clone(),
        brief: task.spec.brief.clone(),
        acceptance: task.spec.acceptance.clone(),
        owns: task.spec.owns.clone(),
        kind: task.spec.kind,
        size: task.size,
        hub: task.hub,
        interface_change: task.spec.interface_change,
        test_mode: task.test_mode,
        languages: run.profile_languages.clone(),
    }
}

/// The ordered candidates with `chosen` selected: the first candidate whose route is
/// `chosen`, or `chosen` appended when the pool lacks it (an explicit route, a
/// fallback). Every other candidate keeps its own reason, or [`PASSED_OVER`] /
/// [`RANKED_AFTER`] when the selector could have taken it.
pub fn select(raw: Vec<Raw>, chosen: &Route) -> (Vec<RoutingCandidate>, u32) {
    let mut raw = raw;
    let selected = match raw.iter().position(|(route, _)| route == chosen) {
        Some(k) => k,
        None => {
            raw.push((chosen.clone(), None));
            raw.len() - 1
        }
    };
    let candidates = raw
        .into_iter()
        .enumerate()
        .map(|(k, (route, reason))| {
            let skipped_reason = match k.cmp(&selected) {
                std::cmp::Ordering::Equal => None,
                std::cmp::Ordering::Less => reason.or_else(|| Some(PASSED_OVER.into())),
                std::cmp::Ordering::Greater => reason.or_else(|| Some(RANKED_AFTER.into())),
            };
            RoutingCandidate {
                route,
                skipped_reason,
            }
        })
        .collect();
    (candidates, selected as u32)
}

/// A first worker's pool (`validate::resolve_route`): every roster entry in roster
/// order at the chosen effort. With a model named by the task only that entry is
/// selectable; otherwise the entries on the task's runtime at its strength are, the
/// first of them being the selector's pick.
pub fn worker_pool(roster: &[ModelEntry], chosen: &Route, explicit: bool) -> Vec<Raw> {
    roster
        .iter()
        .map(|entry| {
            let reason = if entry.runtime != chosen.runtime {
                Some(format!(
                    "runtime {}, the task runs on {}",
                    entry.runtime, chosen.runtime
                ))
            } else if explicit && entry.model != chosen.model {
                Some(format!("the task names the model {}", chosen.model))
            } else if !explicit && entry.strength != chosen.strength {
                Some(format!(
                    "strength {}, the task needs {}",
                    strength_label(entry.strength),
                    strength_label(chosen.strength)
                ))
            } else {
                None
            };
            (route_of(entry, chosen.effort.clone()), reason)
        })
        .collect()
}

/// An escalated session's pool (milestone 9.8 decision 29, ruling F16), in
/// `role_step::escalate`'s order from `from` along `role`'s row: every step it can
/// reach, then `from` unchanged (rung 2 with nothing left); then the row's model and
/// fallback where escalation from `from` never goes.
pub fn escalation_pool(models: &RunModels, role: Role, from: &Route) -> Vec<Raw> {
    let mut raw: Vec<Raw> = (role_step::steps(models, role, from).into_iter())
        .map(|route| (route, None))
        .collect();
    raw.push((from.clone(), None));
    let choice = models.choice(role);
    let fallback = (choice.fallback.as_ref()).map(|f| RunModels::route_of(f, None));
    let not_a_step = format!("not an escalation step from {}", RunModels::label_of(from));
    for route in std::iter::once(models.route(role)).chain(fallback) {
        if !raw.iter().any(|(r, _)| *r == route) {
            raw.push((route, Some(not_a_step.clone())));
        }
    }
    raw
}

/// `entries` weakest first, or strongest first; stable, so ties stay in roster order,
/// as the selectors pick among them.
fn rank(mut entries: Vec<&ModelEntry>, strongest: bool) -> Vec<&ModelEntry> {
    if strongest {
        entries.sort_by_key(|e| std::cmp::Reverse(e.strength));
    } else {
        entries.sort_by_key(|e| e.strength);
    }
    entries
}

/// A reviewer's pool, in `roster::pick_reviewer`'s order against `author` at `level`:
/// the peer runtime's entries at or above the required strength, weakest first; the
/// author runtime's other models at or above it, weakest first; the author runtime's
/// remaining entries, strongest first (its fallback); the author's own route when the
/// roster has no entry on its runtime; then the peer runtime's weaker entries.
pub fn reviewer_pool(roster: &[ModelEntry], author: &Route, level: ReviewLevel) -> Vec<Raw> {
    let (required, effort) = match level {
        ReviewLevel::Small => (Strength::Fast, Effort::LOW),
        ReviewLevel::Medium => (author.strength, Effort::MEDIUM),
        ReviewLevel::Frontier => (Strength::Frontier, Effort::HIGH),
    };
    let below = format!("below the required {} strength", strength_label(required));
    let peer_rt = peer(author.runtime);
    let tier1 = rank(
        roster
            .iter()
            .filter(|e| e.runtime == peer_rt && e.strength >= required)
            .collect(),
        false,
    );
    let tier2 = rank(
        roster
            .iter()
            .filter(|e| {
                e.runtime == author.runtime && e.strength >= required && e.model != author.model
            })
            .collect(),
        false,
    );
    let open = tier1.is_empty() && tier2.is_empty();
    let mut raw: Vec<Raw> = tier1
        .into_iter()
        .chain(tier2)
        .map(|e| (route_of(e, effort.clone()), None))
        .collect();
    let rest = rank(
        roster
            .iter()
            .filter(|e| e.runtime == author.runtime)
            .filter(|e| !raw.iter().any(|(r, _)| *r == route_of(e, effort.clone())))
            .collect(),
        true,
    );
    for entry in rest {
        let reason = if open {
            None
        } else if entry.model == author.model {
            Some("the author's own model".to_string())
        } else {
            Some(below.clone())
        };
        raw.push((route_of(entry, effort.clone()), reason));
    }
    if !roster.iter().any(|e| e.runtime == author.runtime) {
        let fallback = Route {
            effort: effort.clone(),
            ..author.clone()
        };
        let last = "the author's own route, used only when no roster entry fits";
        raw.push((fallback, (!open).then(|| last.to_string())));
    }
    for entry in roster.iter().filter(|e| e.runtime == peer_rt) {
        let route = route_of(entry, effort.clone());
        if !raw.iter().any(|(r, _)| *r == route) {
            raw.push((route, Some(below.clone())));
        }
    }
    raw
}

/// Milestone 9.5 ruling RL-1: each pool entry whose route failed in this task says so,
/// except at the `chosen` model (every entry failed: the selector fell back to it, and
/// the record must not call its own choice skipped).
fn mark_failed(mut raw: Vec<Raw>, task: &Task, chosen: &Route) -> Vec<Raw> {
    use super::model_roles::{failed_in, failed_routes};
    let failed = (failed_routes(task).into_iter())
        .filter(|f| !failed_in(std::slice::from_ref(chosen), f))
        .collect::<Vec<_>>();
    for (route, reason) in raw.iter_mut() {
        if failed_in(&failed, route) {
            *reason = Some(super::model_roles::FAILED_IN_TASK.to_string());
        }
    }
    raw
}

/// Appends `decision` to the task unless one with its identity `(role, session, round,
/// lane)` is there already (a restored or repeated launch); numbers it.
pub fn push(task: &mut Task, mut decision: RoutingDecision) {
    let same = |d: &RoutingDecision| {
        d.role == decision.role
            && d.session == decision.session
            && d.round == decision.round
            && d.lane == decision.lane
    };
    if task.routing_decisions.iter().any(same) {
        return;
    }
    decision.seq = task.routing_decisions.len() as u32 + 1;
    task.routing_decisions.push(decision);
}

#[allow(clippy::too_many_arguments)]
fn decision(
    run: &Run,
    task: &Task,
    (role, session, round): (AgentRole, u32, Option<u32>),
    (trigger, source, policy): (&str, &str, &str),
    chosen: &Route,
    raw: Vec<Raw>,
    now: u64,
) -> RoutingDecision {
    let (candidates, selected_index) = select(raw, chosen);
    RoutingDecision {
        seq: 0,
        at: now,
        role,
        session,
        round,
        lane: None,
        trigger: trigger.into(),
        source: source.into(),
        policy_version: policy.into(),
        pick_policy: None,
        input: input(run, task),
        chosen: chosen.clone(),
        selected_index,
        candidates,
    }
}

/// A worker session of task `i` is being launched on its route (its session number
/// already counted): its first session records `initial`, and a session after rung 2
/// or `run retry` records `escalation` from the route it escalated from. Any other
/// fresh session (a lost resume, say) keeps the route already recorded. A run without
/// history (one from milestone 8a) records nothing (whole-branch review m2). Milestone
/// 9.5 decision 9a: a model list's choice records the list's snapshot.
pub fn record_worker(run: &mut Run, i: usize, now: u64) {
    let from = run.tasks[i].escalated_from.take();
    let step = run.tasks[i].list_escalation.take();
    if !run.history {
        return;
    }
    let task = &run.tasks[i];
    let chosen = task.route.clone();
    let id = (AgentRole::Worker, task.session, None);
    let first = !(task.routing_decisions.iter()).any(|d| d.role == AgentRole::Worker);
    let listed = (from.is_some(), step, first);
    if let Some(d) = lists::worker(run, task, id, listed, &chosen, now) {
        return push(&mut run.tasks[i], d);
    }
    let decision = match from {
        Some(from) => decision(
            run,
            task,
            id,
            ("escalation", "escalation_policy", ROLES_POLICY),
            &chosen,
            mark_failed(
                escalation_pool(run.limits.models(), RunModels::task_role(task), &from),
                task,
                &chosen,
            ),
            now,
        ),
        None if first => {
            // Milestone 9.8 (ruling F16): the task's row; a racer's lane is its fallback.
            // The row's model at the route's effort when the route is that model (a
            // plan's or a user's effort, decision 10).
            let explicit = task.spec.route.model.is_some();
            let source = if explicit {
                "explicit_task"
            } else {
                ROLE_TABLE
            };
            let same = |r: &Route| r.runtime == chosen.runtime && r.model == chosen.model;
            let names = |r: &Route| {
                (explicit && !same(r)).then(|| format!("the task names the model {}", chosen.model))
            };
            let pool = (row_pool(run, RunModels::task_role(task), names).into_iter())
                .map(|(r, why)| {
                    if same(&r) {
                        (chosen.clone(), why)
                    } else {
                        (r, why)
                    }
                })
                .collect();
            decision(
                run,
                task,
                id,
                ("initial", source, ROLES_POLICY),
                &chosen,
                pool,
                now,
            )
        }
        None => return,
    };
    push(&mut run.tasks[i], decision);
}

/// Milestone 9.5 decision 9a: a paired task's test writer session is being launched on
/// `chosen`. Its first session records trigger `test_writer`, its source naming the
/// pick (`peer_route` for the peer runtime's route, else the task route's own source,
/// a model list's `configured_list` with its snapshot when the list chose it);
/// a session after rung 2 or `run retry` records `escalation` from the route it stepped
/// from; any other fresh session (a lost resume, say) records nothing, as a worker's.
/// Nothing for a run without history.
pub fn record_test_writer(run: &mut Run, i: usize, chosen: &Route, now: u64) {
    let from = (run.tasks[i].pair.as_mut()).and_then(|p| p.escalated_from.take());
    if !run.history {
        return;
    }
    let task = &run.tasks[i];
    let id = (AgentRole::TestWriter, task.session, None);
    let first = !(task.routing_decisions.iter()).any(|d| d.role == AgentRole::TestWriter);
    let d = match from {
        Some(from) => decision(
            run,
            task,
            id,
            ("escalation", "escalation_policy", ROLES_POLICY),
            chosen,
            mark_failed(
                escalation_pool(run.limits.models(), Role::TestWriter, &from),
                task,
                chosen,
            ),
            now,
        ),
        None if first => {
            // Ruling T16-7 (N3): on the task's route a list chose, the list's record.
            let listed = (*chosen == task.route)
                .then(|| lists::test_writer(run, task, id, chosen, now))
                .flatten();
            if let Some(d) = listed {
                return push(&mut run.tasks[i], d);
            }
            // Milestone 9.8 (ruling F16): the `test_writer` row; a candidate the
            // overlap rule held off says why (decision 28).
            let held = |route: &Route| {
                let open = *route == *chosen || route.runtime == task.route.runtime;
                (!open).then(|| match missing(&run.orch.installed, route.runtime) {
                    true => NOT_INSTALLED.to_string(),
                    false => OVERLAPPING_OWNS.to_string(),
                })
            };
            let pool = row_pool(run, Role::TestWriter, held);
            decision(
                run,
                task,
                id,
                ("test_writer", ROLE_TABLE, ROLES_POLICY),
                chosen,
                pool,
                now,
            )
        }
        None => return,
    };
    push(&mut run.tasks[i], d);
}

/// Review round `round` of task `i` is being launched on `chosen`, which the reviewer
/// row gave against `author` (milestone 9.8 decision 27; the level no longer chooses).
/// Ruling F16: the row's candidates, the author's own model skipped as such. Nothing for
/// a run without history.
pub fn record_reviewer(
    run: &mut Run,
    i: usize,
    (author, _level): (&Route, ReviewLevel),
    chosen: &Route,
    round: u32,
    now: u64,
) {
    if !run.history {
        return;
    }
    let task = &run.tasks[i];
    let own = |route: &Route| {
        let same = route.runtime == author.runtime && route.model == author.model;
        (same && route != chosen).then(|| "the author's own model".to_string())
    };
    let pool = mark_failed(row_pool(run, Role::Reviewer, own), task, chosen);
    let decision = decision(
        run,
        task,
        (AgentRole::Reviewer, round, Some(round)),
        ("review", ROLE_TABLE, ROLES_POLICY),
        chosen,
        pool,
        now,
    );
    push(&mut run.tasks[i], decision);
}
