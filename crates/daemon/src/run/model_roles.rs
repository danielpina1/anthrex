//! Milestone 9.8 (MR §3.1, §3.4): the run's resolved role table and the rules that read
//! it. Task M9.8.7a first moves here, unchanged, the parts of `route_pick.rs` and
//! `route_pick_step.rs` that outlive them (preflight ruling F13): what a start found
//! installed, the routes that failed in a task (ruling RL-1), and the overlap rule's
//! movers (rulings FW-1, FW-5). Pure (design decision 1).

use std::collections::{BTreeMap, BTreeSet};

use proto::{ModelEntry, Route, Runtime};

use super::globs::any_intersect;
use super::model::Task;

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

/// Milestone 9.7 decision 16: the `roster` entries whose runtime `installed` does not
/// record as missing (the installed skip alone; `reach`'s forecast has no task).
pub(crate) fn installed_roster(roster: &[ModelEntry], installed: &Installed) -> Vec<ModelEntry> {
    (roster.iter())
        .filter(|e| !missing(installed, e.runtime))
        .cloned()
        .collect()
}
