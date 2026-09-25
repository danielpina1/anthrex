//! The cross-task rules, split out of `validate.rs` by rule family: task count, ids,
//! the dependency graph (decision 12), the L rule for touched tasks (decisions 9 and
//! 13), runtime overlap (decision 11) and the edit area (decision 12), plus implicit
//! dependencies (decision 41). Pure — no `std::fs`, `std::process`, `std::thread`,
//! `tokio` or `std::time::SystemTime` (design decision 2).

use std::collections::{BTreeMap, BTreeSet};

use proto::{Runtime, Size, TaskState};

use super::globs::{any_intersect, inside_area, intersects, literal_prefix};
use super::model::Task;
use super::plan::PlanError;

/// Where an edit batch may reach: the whole run, or (for M9's sub-planners) only an
/// area of the tree.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EditScope {
    Run,
    Area { globs: Vec<String> },
}

fn is_active(task: &Task) -> bool {
    !task.state.is_finished()
}

/// An area glob is a literal path or `<literal>/**` (decision 12).
fn is_valid_area_glob(glob: &str) -> bool {
    let literal = glob.strip_suffix("/**").unwrap_or(glob);
    !literal.is_empty() && literal_prefix(literal).len() == literal.split('/').count()
}

/// The rules over the whole task list, decisions 9–13. Every task that is not merged or
/// cancelled is checked; the L rule, the cancelled-dependency rule and the area rule
/// apply only to `touched` tasks (the cancelled-dependency rule can be narrowed further
/// with [`validate_tasks_with`]) (decision 13's L exemption: a task raised to L by
/// rung 3 must not block unrelated edits). `max_tasks` counts every task that is not
/// cancelled. Rule 9 compares the runtimes the plan gives (`default_runtime` fills a
/// spec that names none) and applies to a pair only when the batch touches one of the
/// two: a runtime the engine escalated to (rung 2, `run retry`) is not the plan's, and
/// must not block every later edit (final review A-I1, the same reason as the L
/// exemption); the profile-dependent rules run per task, in `resolve_task`.
pub fn validate_tasks(
    tasks: &[Task],
    touched: &BTreeSet<String>,
    scope: &EditScope,
    max_tasks: u32,
    default_runtime: Runtime,
) -> Vec<PlanError> {
    validate_tasks_with(tasks, touched, None, scope, max_tasks, default_runtime)
}

/// The runtime the plan gives `task`: its spec's, or `default_runtime` when the spec
/// names none or names one a task cannot run on (which `resolve_task` reports).
fn planned_runtime(task: &Task, default_runtime: Runtime) -> Runtime {
    match task.spec.route.runtime {
        Some(runtime) if runtime != Runtime::Shell => runtime,
        _ => default_runtime,
    }
}

/// [`validate_tasks`], with the cancelled-dependency rule limited to `added_deps`
/// (`(task, dep)` pairs) when given. A plan edit passes the dependencies its batch adds,
/// so a task already `blocked(dep_cancelled)` stays editable (M8a.6 fix round 1, F3);
/// `None` checks every dependency of every touched task, as a new plan needs.
pub fn validate_tasks_with(
    tasks: &[Task],
    touched: &BTreeSet<String>,
    added_deps: Option<&BTreeSet<(String, String)>>,
    scope: &EditScope,
    max_tasks: u32,
    default_runtime: Runtime,
) -> Vec<PlanError> {
    let mut errors = Vec::new();
    let counted = tasks
        .iter()
        .filter(|t| t.state != TaskState::Cancelled)
        .count();
    if counted > max_tasks as usize {
        errors.push(PlanError::new(
            None,
            "tasks",
            "range",
            format!("{counted} tasks exceed max_tasks ({max_tasks})"),
        ));
    }
    let area = match scope {
        EditScope::Run => None,
        EditScope::Area { globs } => {
            for glob in globs.iter().filter(|g| !is_valid_area_glob(g)) {
                errors.push(PlanError::new(
                    None,
                    "area",
                    "12.1",
                    format!("{glob} must be <literal>/** or a literal path"),
                ));
            }
            Some(globs)
        }
    };

    let by_id: BTreeMap<&str, &Task> = tasks.iter().rev().map(|t| (t.id(), t)).collect();
    let mut seen = BTreeSet::new();
    for (i, task) in tasks.iter().enumerate() {
        let id = task.id();
        let first = seen.insert(id);
        if !is_active(task) {
            continue;
        }
        let is_touched = touched.contains(id);
        let e = |field: &str, rule: &str, message: String| {
            PlanError::new(Some(id), field, rule, message)
        };
        if !first {
            errors.push(e("id", "id", format!("{id} is used by an earlier task")));
        }
        for dep in &task.spec.deps {
            match by_id.get(dep.as_str()) {
                None => errors.push(e("deps", "12.1", format!("{dep} is not a task"))),
                Some(d)
                    if d.state == TaskState::Cancelled
                        && match added_deps {
                            None => is_touched,
                            Some(added) => added.contains(&(id.to_string(), dep.clone())),
                        } =>
                {
                    errors.push(e("deps", "12.1", format!("{dep} is cancelled")))
                }
                Some(_) => {}
            }
        }
        if is_touched && task.size == Size::L {
            errors.push(e(
                "size",
                "7.2.4",
                "L tasks are never executed; split the task (rule 7.2.4)".to_string(),
            ));
        }
        let runtime = planned_runtime(task, default_runtime);
        for earlier in tasks[..i].iter().filter(|t| is_active(t)) {
            let earlier_runtime = planned_runtime(earlier, default_runtime);
            if earlier_runtime == runtime || !(is_touched || touched.contains(earlier.id())) {
                continue;
            }
            let hit = earlier
                .spec
                .owns
                .iter()
                .find(|g| task.spec.owns.iter().any(|o| intersects(g, o)));
            if let Some(glob) = hit {
                errors.push(e(
                    "owns",
                    "9",
                    format!(
                        "overlaps task {}'s owns ({glob}) and the two tasks run on different runtimes ({}, {}) (rule 9)",
                        earlier.id(),
                        earlier_runtime,
                        runtime
                    ),
                ));
            }
        }
        if let Some(area) = area
            && is_touched
        {
            for glob in task.spec.owns.iter().filter(|g| !inside_area(g, area)) {
                errors.push(e(
                    "owns",
                    "12.1",
                    format!("{glob} is outside the area {}", area.join(", ")),
                ));
            }
        }
    }
    errors.extend(cycles(tasks));
    errors
}

/// Cycles in declared **plus implicit** dependencies (`Task.implicit_deps`), the graph
/// the scheduler actually waits on. `implicit_deps` never closes a cycle; this is the
/// backstop `build_run` runs after filling them (M8a.5 review finding 1).
pub fn combined_cycles(tasks: &[Task]) -> Vec<PlanError> {
    cycles_over(tasks, true)
}

/// Every dependency cycle among unfinished tasks, each reported once, starting from its
/// member that comes first in plan order (`deps: cycle t1 -> t2 -> t1`). Uses Tarjan's
/// strongly connected components; a component of two or more tasks, or one task that
/// depends on itself, is a cycle.
fn cycles(tasks: &[Task]) -> Vec<PlanError> {
    cycles_over(tasks, false)
}

fn cycles_over(tasks: &[Task], with_implicit: bool) -> Vec<PlanError> {
    let active: Vec<&Task> = tasks.iter().filter(|t| is_active(t)).collect();
    let index: BTreeMap<&str, usize> = active
        .iter()
        .enumerate()
        .rev()
        .map(|(i, t)| (t.id(), i))
        .collect();
    let edges: Vec<Vec<usize>> = active
        .iter()
        .map(|t| {
            let implicit: &[String] = if with_implicit { &t.implicit_deps } else { &[] };
            t.spec
                .deps
                .iter()
                .chain(implicit)
                .filter_map(|d| index.get(d.as_str()).copied())
                .collect()
        })
        .collect();

    let mut components = Tarjan::new(&edges).run();
    for c in &mut components {
        c.sort_unstable();
    }
    components.sort_unstable_by_key(|c| c[0]);

    let mut errors = Vec::new();
    for comp in components {
        let start = comp[0];
        let cyclic = comp.len() > 1 || edges[start].contains(&start);
        if !cyclic {
            continue;
        }
        let members: BTreeSet<usize> = comp.iter().copied().collect();
        let path = path_back(start, &edges, &members);
        let names: Vec<&str> = path.iter().map(|&i| active[i].id()).collect();
        errors.push(PlanError::new(
            None,
            "deps",
            "12.1",
            format!("cycle {}", names.join(" -> ")),
        ));
    }
    errors
}

/// A path from `start` back to itself inside one strongly connected component,
/// following each task's deps in their written order (depth first).
fn path_back(start: usize, edges: &[Vec<usize>], members: &BTreeSet<usize>) -> Vec<usize> {
    let mut path = vec![start];
    let mut visited = BTreeSet::from([start]);
    let mut cursors = vec![0usize];
    while let Some(&node) = path.last() {
        let cursor = cursors.last_mut().expect("cursor per path node");
        let Some(&next) = edges[node].get(*cursor) else {
            path.pop();
            cursors.pop();
            continue;
        };
        *cursor += 1;
        if next == start {
            path.push(start);
            return path;
        }
        if members.contains(&next) && visited.insert(next) {
            path.push(next);
            cursors.push(0);
        }
    }
    vec![start, start]
}

struct Tarjan<'a> {
    edges: &'a [Vec<usize>],
    index: Vec<Option<usize>>,
    low: Vec<usize>,
    on_stack: Vec<bool>,
    stack: Vec<usize>,
    next: usize,
    out: Vec<Vec<usize>>,
}

impl<'a> Tarjan<'a> {
    fn new(edges: &'a [Vec<usize>]) -> Self {
        let n = edges.len();
        Tarjan {
            edges,
            index: vec![None; n],
            low: vec![0; n],
            on_stack: vec![false; n],
            stack: Vec::new(),
            next: 0,
            out: Vec::new(),
        }
    }

    fn run(mut self) -> Vec<Vec<usize>> {
        for v in 0..self.edges.len() {
            if self.index[v].is_none() {
                self.visit(v);
            }
        }
        self.out
    }

    fn visit(&mut self, v: usize) {
        self.index[v] = Some(self.next);
        self.low[v] = self.next;
        self.next += 1;
        self.stack.push(v);
        self.on_stack[v] = true;
        for &w in &self.edges[v] {
            match self.index[w] {
                None => {
                    self.visit(w);
                    self.low[v] = self.low[v].min(self.low[w]);
                }
                Some(iw) if self.on_stack[w] => self.low[v] = self.low[v].min(iw),
                Some(_) => {}
            }
        }
        if Some(self.low[v]) == self.index[v] {
            let mut comp = Vec::new();
            while let Some(w) = self.stack.pop() {
                self.on_stack[w] = false;
                comp.push(w);
                if w == v {
                    break;
                }
            }
            self.out.push(comp);
        }
    }
}

/// A task that has started, for decision 41: any state from `preparing` to
/// `merge_queue`, or `blocked` with a start commit (its worktree exists).
fn has_started(task: &Task) -> bool {
    matches!(
        task.state,
        TaskState::Preparing
            | TaskState::Working
            | TaskState::Proof
            | TaskState::Check
            | TaskState::Review
            | TaskState::MergeQueue
    ) || (task.state == TaskState::Blocked && task.start_commit.is_some())
}

/// Decision 41's implicit dependencies: for two unfinished tasks on the same runtime
/// whose `owns` intersect, a task that has not started waits for one that has; when
/// neither has started, the later in plan order waits for the earlier; when both have,
/// neither waits. A wait is skipped when the waiter already declares the other task, or
/// when the other task can already reach the waiter through declared deps **and the
/// implicit deps given so far** (M8a.5 review finding 1): that edge would close a cycle
/// and deadlock both. At plan time no task has started, so only the plan-order rule
/// applies; plan edits (M8a.6) recompute these after every batch.
pub fn implicit_deps(tasks: &[Task]) -> Vec<Vec<String>> {
    let index: BTreeMap<&str, usize> = tasks
        .iter()
        .enumerate()
        .rev()
        .map(|(i, t)| (t.id(), i))
        .collect();
    // Outgoing edges (a task -> what it waits for), declared first, implicit added as
    // they are decided.
    let mut edges: Vec<Vec<usize>> = tasks
        .iter()
        .map(|t| {
            t.spec
                .deps
                .iter()
                .filter_map(|d| index.get(d.as_str()).copied())
                .collect()
        })
        .collect();
    let reaches = |edges: &[Vec<usize>], from: usize, to: usize| -> bool {
        let mut stack = vec![from];
        let mut seen = BTreeSet::new();
        while let Some(n) = stack.pop() {
            if n == to {
                return true;
            }
            if seen.insert(n) {
                stack.extend(edges[n].iter().copied());
            }
        }
        false
    };
    let mut out: Vec<Vec<String>> = vec![Vec::new(); tasks.len()];
    for (i, task) in tasks.iter().enumerate() {
        if !is_active(task) {
            continue;
        }
        for (j, earlier) in tasks[..i].iter().enumerate() {
            if !is_active(earlier)
                || earlier.route.runtime != task.route.runtime
                || !any_intersect(&earlier.spec.owns, &task.spec.owns)
            {
                continue;
            }
            // (waiter, waited for)
            let (w, on) = match (has_started(earlier), has_started(task)) {
                (true, true) => continue,
                (false, true) => (j, i),
                _ => (i, j),
            };
            let on_id = tasks[on].id();
            if tasks[w].spec.deps.iter().any(|d| d == on_id) || reaches(&edges, on, w) {
                continue;
            }
            edges[w].push(on);
            out[w].push(on_id.to_string());
        }
    }
    out
}
