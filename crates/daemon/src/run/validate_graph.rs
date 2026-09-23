//! The cross-task rules, split out of `validate.rs` by rule family: task count, ids,
//! the dependency graph (decision 12), the L rule for touched tasks (decisions 9 and
//! 13), runtime overlap (decision 11) and the edit area (decision 12), plus implicit
//! dependencies (decision 41). Pure — no `std::fs`, `std::process`, `std::thread`,
//! `tokio` or `std::time::SystemTime` (design decision 2).

use std::collections::{BTreeMap, BTreeSet};

use proto::{Size, TaskState};

use super::globs::{any_intersect, inside_area, intersects, literal_prefix};
use super::model::{Profile, Task};
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
/// apply only to `touched` tasks (decision 13's L exemption: a task raised to L by
/// rung 3 must not block unrelated edits). `max_tasks` counts every task that is not
/// cancelled. `_profile` is unused: the profile-dependent rules run per task, in
/// `resolve_task`.
pub fn validate_tasks(
    tasks: &[Task],
    touched: &BTreeSet<String>,
    scope: &EditScope,
    max_tasks: u32,
    _profile: &Profile,
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
                Some(d) if is_touched && d.state == TaskState::Cancelled => {
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
        for earlier in tasks[..i].iter().filter(|t| is_active(t)) {
            if earlier.route.runtime == task.route.runtime {
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
                        earlier.route.runtime,
                        task.route.runtime
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

/// Every dependency cycle among unfinished tasks, each reported once, starting from its
/// member that comes first in plan order (`deps: cycle t1 -> t2 -> t1`). Uses Tarjan's
/// strongly connected components; a component of two or more tasks, or one task that
/// depends on itself, is a cycle.
fn cycles(tasks: &[Task]) -> Vec<PlanError> {
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
            t.spec
                .deps
                .iter()
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

/// Decision 41's implicit dependencies at plan time, when no task has started: for two
/// unfinished tasks on the same runtime whose `owns` intersect, the later in plan order
/// waits for the earlier — unless the earlier already depends (declared, transitively)
/// on the later, which would deadlock them, or the later already declares it.
pub fn implicit_deps(tasks: &[Task]) -> Vec<Vec<String>> {
    let by_id: BTreeMap<&str, &Task> = tasks.iter().rev().map(|t| (t.id(), t)).collect();
    let depends_on = |from: &str, to: &str| -> bool {
        let mut stack = vec![from];
        let mut seen = BTreeSet::new();
        while let Some(id) = stack.pop() {
            if !seen.insert(id) {
                continue;
            }
            if let Some(t) = by_id.get(id) {
                for d in &t.spec.deps {
                    if d == to {
                        return true;
                    }
                    stack.push(d.as_str());
                }
            }
        }
        false
    };
    tasks
        .iter()
        .enumerate()
        .map(|(i, task)| {
            if !is_active(task) {
                return Vec::new();
            }
            tasks[..i]
                .iter()
                .filter(|e| is_active(e))
                .filter(|e| e.route.runtime == task.route.runtime)
                .filter(|e| any_intersect(&e.spec.owns, &task.spec.owns))
                .filter(|e| !task.spec.deps.iter().any(|d| d == e.id()))
                .filter(|e| !depends_on(e.id(), task.id()))
                .map(|e| e.id().to_string())
                .collect()
        })
        .collect()
}
