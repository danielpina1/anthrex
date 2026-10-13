//! Milestone 9.14 decision 26: the inputs id, the tree part of a per-module step's
//! cache key. Pure: the caller reads the tree's entries (`git::tree_entries`) and the
//! module graph.
//!
//! A per-module step keys on every path of the tree outside the modules, plus the
//! paths of the modules its affected set's closure holds (the set and, transitively,
//! every module it depends on, dev-dependencies included). A change in a module
//! outside that closure leaves the key alone, so the step's result is reused. Ruling
//! C30: a `module_test` step keys on the closure of the whole affected set, not of its
//! own module; safe, with fewer hits.

use std::collections::BTreeSet;

use proto::ModuleNames;
use sha2::{Digest, Sha256};

use super::ModuleGraph;
use super::affected::module_name;
use crate::run::git::TreeEntry;
use crate::run::globs::path_module;

/// The prefix of an inputs id, so it is never taken for a tree id.
pub const INPUTS_PREFIX: &str = "m1:";

/// `modules` and every module they depend on in `graph`, transitively; cycles
/// allowed. A name the graph does not hold stays in, with no dependencies.
pub fn closure(graph: &ModuleGraph, modules: &BTreeSet<String>) -> BTreeSet<String> {
    let mut out = modules.clone();
    let mut queue: Vec<String> = out.iter().cloned().collect();
    while let Some(name) = queue.pop() {
        for dep in graph.modules.get(&name).into_iter().flat_map(|m| &m.deps) {
            if out.insert(dep.clone()) {
                queue.push(dep.clone());
            }
        }
    }
    out
}

/// `m1:` and the lowercase hex SHA-256 over each kept entry, in the order given, as
/// `<mode> <object>\t<path>\0`.
pub fn inputs_id(entries: &[TreeEntry], keep: &dyn Fn(&str) -> bool) -> String {
    let mut hash = Sha256::new();
    for entry in entries.iter().filter(|e| keep(&e.path)) {
        hash.update(entry.mode.as_bytes());
        hash.update(b" ");
        hash.update(entry.object.as_bytes());
        hash.update(b"\t");
        hash.update(entry.path.as_bytes());
        hash.update(b"\0");
    }
    let hex: String = hash.finalize().iter().map(|b| format!("{b:02x}")).collect();
    format!("{INPUTS_PREFIX}{hex}")
}

/// Whether a path is an input of a step whose closure is `closure`: a path in no
/// module, in a module whose name cannot be found, or in a module of the closure.
pub fn keeper<'a>(
    closure: &'a BTreeSet<String>,
    modules: &'a [String],
    names: ModuleNames,
    graph: &'a ModuleGraph,
) -> impl Fn(&str) -> bool + 'a {
    move |path| match path_module(path, modules) {
        None => true,
        Some(dir) => match module_name(&dir, names, graph) {
            Ok(name) => closure.contains(&name),
            Err(_) => true,
        },
    }
}

/// A per-module step's key tree for the affected set `set`: `tree` when the closure
/// holds every module of the graph (an inputs id over everything is the tree), else
/// the inputs id of `entries` (`tree`'s own).
pub fn key_tree(
    tree: &str,
    entries: &[TreeEntry],
    set: &BTreeSet<String>,
    modules: &[String],
    names: ModuleNames,
    graph: &ModuleGraph,
) -> String {
    let closure = closure(graph, set);
    if graph.modules.keys().all(|m| closure.contains(m)) {
        return tree.to_string();
    }
    inputs_id(entries, &keeper(&closure, modules, names, graph))
}
