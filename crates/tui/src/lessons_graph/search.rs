//! Decision 47 (task M9.15.15): the graph page's search. Pure.
//!
//! `/` opens a one-line query on the hint row; every keystroke filters,
//! case-insensitively, over a lesson's title, body and files and a module's name
//! (its label); matches are lit and the rest dimmed. `Enter` ends typing and keeps the
//! filter; `n`/`N` select the next/previous match in id order (wrapping); `Esc` clears
//! the query, and a second `Esc` (nothing left to clear) leaves the page. An empty
//! query matches nothing and dims nothing.

use crossterm::event::KeyCode;
use proto::{GraphNode, GraphNodeKind, LessonGraph};

/// The query and whether it is being typed.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Search {
    pub query: String,
    pub typing: bool,
}

impl Search {
    /// `/`: a fresh query, typed.
    pub fn open(&mut self) {
        self.query.clear();
        self.typing = true;
    }

    /// A key while typing: a character extends the query, Backspace shortens it,
    /// `Enter` ends typing (the filter stays), `Esc` clears. `true` when it was taken.
    pub fn key(&mut self, code: KeyCode) -> bool {
        if !self.typing {
            return false;
        }
        match code {
            KeyCode::Char(c) => self.query.push(c),
            KeyCode::Backspace => {
                self.query.pop();
            }
            KeyCode::Enter => self.typing = false,
            KeyCode::Esc => {
                self.esc();
            }
            _ => {}
        }
        true
    }

    /// `Esc`: clears the query (and ends typing); `false` when there was nothing to
    /// clear, so the page's next `Esc` handler runs.
    pub fn esc(&mut self) -> bool {
        let had = self.typing || !self.query.is_empty();
        self.query.clear();
        self.typing = false;
        had
    }

    /// Whether a filter is in force (a non-empty query).
    pub fn active(&self) -> bool {
        !self.query.is_empty()
    }
}

/// Whether `node` matches `needle` (already lowercased, non-empty).
fn node_matches(graph: &LessonGraph, node: &GraphNode, needle: &str) -> bool {
    let has = |text: &str| text.to_lowercase().contains(needle);
    match node.kind {
        GraphNodeKind::Module | GraphNodeKind::Root => has(&node.label),
        GraphNodeKind::Lesson => match graph.lessons.iter().find(|l| l.id == node.id) {
            Some(l) => has(&l.title) || has(&l.body) || l.files.iter().any(|f| has(f)),
            None => has(&node.label),
        },
    }
}

/// One flag per node of `graph`, in its order: whether it matches `query`. An empty
/// query matches nothing.
pub fn matches(graph: &LessonGraph, query: &str) -> Vec<bool> {
    if query.is_empty() {
        return vec![false; graph.nodes.len()];
    }
    let needle = query.to_lowercase();
    graph
        .nodes
        .iter()
        .map(|n| node_matches(graph, n, &needle))
        .collect()
}

/// `n` (`forward`) or `N`: the next or previous match after `current` in id order,
/// wrapping; the first (or last) when `current` is no match or none. `None` with no
/// match.
pub fn cycle(
    graph: &LessonGraph,
    lit: &[bool],
    current: Option<&str>,
    forward: bool,
) -> Option<String> {
    let mut ids: Vec<&str> = graph
        .nodes
        .iter()
        .zip(lit)
        .filter(|(_, m)| **m)
        .map(|(n, _)| n.id.as_str())
        .collect();
    ids.sort_unstable();
    ids.dedup();
    if ids.is_empty() {
        return None;
    }
    let next = match current {
        Some(cur) if forward => ids.iter().find(|id| **id > cur).unwrap_or(&ids[0]),
        Some(cur) => ids
            .iter()
            .rev()
            .find(|id| **id < cur)
            .unwrap_or(&ids[ids.len() - 1]),
        None if forward => &ids[0],
        None => &ids[ids.len() - 1],
    };
    Some(next.to_string())
}

#[cfg(test)]
#[path = "search_tests.rs"]
mod tests;
