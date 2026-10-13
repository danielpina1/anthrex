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
//! - **Edge cap** (M9.15.13's edge-cap ruling, before layout): each lesson keeps at
//!   most [`SHARES_PER_LESSON`] `shares_files` edges and the graph at most
//!   [`SHARES_MAX`], the strongest first (see [`strongest`]).
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
use crate::run::globs::{intersects, literal_prefix};

/// The root hub: files in no module, and lessons with no files (decision 42).
pub const ROOT_ID: &str = "m:(root)";
/// The root hub's label.
pub const ROOT_LABEL: &str = "(root)";
/// The most module hubs drawn, by name (decision 43); the root hub is extra.
pub const MODULE_HUBS_MAX: usize = 100;
/// The most `shares_files` edges one lesson keeps (the edge-cap ruling).
pub const SHARES_PER_LESSON: usize = 8;
/// The most `shares_files` edges in one graph (the edge-cap ruling). At
/// [`GRAPH_NODES_MAX`] nodes the per-lesson cap already keeps the total at or below
/// 299 × 8 / 2 = 1,196, so this is a backstop should either cap grow.
pub const SHARES_MAX: usize = 2_000;

/// One candidate `shares_files` edge: `from < to`, and `strength` intersecting
/// `(file, file)` pairs between the two lessons.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Share<'a> {
    pub(super) from: &'a str,
    pub(super) to: &'a str,
    pub(super) strength: u32,
}

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
    let mut shares: Vec<Share> = Vec::new();
    for (i, a) in shown.iter().enumerate() {
        for b in &shown[i + 1..] {
            count_pair();
            let strength = shared(&a.files, &b.files);
            if strength > 0 {
                let (from, to) = if a.id < b.id { (a, b) } else { (b, a) };
                shares.push(Share {
                    from: &from.id,
                    to: &to.id,
                    strength,
                });
            }
        }
    }
    let keep = strongest(&shares, SHARES_PER_LESSON, SHARES_MAX);
    for (share, _) in shares.iter().zip(keep).filter(|(_, k)| *k) {
        edges.push(edge(share.from, share.to, GraphEdgeKind::SharesFiles));
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

/// The number of `(file, file)` pairs of `a` and `b` that intersect: a pair of
/// lessons' `shares_files` strength (0 when they share nothing). Symmetric.
fn shared(a: &[String], b: &[String]) -> u32 {
    let count: usize = (a.iter())
        .map(|ga| b.iter().filter(|gb| intersects(ga, gb)).count())
        .sum();
    u32::try_from(count).unwrap_or(u32::MAX)
}

/// Which of `shares` survive the edge cap, as flags in `shares`' order (so the kept
/// edges keep their pairwise order, and a graph under both caps is unchanged). The
/// candidates are visited strongest first, ties by `(from, to)` id; one is kept while
/// both its lessons hold fewer than `per_lesson` kept edges and fewer than `total` are
/// kept. Deterministic: it depends on ids and strengths only, never on input order.
pub(super) fn strongest(shares: &[Share], per_lesson: usize, total: usize) -> Vec<bool> {
    let mut order: Vec<usize> = (0..shares.len()).collect();
    order.sort_by(|&i, &j| {
        let (a, b) = (&shares[i], &shares[j]);
        (Reverse(a.strength), a.from, a.to).cmp(&(Reverse(b.strength), b.from, b.to))
    });
    let mut keep = vec![false; shares.len()];
    let mut degree: BTreeMap<&str, usize> = BTreeMap::new();
    let mut kept = 0;
    for i in order {
        if kept == total {
            break;
        }
        let share = &shares[i];
        let full = |id: &str| degree.get(id).is_some_and(|d| *d >= per_lesson);
        if full(share.from) || full(share.to) {
            continue;
        }
        *degree.entry(share.from).or_insert(0) += 1;
        *degree.entry(share.to).or_insert(0) += 1;
        keep[i] = true;
        kept += 1;
    }
    keep
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
