//! Decision 22: the affected set of a change (TT §3.3). Pure: the caller hands in the
//! changed paths and the module graph.

use std::collections::{BTreeMap, BTreeSet};

use proto::ModuleNames;

use super::{Affected, GraphState, ModuleGraph, TierProfile};
use crate::run::globs::{OwnsMatcher, path_module};

/// A matcher for `globs` with `owns`' rules (a literal entry covers everything below
/// it). `None` for an empty list; a glob validation would have refused matches
/// nothing, which only ever widens the set.
fn matcher(globs: &[String]) -> Option<OwnsMatcher> {
    (!globs.is_empty())
        .then(|| OwnsMatcher::new(globs).ok())
        .flatten()
}

fn matches(matcher: &Option<OwnsMatcher>, path: &str) -> bool {
    matcher.as_ref().is_some_and(|m| m.matches(path))
}

/// Decision 22, its rules in order; the first that matches wins and names the first
/// path, in sorted order, that triggered it.
pub fn affected(
    changed: &[String],
    tiers: &TierProfile,
    hub: &[String],
    source: &[String],
    modules: &[String],
    graph: &GraphState,
) -> Affected {
    let mut paths: Vec<&str> = changed.iter().map(String::as_str).collect();
    paths.sort_unstable();
    paths.dedup();

    // 1. A full trigger or a hub file.
    let triggers = matcher(&tiers.full_triggers);
    let hubs = matcher(hub);
    for path in &paths {
        if matches(&triggers, path) {
            return Affected::Full(format!("full trigger: {path}"));
        }
        if matches(&hubs, path) {
            return Affected::Full(format!("hub file: {path}"));
        }
    }

    // 2. Each path's module directory; a path in none is ignored only when nothing
    // says it executes.
    let sources = matcher(source);
    let tests = matcher(&tiers.test_paths);
    let mut dirs = BTreeSet::new();
    for path in &paths {
        match path_module(path, modules) {
            Some(dir) => {
                dirs.insert(dir);
            }
            None if !source.is_empty() && !matches(&sources, path) && !matches(&tests, path) => {}
            None => return Affected::Full(format!("unowned path {path}")),
        }
    }

    // 3. An unknown graph.
    let graph = match graph {
        GraphState::Known(graph) => graph,
        GraphState::Unknown(reason) => {
            return Affected::Full(format!("module graph unknown: {reason}"));
        }
    };

    // Decision 10: each module directory's name.
    let mut changed_modules = BTreeSet::new();
    for dir in &dirs {
        match module_name(dir, tiers.module_names, graph) {
            Ok(name) => {
                changed_modules.insert(name);
            }
            Err(reason) => return Affected::Full(reason),
        }
    }

    // 4. The closure over reverse dependencies.
    let closure = dependents(graph, changed_modules);

    // 5. Every module.
    if !closure.is_empty() && closure.len() == graph.modules.len() {
        return Affected::Full("every module affected".to_string());
    }

    // 6.
    Affected::Modules(closure)
}

/// A module directory's name in `graph`: its last component with `dir` names, the
/// package whose manifest directory it is with `cargo` names.
fn module_name(dir: &str, names: ModuleNames, graph: &ModuleGraph) -> Result<String, String> {
    match names {
        ModuleNames::Cargo => graph
            .modules
            .iter()
            .find(|(_, info)| info.dir.as_deref() == Some(dir))
            .map(|(name, _)| name.clone())
            .ok_or_else(|| format!("no package for module {dir}")),
        ModuleNames::Dir => {
            let name = dir.rsplit('/').next().unwrap_or(dir).to_string();
            if graph.modules.contains_key(&name) {
                Ok(name)
            } else {
                Err(format!("module {name} is not in the module graph"))
            }
        }
    }
}

/// `start` and every module that depends on one of them, transitively; cycles allowed.
fn dependents(graph: &ModuleGraph, start: BTreeSet<String>) -> BTreeSet<String> {
    let mut reverse: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
    for (name, info) in &graph.modules {
        for dep in &info.deps {
            reverse.entry(dep.as_str()).or_default().push(name.as_str());
        }
    }
    let mut out = start;
    let mut queue: Vec<String> = out.iter().cloned().collect();
    while let Some(name) = queue.pop() {
        for user in reverse.get(name.as_str()).into_iter().flatten() {
            if out.insert(user.to_string()) {
                queue.push(user.to_string());
            }
        }
    }
    out
}
