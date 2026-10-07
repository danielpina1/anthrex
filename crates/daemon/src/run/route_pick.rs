//! Milestone 9.5 decision 9a for tasks (task M9.5.10a): which candidate of the user's
//! model lists a task's worker gets ([`pick`], at a build and for the tasks an edit adds
//! or re-routes), the candidate rung 2 moves it to ([`next_candidate`]), and a reviewer
//! from the `review` list ([`reviewer`]). The lists are the run's frozen copy
//! (`RunLimits.route_lists`). Every choice is snapshotted with the entire list and why
//! each candidate was skipped, for the routing history (`routing.rs`). Task M9.5.10b:
//! research and review tasks take the `scout` and `review` lists (ruling RL-4), a role's
//! session takes [`role`], and rung 2, `run retry` and the reviewer skip a route that
//! failed in this task ([`failed_routes`], ruling RL-1). Pure (design decision 1).

use std::collections::{BTreeMap, BTreeSet};

use proto::{Effort, ModelEntry, Route, RoutingCandidate, Runtime, Size, Strength, TaskKind};

use super::globs::any_intersect;
use super::model::{
    FrozenList, ListPick, ListPolicy, ReviewLevel, RouteListsFrozen, Run, RunLimits, Task,
};
use super::orch::roles::EARLIER_TAKEN;
use super::roster::pick_reviewer;

/// Decision 9a's policy version for a list's choice.
pub const LIST_POLICY: &str = "m9.5-list-v1";
/// A reviewer list's `pick_policy`.
pub const FIRST_QUALIFYING: &str = "first_qualifying";
/// Skip reasons, as the routing history records them.
pub const OVERLAPPING_OWNS: &str = "overlapping owns";
pub const BELOW_STRENGTH: &str = "below the author's strength";
pub const NOT_INSTALLED: &str = super::orch::roles::NOT_INSTALLED;
pub const AUTHOR_RUNTIME: &str = "the author's runtime";
pub const CURRENT_ROUTE: &str = "the current route";
/// Ruling T10a-1: rung 2 never steps to a weaker strength, nor to a lower effort at the
/// same strength.
pub const BELOW_CURRENT: &str = "below the current route";
/// Ruling RL-1: the route of a session of this task that ended for an environment reason.
pub const FAILED_IN_TASK: &str = "failed in this task";

/// What a run's start found installed (`run.orch.installed`, by runtime label). A
/// runtime it does not name (a plan-file run records nothing) counts as installed.
pub type Installed = BTreeMap<String, bool>;

fn missing(installed: &Installed, runtime: Runtime) -> bool {
    installed.get(runtime.label()) == Some(&false)
}

/// Ruling RL-1: the routes of this task's sessions that ended for an environment reason
/// (`AgentRound::environment_failed`), which rung 2 and `run retry` skip.
pub fn failed_routes(task: &Task) -> Vec<Route> {
    (task.rounds.iter())
        .filter(|r| r.environment_failed)
        .map(|r| r.route.clone())
        .collect()
}

/// Whether `route` is, by runtime and model, one of `failed` (an effort changes nothing
/// about a model that cannot run).
pub fn failed_in(failed: &[Route], route: &Route) -> bool {
    (failed.iter()).any(|f| f.runtime == route.runtime && f.model == route.model)
}

/// The list a task takes (decision 9a, ruling RL-4): research by `scout`, review by
/// `review`, any other task by its class's.
pub fn task_list<'a>(lists: &'a RouteListsFrozen, task: &Task) -> &'a FrozenList {
    match task.spec.kind {
        TaskKind::Research => &lists.scout,
        TaskKind::Review => &lists.review,
        _ => class_list(lists, task),
    }
}

/// The rotation a task's pick belongs to: its class's (0 S, 1 M, 2 hub), else 3 for
/// research and 4 for review tasks.
fn list_key(task: &Task) -> usize {
    match task.spec.kind {
        TaskKind::Research => 3,
        TaskKind::Review => 4,
        _ => class(task),
    }
}

/// A list's `pick` for `task`: a review task takes the first unskipped candidate.
fn policy(list: &FrozenList, task: &Task) -> ListPolicy {
    match task.spec.kind {
        TaskKind::Review => ListPolicy::First,
        _ => list.pick,
    }
}

/// Decision 9a for a role: the list's snapshot (each candidate's skip reason beside
/// it), the route chosen (`None`: every candidate was skipped, so today's resolution
/// holds), the list's `pick` and the session's rotation position.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RolePick {
    pub route: Option<Route>,
    pub candidates: Vec<RoutingCandidate>,
    pub pick: ListPolicy,
    pub rotation: u32,
}

/// Decision 9a for a role (`scout`, `decider`, `planner`, `orchestrator`): session
/// `rotation`'s candidate (0-based, in start order). `first` takes the first one
/// installed, `spread` the first installed from `rotation % n` on, cycling; a
/// candidate without an effort takes `effort`. `None` with no list.
pub fn role(
    list: &FrozenList,
    rotation: u32,
    effort: Effort,
    installed: &Installed,
) -> Option<RolePick> {
    let n = list.candidates.len();
    if n == 0 {
        return None;
    }
    let start = match list.pick {
        ListPolicy::First => 0,
        ListPolicy::Spread => rotation as usize % n,
    };
    let usable = |k: usize| !missing(installed, list.candidates[k].runtime);
    let chosen = (0..n).map(|d| (start + d) % n).find(|&k| usable(k));
    let candidates: Vec<RoutingCandidate> = (0..n)
        .map(|k| RoutingCandidate {
            route: list.candidates[k].route(effort.clone()),
            skipped_reason: match (usable(k), Some(k) == chosen) {
                (false, _) => Some(NOT_INSTALLED.to_string()),
                (true, false) => Some(EARLIER_TAKEN.to_string()),
                (true, true) => None,
            },
        })
        .collect();
    Some(RolePick {
        route: chosen.map(|k| candidates[k].route.clone()),
        candidates,
        pick: list.pick,
        rotation,
    })
}

/// Whether `new`, `old` re-sized, picks again (review m2, ruling T10a-4: one rule for
/// an amend and a decider's raise): `old` took a list's candidate and `new` another list.
pub fn repicks(old: &Task, new: &Task) -> bool {
    let listed = old.list_pick.as_ref().is_some_and(|p| p.chosen.is_some());
    listed && list_key(old) != list_key(new)
}

/// The list of the task's class: hub, S, else M (a task rung 3 raised to L too).
pub fn class_list<'a>(lists: &'a RouteListsFrozen, task: &Task) -> &'a FrozenList {
    match class(task) {
        0 => &lists.s,
        1 => &lists.m,
        _ => &lists.hub,
    }
}

/// The task's class as an index: 0 S, 1 M, 2 hub.
fn class(task: &Task) -> usize {
    match (task.hub, task.size) {
        (true, _) => 2,
        (false, Size::S) => 0,
        _ => 1,
    }
}

/// The class's frozen default effort, for a candidate that names none.
fn class_effort(limits: &RunLimits, task: &Task) -> Effort {
    let r = &limits.class_routes;
    [r.s.effort.clone(), r.m.effort.clone(), r.hub.effort.clone()][class(task)].clone()
}

/// Whether the plan leaves the task's route to its class list: its route names no
/// model and no strength (a runtime or an effort alone narrows the pick).
fn takes_list(task: &Task) -> bool {
    let given = &task.spec.route;
    given.model.is_none() && given.strength.is_none()
}

/// Candidate `k`'s route for `task`: the plan's effort, else the candidate's, else
/// the class's.
fn route_for(limits: &RunLimits, task: &Task, list: &FrozenList, k: usize) -> Route {
    let mut route = list.candidates[k].route(class_effort(limits, task));
    if let Some(effort) = &task.spec.route.effort {
        route.effort = effort.clone();
    }
    route
}

fn snapshot(
    limits: &RunLimits,
    task: &Task,
    list: &FrozenList,
    reasons: &[Option<String>],
) -> Vec<RoutingCandidate> {
    (reasons.iter().enumerate())
        .map(|(k, reason)| RoutingCandidate {
            route: route_for(limits, task, list, k),
            skipped_reason: reason.clone(),
        })
        .collect()
}

/// The indexes of the unfinished tasks `ids` names: an edit batch's added tasks and
/// re-routed amends.
fn targets(tasks: &[Task], ids: &BTreeSet<String>) -> BTreeSet<usize> {
    (tasks.iter().enumerate())
        .filter(|(_, t)| ids.contains(t.id()) && !t.state.is_finished())
        .map(|(i, _)| i)
        .collect()
}

fn overlap(a: &Task, b: &Task) -> bool {
    any_intersect(&a.spec.owns, &b.spec.owns)
}

/// Whose route the overlap rule checks (ruling FW-5).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Mover {
    /// A task's worker. Validation rule 9 refuses overlapping tasks on different
    /// runtimes whatever their deps, so every unfinished overlapping task counts.
    Worker,
    /// A racer or a test writer: a transient session rule 9 never sees. A task that
    /// waits on the routed one never runs beside it and does not count (ruling FW-1).
    Transient,
}

/// Ruling FW-5: the tasks that declare they wait on task `i`, directly or through other
/// declared deps. An implicit `owns` dependency is left out: it links tasks on one
/// runtime only, so the move it would allow dissolves it.
fn declared_dependents(tasks: &[Task], i: usize) -> BTreeSet<usize> {
    let mut found = BTreeSet::new();
    let mut frontier = vec![i];
    while let Some(k) = frontier.pop() {
        let id = tasks[k].id();
        for (j, t) in tasks.iter().enumerate() {
            if j != i && t.spec.deps.iter().any(|d| d == id) && found.insert(j) {
                frontier.push(j);
            }
        }
    }
    found
}

/// The tasks the overlap rule counts against task `i` for `mover`: every other
/// unfinished one, but for a racer or a test writer not those that declare they wait on
/// it (rulings FW-1, FW-5).
fn alongside(tasks: &[Task], i: usize, mover: Mover) -> impl Iterator<Item = (usize, &Task)> {
    let waiting = match mover {
        Mover::Worker => BTreeSet::new(),
        Mover::Transient => declared_dependents(tasks, i),
    };
    (tasks.iter().enumerate())
        .filter(move |(j, t)| *j != i && !t.state.is_finished() && !waiting.contains(j))
}

/// The targets that take their class list, grouped by `owns` intersection within a
/// class (contract rule 22: two runtimes never share overlapping `owns`), in plan order
/// of each group's first task.
fn groups(tasks: &[Task], pending: &BTreeSet<usize>) -> Vec<Vec<usize>> {
    let mut groups: Vec<Vec<usize>> = Vec::new();
    for &i in pending {
        let joins: Vec<usize> = (groups.iter().enumerate())
            .filter(|(_, g)| {
                let first = &tasks[g[0]];
                list_key(first) == list_key(&tasks[i])
                    && g.iter().any(|&j| overlap(&tasks[j], &tasks[i]))
            })
            .map(|(k, _)| k)
            .collect();
        let mut merged = vec![i];
        for k in joins.into_iter().rev() {
            merged.extend(groups.remove(k));
        }
        merged.sort_unstable();
        groups.push(merged);
    }
    groups.sort_by_key(|g| g[0]);
    groups
}

/// Why candidate `c` cannot route `group`: not installed; a member names another
/// runtime; or an unfinished task already routed on another runtime overlaps a member.
fn skip(
    tasks: &[Task],
    pending: &BTreeSet<usize>,
    group: &[usize],
    runtime: Runtime,
    installed: &Installed,
) -> Option<String> {
    if missing(installed, runtime) {
        return Some(NOT_INSTALLED.to_string());
    }
    let named = (group.iter()).find_map(|&i| tasks[i].spec.route.runtime.filter(|r| *r != runtime));
    if let Some(named) = named {
        return Some(format!("the task names the runtime {named}"));
    }
    // Ruling FW-5: a worker's pick, which rule 9 re-validates, exempts no dependent.
    let held = (tasks.iter().enumerate())
        .filter(|(j, t)| !pending.contains(j) && !t.state.is_finished())
        .filter(|(_, t)| t.route.runtime != runtime)
        .any(|(_, t)| group.iter().any(|&i| overlap(&tasks[i], t)));
    held.then(|| OVERLAPPING_OWNS.to_string())
}

/// A `spread` group's place in the rotation: an unfinished task of its class that the
/// list already routed and that overlaps a member gives its candidate and slot (an
/// added task joins its overlap group); otherwise the class's next slot.
fn joined(tasks: &[Task], pending: &BTreeSet<usize>, group: &[usize]) -> Option<(usize, u32)> {
    let c = list_key(&tasks[group[0]]);
    (tasks.iter().enumerate())
        .filter(|(j, t)| !pending.contains(j) && !t.state.is_finished() && list_key(t) == c)
        .filter(|(_, t)| group.iter().any(|&i| overlap(&tasks[i], t)))
        .find_map(|(_, t)| {
            let p = t.list_pick.as_ref()?;
            Some((p.chosen? as usize, p.slot?))
        })
}

/// Decision 9a: routes the `targets` by their lists ([`task_list`]: research and review
/// tasks by the role lists, review tasks always `first`). A target whose plan names a
/// model or a strength keeps its route, with the list snapshotted for the history; the
/// others, grouped by overlapping `owns`, take per group the first unskipped candidate
/// (`first`), or round-robin from their rotation slot (`spread`). The chosen candidate
/// becomes the task's route, and its reviewer is chosen again for it. With every
/// candidate skipped the route stays today's resolution. A class with no list is left
/// exactly as it is.
pub fn pick(
    limits: &RunLimits,
    roster: &[ModelEntry],
    tasks: &mut [Task],
    targets: &BTreeSet<usize>,
    installed: &Installed,
) {
    let lists = &limits.route_lists;
    let mut pending = BTreeSet::new();
    for &i in targets {
        tasks[i].list_pick = None;
        // Research and review tasks route by the role lists (ruling RL-4).
        let list = task_list(lists, &tasks[i]);
        if list.is_empty() {
            continue;
        }
        if takes_list(&tasks[i]) {
            pending.insert(i);
            continue;
        }
        let reasons: Vec<Option<String>> = (list.candidates.iter())
            .map(|c| missing(installed, c.runtime).then(|| NOT_INSTALLED.to_string()))
            .collect();
        tasks[i].list_pick = Some(ListPick {
            candidates: snapshot(limits, &tasks[i], list, &reasons),
            chosen: None,
            pick: policy(list, &tasks[i]),
            slot: None,
        });
    }
    let mut next_slot = [0u32; 5];
    for t in tasks.iter() {
        if let Some(slot) = t.list_pick.as_ref().and_then(|p| p.slot) {
            next_slot[list_key(t)] = next_slot[list_key(t)].max(slot + 1);
        }
    }
    for group in groups(tasks, &pending) {
        let list = task_list(lists, &tasks[group[0]]);
        let n = list.candidates.len();
        let key = list_key(&tasks[group[0]]);
        // Ruling T10a-2: a group with a runtime-only route takes that runtime's first
        // unskipped candidate and no rotation slot (`skip` narrows it to the runtime).
        let named = (group.iter()).any(|&i| tasks[i].spec.route.runtime.is_some());
        let (start, slot) = match policy(list, &tasks[group[0]]) {
            ListPolicy::First => (0, None),
            ListPolicy::Spread if named => (0, None),
            ListPolicy::Spread => match joined(tasks, &pending, &group) {
                Some((k, slot)) => (k, Some(slot)),
                None => {
                    let slot = next_slot[key];
                    next_slot[key] += 1;
                    (slot as usize % n, Some(slot))
                }
            },
        };
        let reasons: Vec<Option<String>> = (list.candidates.iter())
            .map(|c| skip(tasks, &pending, &group, c.runtime, installed))
            .collect();
        let chosen = (0..n)
            .map(|d| (start + d) % n)
            .find(|&k| reasons[k].is_none());
        for &i in &group {
            let candidates = snapshot(limits, &tasks[i], list, &reasons);
            let route = chosen.map(|k| route_for(limits, &tasks[i], list, k));
            let task = &mut tasks[i];
            if let Some(route) = route {
                task.review_route = (task.review_level)
                    .map(|level| review_route(lists, roster, &route, level, installed));
                task.route = route;
            }
            task.list_pick = Some(ListPick {
                candidates,
                chosen: chosen.map(|k| k as u32),
                pick: policy(list, task),
                slot,
            });
        }
        for i in &group {
            pending.remove(i);
        }
    }
}

/// [`pick`] over every task of a build. What is installed is not known yet (the
/// start's probe is recorded on the run later, `run.orch.installed`).
pub fn pick_all(limits: &RunLimits, roster: &[ModelEntry], tasks: &mut [Task]) {
    let all = (0..tasks.len()).collect();
    pick(limits, roster, tasks, &all, &Installed::new());
}

/// [`pick`] for the unfinished tasks of `run` that `ids` names (an edit batch's added
/// tasks and re-routed amends), over what the run's start found installed.
pub fn pick_named(run: &mut Run, ids: &BTreeSet<String>) {
    let targets = targets(&run.tasks, ids);
    let installed = &run.orch.installed;
    pick(
        &run.limits,
        &run.roster,
        &mut run.tasks,
        &targets,
        installed,
    );
}

/// `get_context`'s `limits.routes` (decision 9a: the planners see the lists): each
/// non-empty list by its table name, with its `pick` and candidates (an `effort` only
/// where the list sets one). `None` with no list, so the context is as before.
pub fn context_routes(lists: &RouteListsFrozen) -> Option<serde_json::Value> {
    let named = (lists.named().into_iter()).filter(|(_, list)| !list.is_empty());
    let entries: serde_json::Map<String, serde_json::Value> = named
        .map(|(name, list)| {
            let candidates: Vec<serde_json::Value> = (list.candidates.iter())
                .map(|c| {
                    let mut entry = serde_json::json!({
                        "runtime": c.runtime, "model": c.model, "strength": c.strength,
                    });
                    if let Some(effort) = &c.effort {
                        entry["effort"] = serde_json::json!(effort);
                    }
                    entry
                })
                .collect();
            let list = serde_json::json!({"pick": list.pick.label(), "candidates": candidates});
            (name.to_string(), list)
        })
        .collect();
    (!entries.is_empty()).then_some(serde_json::Value::Object(entries))
}

/// Decision 9a's reviewer from the `review` list, `None` with no list: the first
/// candidate that is installed, did not fail in this task (`failed`, ruling RL-1), is
/// on the other runtime than `author`, and at or above
/// both the author's strength and the level's, at its own effort or else the level's;
/// with the whole list, each candidate's skip reason beside it. The route is `None`
/// when none qualifies (the caller takes `pick_reviewer`).
pub fn reviewer(
    lists: &RouteListsFrozen,
    author: &Route,
    level: ReviewLevel,
    installed: &Installed,
    failed: &[Route],
) -> Option<(Option<Route>, Vec<RoutingCandidate>)> {
    let list = &lists.review;
    if list.is_empty() {
        return None;
    }
    let (floor, effort) = match level {
        ReviewLevel::Small => (Strength::Fast, Effort::LOW),
        ReviewLevel::Medium => (author.strength, Effort::MEDIUM),
        ReviewLevel::Frontier => (Strength::Frontier, Effort::HIGH),
    };
    let required = floor.max(author.strength);
    let mut chosen = None;
    let candidates = (list.candidates.iter())
        .map(|c| {
            let route = c.route(effort.clone());
            let reason = if missing(installed, c.runtime) {
                Some(NOT_INSTALLED)
            } else if failed_in(failed, &route) {
                Some(FAILED_IN_TASK)
            } else if c.runtime == author.runtime {
                Some(AUTHOR_RUNTIME)
            } else if c.strength < required {
                Some(BELOW_STRENGTH)
            } else {
                None
            };
            if reason.is_none() && chosen.is_none() {
                chosen = Some(route.clone());
            }
            RoutingCandidate {
                route,
                skipped_reason: reason.map(String::from),
            }
        })
        .collect();
    Some((chosen, candidates))
}

/// The reviewer's route for `author` at `level`: [`reviewer`]'s when the `review` list
/// has a qualifying candidate, else `roster::pick_reviewer`'s.
pub fn review_route(
    lists: &RouteListsFrozen,
    roster: &[ModelEntry],
    author: &Route,
    level: ReviewLevel,
    installed: &Installed,
) -> Route {
    reviewer(lists, author, level, installed, &[])
        .and_then(|(route, _)| route)
        .unwrap_or_else(|| pick_reviewer(roster, author, level))
}

/// A task's reviewer as resolution forecasts it (decision 9a: the `review` list first),
/// before anything is known installed.
pub fn forecast(
    limits: &RunLimits,
    roster: &[ModelEntry],
    author: &Route,
    level: ReviewLevel,
) -> Route {
    review_route(
        &limits.route_lists,
        roster,
        author,
        level,
        &Installed::new(),
    )
}

#[path = "route_pick_step.rs"]
mod step;
pub(crate) use step::{escalate_for, installed_roster};
pub use step::{
    every_route_failed, next_candidate, racer_route, rung2_route, writer_route,
    writer_route_failed, writer_step,
};

#[cfg(test)]
#[path = "route_pick_tests.rs"]
mod tests;
