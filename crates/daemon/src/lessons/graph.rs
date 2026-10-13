//! Milestone 9.15 decisions 41 to 43 (task M9.15.13): the knowledge graph's nodes and
//! edges, built from the folded store, the consolidated lessons and the modules. Pure:
//! the caller reads the store, the history and the module source (on `spawn_blocking`).
//!
//! - **Nodes**: one hub per module (`m:<name>`, at most [`MODULE_HUBS_MAX`], by name),
//!   the root hub [`ROOT_ID`] (always), and one node per active or proposed lesson; a
//!   known failure whose `fp` was dismissed is left out.
//! - **Edges**: `touches` (lesson → the module whose directory is the longest
//!   component prefix of a file's literal prefix, else the root hub; once per hub),
//!   `shares_files` (two lessons whose files intersect, once, `from < to`) and
//!   `from_signature` (a distilled lesson → `kf-<fp>` when its distill entry's `fps`
//!   held that drawn known failure's `fp`).
//! - **Cap**: [`GRAPH_NODES_MAX`] nodes in all; the hubs first, then the lessons in
//!   decision 21's freeze order (`known`, `approved`, `kept`, `on_trial`, then
//!   `proposed`; newest `last_seen` first; then the id). The rest are counted in
//!   `omitted`.
//!
//! Cost: one sort of the lessons, then work over the drawn ones only: the pairwise
//! `shares_files` step is bounded by the cap (`PAIRS` counts it in the tests), never by
//! the size of the store.
//!
//! Labels are plain text: a title or module name passes through `safe_text::one_line`.

use std::cmp::Reverse;
use std::collections::{BTreeMap, BTreeSet};

use proto::safe_text::one_line;
use proto::{
    GRAPH_NODES_MAX, GraphEdge, GraphEdgeKind, GraphNode, GraphNodeKind, LessonGraph, LessonStatus,
    ModuleSource,
};

use super::fold::{Lesson, Store};
use crate::run::globs::{any_intersect, literal_prefix};

/// The root hub: files in no module, and lessons with no files (decision 42).
pub const ROOT_ID: &str = "m:(root)";
/// The root hub's label.
pub const ROOT_LABEL: &str = "(root)";
/// The most module hubs drawn, by name (decision 43); the root hub is extra.
pub const MODULE_HUBS_MAX: usize = 100;

#[cfg(test)]
thread_local! {
    /// Lesson pairs the `shares_files` step compared on this thread: the cost test's
    /// measure, with no wall-clock bound.
    pub(super) static PAIRS: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };
}

/// Counts one compared pair (tests only).
fn count_pair() {
    #[cfg(test)]
    PAIRS.with(|c| c.set(c.get() + 1));
}

/// A module hub's node id.
pub fn module_id(name: &str) -> String {
    format!("m:{name}")
}

/// Decisions 41 to 43: the graph of `lessons` (consolidated) over `modules` (`(name,
/// directory)`; a module whose directory is unknown gets a hub and no files), its
/// modules read from `source`. `project` is left empty for the caller to fill.
pub fn build(
    store: &Store,
    lessons: &[Lesson],
    modules: &[(String, Option<String>)],
    source: ModuleSource,
) -> LessonGraph {
    let hubs = hubs(modules);
    let mut nodes = vec![GraphNode {
        id: ROOT_ID.to_string(),
        kind: GraphNodeKind::Root,
        label: ROOT_LABEL.to_string(),
        status: None,
        layer: None,
    }];
    nodes.extend(hubs.iter().map(|(name, _)| GraphNode {
        id: module_id(name),
        kind: GraphNodeKind::Module,
        label: one_line(name),
        status: None,
        layer: None,
    }));

    let mut shown: Vec<&Lesson> = (lessons.iter())
        .filter(|l| rank(l.status).is_some() && !dismissed(store, l))
        .collect();
    shown.sort_by(|a, b| {
        (rank(a.status), Reverse(a.last_seen), &a.id).cmp(&(
            rank(b.status),
            Reverse(b.last_seen),
            &b.id,
        ))
    });
    let room = GRAPH_NODES_MAX.saturating_sub(nodes.len());
    let omitted = shown.len().saturating_sub(room);
    shown.truncate(room);

    nodes.extend(shown.iter().map(|l| GraphNode {
        id: l.id.clone(),
        kind: GraphNodeKind::Lesson,
        label: one_line(&l.title),
        status: Some(l.status),
        layer: Some(l.layer),
    }));

    let dirs: Vec<(String, Vec<&str>)> = (hubs.iter())
        .filter_map(|(name, dir)| dir.as_deref().map(|d| (module_id(name), components(d))))
        .collect();
    let mut edges = Vec::new();
    for lesson in &shown {
        for hub in touched(&lesson.files, &dirs) {
            edges.push(edge(&lesson.id, &hub, GraphEdgeKind::Touches));
        }
    }
    for (i, a) in shown.iter().enumerate() {
        for b in &shown[i + 1..] {
            count_pair();
            if any_intersect(&a.files, &b.files) {
                let (from, to) = if a.id < b.id { (a, b) } else { (b, a) };
                edges.push(edge(&from.id, &to.id, GraphEdgeKind::SharesFiles));
            }
        }
    }
    let known: BTreeSet<&str> = (shown.iter())
        .filter(|l| l.status == LessonStatus::Known)
        .map(|l| l.id.as_str())
        .collect();
    for lesson in &shown {
        let Some(entry) = (lesson.queued.as_ref()).and_then(|q| store.queued.get(q)) else {
            continue;
        };
        let mut linked: Vec<String> = Vec::new();
        for fp in &entry.fps {
            let target = format!("kf-{fp}");
            if known.contains(target.as_str()) && !linked.contains(&target) {
                edges.push(edge(&lesson.id, &target, GraphEdgeKind::FromSignature));
                linked.push(target);
            }
        }
    }

    LessonGraph {
        project: Default::default(),
        nodes,
        edges,
        lessons: shown.iter().map(|l| l.info()).collect(),
        omitted: u32::try_from(omitted).unwrap_or(u32::MAX),
        module_source: source,
    }
}

/// The drawn module hubs: each name once (its first directory kept), sorted by name,
/// at most [`MODULE_HUBS_MAX`].
fn hubs(modules: &[(String, Option<String>)]) -> Vec<(String, Option<String>)> {
    let mut by_name: BTreeMap<&str, &Option<String>> = BTreeMap::new();
    for (name, dir) in modules {
        by_name.entry(name.as_str()).or_insert(dir);
    }
    (by_name.into_iter())
        .take(MODULE_HUBS_MAX)
        .map(|(name, dir)| (name.to_string(), dir.clone()))
        .collect()
}

/// A module directory's components; `.` and empty ones dropped, so the repository's
/// root directory is the empty prefix (every file is under it).
fn components(dir: &str) -> Vec<&str> {
    (dir.split('/'))
        .filter(|c| !c.is_empty() && *c != ".")
        .collect()
}

/// The hubs `files` touch, in file order, each once: per file the module whose
/// directory is the longest component prefix of the file's literal prefix (the first
/// such module on a tie), else the root hub; the root hub alone for no files.
fn touched(files: &[String], dirs: &[(String, Vec<&str>)]) -> Vec<String> {
    let mut hubs: Vec<String> = Vec::new();
    let mut add = |hub: &str| {
        if !hubs.iter().any(|h| h == hub) {
            hubs.push(hub.to_string());
        }
    };
    if files.is_empty() {
        add(ROOT_ID);
    }
    for file in files {
        let prefix: Vec<&str> = (literal_prefix(file).into_iter())
            .filter(|c| !c.is_empty() && *c != ".")
            .collect();
        let mut best: Option<&(String, Vec<&str>)> = None;
        for module in dirs {
            let longer = best.is_none_or(|b| module.1.len() > b.1.len());
            if prefix.starts_with(&module.1) && longer {
                best = Some(module);
            }
        }
        add(best.map_or(ROOT_ID, |(hub, _)| hub.as_str()));
    }
    hubs
}

/// Decision 43's order: decision 21's freeze ranks, `proposed` after them; `None` for a
/// lesson that is not drawn (`rejected`, `retired`).
fn rank(status: LessonStatus) -> Option<u8> {
    match status {
        LessonStatus::Known => Some(0),
        LessonStatus::Approved => Some(1),
        LessonStatus::Kept => Some(2),
        LessonStatus::OnTrial => Some(3),
        LessonStatus::Proposed => Some(4),
        LessonStatus::Rejected | LessonStatus::Retired => None,
    }
}

/// A known failure whose `fp` the store dismissed (decision 48).
fn dismissed(store: &Store, lesson: &Lesson) -> bool {
    (lesson.id.strip_prefix("kf-")).is_some_and(|fp| store.dismissed.contains(fp))
}

fn edge(from: &str, to: &str, kind: GraphEdgeKind) -> GraphEdge {
    GraphEdge {
        from: from.to_string(),
        to: to.to_string(),
        kind,
    }
}

#[cfg(test)]
#[path = "graph_tests.rs"]
mod tests;
