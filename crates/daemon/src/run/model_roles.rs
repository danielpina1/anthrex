//! Milestone 9.8 (MR §3.1, §3.4): the run's resolved role table ([`RunModels`], frozen
//! at start, decision 9) and the rules that read it: which row a task takes (decision
//! 10), the reviewer (decision 27, D3), the racer and the test writer (decision 28).
//! Task M9.8.7a first moved here, unchanged, the parts of `route_pick.rs` and
//! `route_pick_step.rs` that outlive them (preflight ruling F13): what a start found
//! installed, the routes that failed in a task (ruling RL-1), and the overlap rule's
//! movers (rulings FW-1, FW-5). Pure (design decision 1).

use std::collections::{BTreeMap, BTreeSet};

use proto::models::{BrainstormChoice, ModelRef, ModelTable, Role, RoleChoice};
use proto::{CatalogSource, Effort, ModelCatalog, Route, Runtime, Size, Strength, TaskKind};
use serde::{Deserialize, Serialize};

use super::globs::any_intersect;
use super::model::{Run, Task};

/// The efforts a catalog reported for one model (decision 21), kept with the run so
/// escalation (M9.8.8) can step along them.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModelEfforts {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub efforts: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default: Option<String>,
}

/// Decision 3's resolved table of a run: every role resolved (repository, then the
/// global table, then the built-in), frozen at start (decision 9).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RunModels {
    /// Every role of `Role::all()`, resolved.
    pub rows: BTreeMap<Role, RoleChoice>,
    pub brainstorm: BrainstormChoice,
    /// The efforts the catalog reported for each row's model and fallback.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub efforts: BTreeMap<ModelRef, ModelEfforts>,
}

/// Decision 27's run-log line when the reviewer row has nothing but the author's model.
fn same_model_line(model: &ModelRef) -> String {
    format!(
        "reviewer: {} reviews work by the same model; set an \"if it struggles\" model for the reviewer in C-b S",
        model.label()
    )
}

/// Whether `model` is exactly `route`'s model: the same runtime and id (`default` equal
/// only to `default`).
fn is_model(model: &ModelRef, route: &Route) -> bool {
    model.runtime == route.runtime && model.route_model() == route.model
}

/// Whether `a` and `b` are the same model (runtime and id).
fn is_model_route(a: &Route, b: &Route) -> bool {
    a.runtime == b.runtime && a.model == b.model
}

impl RunModels {
    /// Every role resolved by `config::models::resolve` (MR §3.4), and the brainstorm
    /// pair.
    pub fn resolve(global: &ModelTable, repo: Option<&ModelTable>) -> RunModels {
        let rows = (Role::all())
            .map(|role| (role, config::models::resolve(role, repo, global)))
            .collect();
        RunModels {
            rows,
            brainstorm: config::models::resolve_brainstorm(repo, global),
            efforts: BTreeMap::new(),
        }
    }

    /// Decision 21: fills `efforts` from the `Live` and `Cached` catalogs, else, for a
    /// runtime with neither, from its `Builtin` catalog or the built-in list (M9.8.8
    /// fix round 1; a `Builtin` list validates nothing), and replaces a row's effort its model does not offer
    /// with the model's default effort; the run-log lines, one per replaced row. A model
    /// the catalog does not list, or lists without efforts, keeps its row as configured.
    pub fn validate(&mut self, catalogs: &[ModelCatalog]) -> Vec<String> {
        let discovered =
            |c: &&ModelCatalog| matches!(c.source, CatalogSource::Live | CatalogSource::Cached);
        let known = |m: &ModelRef| catalogs.iter().filter(discovered).find_map(|c| c.find(m));
        // M9.8.8 fix round 1 (controller ruling): a runtime with no discovered catalog
        // takes its `Builtin` catalog's lists, else the built-in list's, for escalation
        // only (decision 29); they validate nothing.
        let builtin: Vec<ModelCatalog> = [Runtime::Claude, Runtime::Codex]
            .into_iter()
            .filter(|r| !catalogs.iter().filter(discovered).any(|c| c.runtime == *r))
            .map(|r| match catalogs.iter().find(|c| c.runtime == r) {
                Some(c) => c.clone(),
                None => crate::models::builtin::catalog(r, String::new(), 0, String::new()),
            })
            .collect();
        let listed = |m: &ModelRef| known(m).or_else(|| builtin.iter().find_map(|c| c.find(m)));
        let mut lines = Vec::new();
        for choice in self.rows.values_mut() {
            for m in std::iter::once(&choice.model).chain(&choice.fallback) {
                if let Some(found) = listed(m) {
                    let efforts = ModelEfforts {
                        efforts: found.efforts.clone(),
                        default: found.default_effort.clone(),
                    };
                    self.efforts.insert(m.clone(), efforts);
                }
            }
            let Some(found) = known(&choice.model) else {
                continue;
            };
            let Some(effort) = choice.effort.as_deref() else {
                continue;
            };
            if found.efforts.is_empty() || found.efforts.iter().any(|e| e == effort) {
                continue;
            }
            let using = found
                .default_effort
                .as_deref()
                .unwrap_or("the model's default");
            lines.push(format!(
                "effort '{effort}' not offered by {}; using {using}",
                found.id
            ));
            choice.effort = found.default_effort.clone();
        }
        lines
    }

    /// The resolved row of `role`.
    pub fn choice(&self, role: Role) -> &RoleChoice {
        self.rows
            .get(&role)
            .expect("a run's models resolve every role")
    }

    /// The row's model at its effort.
    pub fn route(&self, role: Role) -> Route {
        let choice = self.choice(role);
        RunModels::route_of(&choice.model, choice.effort.as_deref())
    }

    /// `model` at `effort` (`None`: the model's default) as a route.
    pub fn route_of(model: &ModelRef, effort: Option<&str>) -> Route {
        Route {
            runtime: model.runtime,
            model: model.route_model().to_string(),
            // removed in M9.8.14; no rule reads it after M9.8.8
            strength: Strength::Standard,
            effort: Effort::of(effort),
        }
    }

    /// Decision 10: the row task `task` takes.
    pub fn task_role(task: &Task) -> Role {
        RunModels::role_of(task.spec.kind, task.hub, task.size)
    }

    /// Decision 10 by the task's fields: research and review tasks take their rows; a
    /// hub task `implementer.hub`; size `S` `implementer.small`; anything else (`M`,
    /// and an `L` rung 3 raised) `implementer.medium`.
    pub fn role_of(kind: TaskKind, hub: bool, size: Size) -> Role {
        match (kind, hub, size) {
            (TaskKind::Research, ..) => Role::Research,
            (TaskKind::Review, ..) => Role::Reviewer,
            (_, true, _) => Role::ImplementerHub,
            (_, false, Size::S) => Role::ImplementerSmall,
            _ => Role::ImplementerMedium,
        }
    }

    /// Decision 27 (D3): the reviewer row's model, unless it is exactly the author's;
    /// then the row's fallback at its default effort; with no fallback other than the
    /// author's model, the row's model and the run-log line saying so.
    pub fn reviewer_route(&self, author: &Route) -> (Route, Option<String>) {
        let choice = self.choice(Role::Reviewer);
        if !is_model(&choice.model, author) {
            return (self.route(Role::Reviewer), None);
        }
        match choice.fallback.as_ref().filter(|f| !is_model(f, author)) {
            Some(fallback) => (RunModels::route_of(fallback, None), None),
            None => (
                self.route(Role::Reviewer),
                Some(same_model_line(&choice.model)),
            ),
        }
    }

    /// The model `route` runs, as the table names it (`<runtime>:<id>`, or `default`).
    pub fn model_of(route: &Route) -> ModelRef {
        let id = (!route.model.is_empty()).then(|| route.model.clone());
        ModelRef {
            runtime: route.runtime,
            id,
        }
    }

    /// `route`'s model's label ([`RunModels::model_of`]).
    pub fn label_of(route: &Route) -> String {
        RunModels::model_of(route).label()
    }

    /// Whether routes `a` and `b` run the same model (runtime and id).
    pub fn same_model(a: &Route, b: &Route) -> bool {
        is_model_route(a, b)
    }

    /// Decision 27's run-log line for a reviewer on `author`'s own model.
    pub fn same_model_line(author: &Route) -> String {
        same_model_line(&RunModels::model_of(author))
    }

    /// Decision 28: a racing task's second lane takes its row's fallback at its default
    /// effort, else the task's own route.
    pub fn racer_route(&self, role: Role, current: &Route) -> Route {
        match &self.choice(role).fallback {
            Some(fallback) => RunModels::route_of(fallback, None),
            None => current.clone(),
        }
    }
}

/// Milestone 9.8 (D2, MR §7) for a role with no run (a decider, the onboarding scout):
/// `choice`'s model at its effort, unless `installed` records its runtime missing and
/// the row's fallback is on the other runtime, not missing: then the fallback at its
/// default effort, and the row's route it moved from. Never a model the row does not
/// name; `installed` empty counts everything installed.
pub fn row_route_over(choice: &RoleChoice, installed: &Installed) -> (Route, Option<Route>) {
    let own = RunModels::route_of(&choice.model, choice.effort.as_deref());
    let fallback = (choice.fallback.as_ref()).filter(|f| {
        missing(installed, own.runtime)
            && f.runtime != own.runtime
            && !missing(installed, f.runtime)
    });
    match fallback {
        Some(f) => (RunModels::route_of(f, None), Some(own)),
        None => (own, None),
    }
}

/// A skip reason: the overlap rule holds the route off (decision 28; rulings FW-1, FW-5).
pub const OVERLAPPING_OWNS: &str = "overlapping owns";
pub const NOT_INSTALLED: &str = super::orch::roles::NOT_INSTALLED;

/// Ruling RL-1: the route of a session of this task that ended for an environment reason.
pub const FAILED_IN_TASK: &str = "failed in this task";

/// What a run's start found installed (`run.orch.installed`, by runtime label). A
/// runtime it does not name (a plan-file run records nothing) counts as installed.
pub type Installed = BTreeMap<String, bool>;

pub(crate) fn missing(installed: &Installed, runtime: Runtime) -> bool {
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

pub(crate) fn overlap(a: &Task, b: &Task) -> bool {
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
pub(crate) fn alongside(
    tasks: &[Task],
    i: usize,
    mover: Mover,
) -> impl Iterator<Item = (usize, &Task)> {
    let waiting = match mover {
        Mover::Worker => BTreeSet::new(),
        Mover::Transient => declared_dependents(tasks, i),
    };
    (tasks.iter().enumerate())
        .filter(move |(j, t)| *j != i && !t.state.is_finished() && !waiting.contains(j))
}

/// Decision 27 at a reviewer's launch, with ruling RL-1: [`RunModels::reviewer_route`]
/// against `author`; when that route failed in this task, the row's other model (its
/// fallback at its default effort, or its model at its effort), unless that one failed
/// in this task too or is the author's.
pub fn reviewer_at_launch(
    models: &RunModels,
    author: &Route,
    failed: &[Route],
) -> (Route, Option<String>) {
    let (route, line) = models.reviewer_route(author);
    if !failed_in(failed, &route) {
        return (route, line);
    }
    let choice = models.choice(Role::Reviewer);
    let own = models.route(Role::Reviewer);
    let fallback = (choice.fallback.as_ref()).map(|f| RunModels::route_of(f, None));
    let other = (std::iter::once(own).chain(fallback))
        .find(|r| !(failed_in(failed, r) || is_model_route(r, author)));
    other.map_or((route, line), |r| (r, None))
}

/// Decision 28 (rulings FW-1, FW-5, T10a-6): whether a lane on `runtime` may run beside
/// task `i`: the runtime is the task's own, or it is installed for the run and no
/// unfinished task that `mover`'s rule counts ([`alongside`]) on another runtime than
/// `runtime` overlaps the task's `owns` (contract rule 22: two runtimes never share
/// overlapping `owns`). `open_roster`'s predicate, extracted (ruling F40).
pub(crate) fn runtime_open(run: &Run, i: usize, runtime: Runtime, mover: Mover) -> bool {
    let task = &run.tasks[i];
    let held = || {
        alongside(&run.tasks, i, mover).any(|(_, t)| t.route.runtime != runtime && overlap(t, task))
    };
    runtime == task.route.runtime || !(missing(&run.orch.installed, runtime) || held())
}

/// Decision 28: a paired task's test writer: the `test_writer` row, unless it runs on
/// another runtime than the task's that [`runtime_open`] holds it off; then the task's
/// own route.
pub fn writer_route(run: &Run, i: usize) -> Route {
    let route = run.limits.models().route(Role::TestWriter);
    lane_or_own(run, i, route)
}

/// Decision 28: a racing task's second racer: its row's fallback, else its own route
/// ([`RunModels::racer_route`]), held to the overlap rule as the test writer is.
pub fn racer_route(run: &Run, i: usize) -> Route {
    let task = &run.tasks[i];
    let role = RunModels::task_role(task);
    let route = run.limits.models().racer_route(role, &task.route);
    lane_or_own(run, i, route)
}

fn lane_or_own(run: &Run, i: usize, route: Route) -> Route {
    match runtime_open(run, i, route.runtime, Mover::Transient) {
        true => route,
        false => run.tasks[i].route.clone(),
    }
}

#[cfg(test)]
#[path = "model_roles_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "model_roles_tests_efforts.rs"]
mod tests_efforts;
