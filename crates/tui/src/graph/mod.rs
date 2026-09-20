//! The pure left-to-right layout behind the graph overview (spec §4.2).
//!
//! Input is the visible row list the sidebar already builds — after collapse
//! and after the filter — so the graph inherits folding, filtering and the
//! `j`/`k` order with no code of its own (decision 8). Output is a rectangle
//! per node in canvas coordinates, plus the parent-to-children connections the
//! painter draws between them.
//!
//! Nothing here draws, clips or scrolls: painting is a separate pass over this
//! result (decision 9), which is why this one is asserted by exact rectangles.

use crate::tree::{NodeKey, Row, RowKind};
use ratatui::layout::Rect;
use unicode_width::UnicodeWidthStr;

/// A tier is never narrower than this, borders included, so a project called
/// `ab` still reads as a box.
pub const MIN_NODE_WIDTH: u16 = 12;
/// A tier is never wider than this; a single very long label is truncated by
/// the painter rather than allowed to stretch its whole tier.
pub const MAX_NODE_WIDTH: u16 = 30;
/// Top border, content, bottom border.
pub const NODE_HEIGHT: u16 = 3;
/// Columns between a tier's right edge and the next tier's left edge. The
/// edges are drawn in them.
pub const TIER_GAP: u16 = 3;
/// Blank rows between two stacked sibling boxes.
pub const ROW_GAP: u16 = 1;

/// Two borders and one space of padding each side, which a node's width adds
/// to its content (decision 3).
const BORDERS_AND_PADDING: u16 = 4;
/// The status glyph and the space after it, which precede the content text in
/// every box (decision 11). Every status glyph is one column wide.
const GLYPH_COLUMNS: usize = 2;

/// One box on the canvas.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlacedNode {
    pub key: NodeKey,
    pub rect: Rect,
    pub depth: u16,
}

/// One parent-to-children connection, in canvas coordinates.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Edge {
    pub parent: NodeKey,
    pub children: Vec<NodeKey>,
}

/// Every node placed on the canvas, with the canvas's own size.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Layout {
    pub nodes: Vec<PlacedNode>,
    pub edges: Vec<Edge>,
    /// The bounding box of every placed node, as `(width, height)`.
    pub size: (u16, u16),
}

impl Layout {
    /// The placement of one node, or `None` when it is not on the canvas —
    /// filtered out, or below something collapsed.
    pub fn node(&self, key: &NodeKey) -> Option<&PlacedNode> {
        self.nodes.iter().find(|node| &node.key == key)
    }
}

/// Places every visible row on the canvas.
///
/// `rows` is in visible pre-order, so a row's parent is the nearest row above
/// it whose depth is one less; a row whose parent tier is missing is treated
/// as a root rather than dropped.
pub fn layout(rows: &[Row<'_>]) -> Layout {
    if rows.is_empty() {
        return Layout::default();
    }

    let tiers = tier_count(rows);
    let (children, roots) = parentage(rows, tiers);
    let widths = tier_widths(rows, tiers);
    let xs = tier_columns(&widths);

    let mut tops = vec![0; rows.len()];
    let mut next_row = 0;
    for root in roots {
        place(root, &children, &mut tops, &mut next_row);
    }

    let mut nodes = Vec::with_capacity(rows.len());
    let mut edges = Vec::new();
    let mut size = (0, 0);
    for (index, row) in rows.iter().enumerate() {
        let tier = usize::from(row.depth);
        let rect = Rect {
            x: xs[tier],
            y: tops[index],
            width: widths[tier],
            height: NODE_HEIGHT,
        };
        size.0 = size.0.max(rect.x.saturating_add(rect.width));
        size.1 = size.1.max(rect.y.saturating_add(rect.height));
        nodes.push(PlacedNode {
            key: row.key.clone(),
            rect,
            depth: row.depth,
        });
        if !children[index].is_empty() {
            edges.push(Edge {
                parent: row.key.clone(),
                children: children[index]
                    .iter()
                    .map(|child| rows[*child].key.clone())
                    .collect(),
            });
        }
    }
    Layout { nodes, edges, size }
}

/// The visible children of every row, and the indices of the roots.
///
/// Depth alone carries the shape: in pre-order the parent of a row is whatever
/// row was last seen one level up, and anything deeper than that row is no
/// longer in scope once we come back out.
fn parentage(rows: &[Row<'_>], tiers: usize) -> (Vec<Vec<usize>>, Vec<usize>) {
    let mut children = vec![Vec::new(); rows.len()];
    let mut roots = Vec::new();
    let mut last_in_tier: Vec<Option<usize>> = vec![None; tiers];
    for (index, row) in rows.iter().enumerate() {
        let tier = usize::from(row.depth);
        match tier.checked_sub(1).and_then(|above| last_in_tier[above]) {
            Some(parent) => children[parent].push(index),
            None => roots.push(index),
        }
        last_in_tier[tier] = Some(index);
        for stale in &mut last_in_tier[tier + 1..] {
            *stale = None;
        }
    }
    (children, roots)
}

/// One width per tier: the widest content in it plus borders and padding,
/// held between the floor and the cap (decision 3).
fn tier_widths(rows: &[Row<'_>], tiers: usize) -> Vec<u16> {
    let mut widths = vec![MIN_NODE_WIDTH; tiers];
    for row in rows {
        let content = u16::try_from(content_width(row)).unwrap_or(MAX_NODE_WIDTH);
        let wanted = content
            .saturating_add(BORDERS_AND_PADDING)
            .clamp(MIN_NODE_WIDTH, MAX_NODE_WIDTH);
        let width = &mut widths[usize::from(row.depth)];
        *width = (*width).max(wanted);
    }
    widths
}

/// The left edge of each tier: the sum of the widths to its left, plus one
/// `TIER_GAP` for each (decision 5). Not `depth × constant` — tiers differ.
fn tier_columns(widths: &[u16]) -> Vec<u16> {
    let mut xs = Vec::with_capacity(widths.len());
    let mut x: u16 = 0;
    for width in widths {
        xs.push(x);
        x = x.saturating_add(*width).saturating_add(TIER_GAP);
    }
    xs
}

/// One more than the deepest row's depth: how many tiers the canvas has.
fn tier_count(rows: &[Row<'_>]) -> usize {
    rows.iter()
        .map(|row| usize::from(row.depth).saturating_add(1))
        .max()
        .unwrap_or_default()
}

/// Assigns the top row of one subtree, post-order (decision 6).
///
/// A leaf takes the next free row; a parent centres on the span from its first
/// child's top to its last child's bottom, rounded down. A parent therefore
/// always lands inside its children's span and never collides with the rows a
/// later sibling will take.
fn place(index: usize, children: &[Vec<usize>], tops: &mut [u16], next_row: &mut u16) {
    let Some((first_child, last_child)) = children[index]
        .first()
        .copied()
        .zip(children[index].last().copied())
    else {
        tops[index] = *next_row;
        *next_row = next_row.saturating_add(NODE_HEIGHT).saturating_add(ROW_GAP);
        return;
    };

    for child in &children[index] {
        place(*child, children, tops, next_row);
    }
    let top = tops[first_child];
    let bottom = tops[last_child]
        .saturating_add(NODE_HEIGHT)
        .saturating_sub(1);
    let span = bottom.saturating_sub(top).saturating_add(1);
    tops[index] = top.saturating_add(span.saturating_sub(NODE_HEIGHT) / 2);
}

/// The text drawn inside a node's box after its status glyph: the window's
/// tree position for window nodes, then the name or label (decision 11).
///
/// The painter draws exactly this, so the width a tier is sized to and the
/// text that has to fit in it cannot drift apart.
pub(crate) fn content_text(row: &Row<'_>) -> String {
    match &row.kind {
        RowKind::Project { name, .. } => name.clone(),
        RowKind::Window { info, position, .. } => format!("{position} {}", info.name),
        RowKind::Subagent { info } => match info.label.as_deref() {
            Some(label) => format!("{}: {label}", info.kind),
            None => info.kind.clone(),
        },
    }
}

/// The display width of everything inside a node's box, glyph included.
///
/// Display width, not byte length and not a character count: one CJK label
/// measured wrongly pushes every border in its tier out of line (decision 10).
fn content_width(row: &Row<'_>) -> usize {
    GLYPH_COLUMNS.saturating_add(UnicodeWidthStr::width(content_text(row).as_str()))
}

#[cfg(test)]
mod tests;
