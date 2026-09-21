//! The painter: fills a character grid the size of the viewport with every
//! placed node's box and every parent-to-children edge, offset by the pan,
//! then converts the grid to ratatui `Line`s (decision 9). That keeps painting
//! testable by exact strings, with no terminal involved.
//!
//! Boxes first, edges second: an edge's last act is to turn the border cell it
//! meets into a junction, which only works if the border is already there
//! (decision 13).

use super::{Edge, Layout, Pan};
use crate::app::App;
use crate::theme;
use crate::tree::{NodeKey, Row, RowKind};
use crate::ui::tree_view::truncate;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use std::collections::HashMap;
use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthStr;

/// Fills a character grid the size of `area` with every node `layout` places
/// and every edge it records, offset by `pan`, and converts it to lines.
///
/// A box or an edge that only partly overlaps the viewport is clipped one cell
/// at a time rather than dropped whole: each cell is placed independently, so
/// whatever falls inside `area` survives even when the shape's own origin does
/// not.
///
/// `rows` is the very list `layout` was built from, and the caller passes it in
/// rather than rebuilding it: painting runs at least ten times a second in a
/// process meant to run for days. `layout` pushes one `PlacedNode` per row in
/// row order, so the two zip and no node has to be looked up by key.
pub fn paint(
    layout: &Layout,
    area: Rect,
    pan: Pan,
    rows: &[Row<'_>],
    app: &App,
) -> Vec<Line<'static>> {
    let mut grid = Grid::new(area, pan);
    for (node, row) in layout.nodes.iter().zip(rows) {
        debug_assert_eq!(
            node.key, row.key,
            "layout places one node per row, in order"
        );
        paint_node(&mut grid, node.rect, row, app);
    }
    // One pass over the nodes instead of one scan per edge endpoint: an edge
    // names its parent and children by key, and there are as many edges as
    // there are nodes with children.
    let placed: HashMap<&NodeKey, Rect> = layout
        .nodes
        .iter()
        .map(|node| (&node.key, node.rect))
        .collect();
    for edge in &layout.edges {
        paint_edge(&mut grid, &placed, edge);
    }
    grid.into_lines()
}

/// One canvas cell: the text it shows and the style it shows it in. A wide
/// character's second column holds an empty string, the same trick a
/// terminal buffer uses to give one grapheme two columns.
#[derive(Clone)]
struct Cell {
    text: String,
    style: Style,
}

impl Default for Cell {
    fn default() -> Self {
        Cell {
            text: " ".to_owned(),
            style: Style::default(),
        }
    }
}

/// The viewport's own character grid, addressed in canvas coordinates: every
/// write is offset by `pan` and dropped when it lands outside `width` x
/// `height`.
struct Grid {
    pan: Pan,
    width: u16,
    height: u16,
    cells: Vec<Cell>,
}

impl Grid {
    fn new(area: Rect, pan: Pan) -> Self {
        let cells = vec![Cell::default(); usize::from(area.width) * usize::from(area.height)];
        Self {
            pan,
            width: area.width,
            height: area.height,
            cells,
        }
    }

    /// Where a canvas coordinate lands in `cells`, or `None` when it lands
    /// outside the viewport: above or left of `pan`, or past the grid's own
    /// width or height. That is the clipping this module promises, and every
    /// write goes through it.
    fn index(&self, canvas_x: u16, canvas_y: u16) -> Option<usize> {
        let x = canvas_x.checked_sub(self.pan.x)?;
        let y = canvas_y.checked_sub(self.pan.y)?;
        if x >= self.width || y >= self.height {
            return None;
        }
        Some(usize::from(y) * usize::from(self.width) + usize::from(x))
    }

    /// Writes one cell at a canvas coordinate, dropping it when it falls
    /// outside the viewport.
    fn place(&mut self, canvas_x: u16, canvas_y: u16, text: String, style: Style) {
        if let Some(index) = self.index(canvas_x, canvas_y) {
            self.cells[index] = Cell { text, style };
        }
    }

    /// Writes one cell of an edge, in the plain border style the gap columns
    /// belong to.
    fn place_edge_cell(&mut self, canvas_x: u16, canvas_y: u16, text: &str) {
        self.place(canvas_x, canvas_y, text.to_owned(), theme::border());
    }

    /// Replaces a cell's text but keeps the style already under it, so a
    /// junction painted onto a box's border inherits that box's focused or
    /// selected styling instead of reverting it to the plain border style.
    fn overwrite_text(&mut self, canvas_x: u16, canvas_y: u16, text: &str) {
        if let Some(index) = self.index(canvas_x, canvas_y) {
            self.cells[index].text = text.to_owned();
        }
    }

    /// Writes one row of a box, `selected` reversing every cell's colours the
    /// way a selected sidebar row is today (decision 12).
    fn place_row(&mut self, x0: u16, y: u16, slots: Vec<(String, Style)>, selected: bool) {
        for (offset, (text, style)) in slots.into_iter().enumerate() {
            let Ok(dx) = u16::try_from(offset) else {
                continue;
            };
            let style = if selected {
                style.add_modifier(Modifier::REVERSED)
            } else {
                style
            };
            self.place(x0.saturating_add(dx), y, text, style);
        }
    }

    /// Converts every row to a `Line`, merging adjacent cells that share a
    /// style into one span rather than emitting one span per column.
    fn into_lines(self) -> Vec<Line<'static>> {
        let width = usize::from(self.width);
        (0..usize::from(self.height))
            .map(|row| Line::from(merge_runs(&self.cells[row * width..(row + 1) * width])))
            .collect()
    }
}

/// Coalesces a row of cells into spans, one per run of cells that share a
/// style — the border and the padding are usually one run each, and the
/// status glyph is its own because its colour differs from its neighbours.
fn merge_runs(cells: &[Cell]) -> Vec<Span<'static>> {
    let mut spans: Vec<Span<'static>> = Vec::new();
    let last_index = cells.len().saturating_sub(1);
    for (index, cell) in cells.iter().enumerate() {
        let text = edge_text(cell, index == 0, index == last_index);
        match spans.last_mut() {
            Some(last) if last.style == cell.style => {
                let mut content = last.content.to_string();
                content.push_str(text);
                last.content = content.into();
            }
            _ => spans.push(Span::styled(text.to_owned(), cell.style)),
        }
    }
    spans
}

/// The text one cell contributes to its line, with the two halves of a wide
/// grapheme that the viewport's own edges can separate put right.
///
/// A wide grapheme occupies two slots — the grapheme, then an empty
/// continuation — and `Grid::index` clips one slot at a time. When the pan
/// lands on the second slot the grapheme is clipped away and the empty string
/// survives; when the viewport's right edge falls between the two, the
/// grapheme survives without its continuation. Either way the pair has lost a
/// cell, so the survivor becomes a space and the line keeps the area's display
/// width. Left alone, the first case leaves the line one column short and
/// ratatui left-aligns it, shifting every border to its right out of line with
/// the rows above and below; the second leaves it one column too long.
fn edge_text(cell: &Cell, first: bool, last: bool) -> &str {
    let orphaned_continuation = first && cell.text.is_empty();
    let orphaned_grapheme = last && UnicodeWidthStr::width(cell.text.as_str()) == 2;
    if orphaned_continuation || orphaned_grapheme {
        " "
    } else {
        &cell.text
    }
}

/// Paints one node's three rows into `grid`, at its placed rectangle.
fn paint_node(grid: &mut Grid, rect: Rect, row: &Row<'_>, app: &App) {
    let selected = is_selected(row, app);
    let [top, content, bottom] = node_rows(row, app, rect.width);
    grid.place_row(rect.x, rect.y, top, selected);
    grid.place_row(rect.x, rect.y.saturating_add(1), content, selected);
    grid.place_row(rect.x, rect.y.saturating_add(2), bottom, selected);
}

/// The three rows of one box: rounded borders top and bottom, and one
/// content row in between — the status glyph in its status colour, then the
/// tree position for window nodes, then the name or label, truncated with
/// `…` (decision 11).
fn node_rows(row: &Row<'_>, app: &App, width: u16) -> [Vec<(String, Style)>; 3] {
    let width = usize::from(width);
    let border = border_style(row, app);
    let (glyph, glyph_color) = glyph_and_color(&row.kind, app);
    let text = super::content_text(row);

    let mut top = vec![(String::from("─"), border); width];
    top[0] = (String::from("╭"), border);
    top[width - 1] = (String::from("╮"), border);

    let mut bottom = vec![(String::from("─"), border); width];
    bottom[0] = (String::from("╰"), border);
    bottom[width - 1] = (String::from("╯"), border);

    let mut content = interior_slots(width - 2, glyph, glyph_color, &text);
    content.insert(0, (String::from("│"), border));
    content.push((String::from("│"), border));

    [top, content, bottom]
}

/// The content row between a box's two borders: one space, the glyph, one
/// space, then the text truncated to what is left, then spaces to the end.
///
/// A wide character occupies two consecutive slots so every slot is exactly
/// one canvas column — the unit `Grid::place` works in — which is what keeps
/// a CJK label from pushing the right border out of line (decision 10).
fn interior_slots(
    inner: usize,
    glyph: &'static str,
    glyph_color: Color,
    text: &str,
) -> Vec<(String, Style)> {
    let plain = Style::default();
    let mut slots = vec![
        (" ".to_owned(), plain),
        (glyph.to_owned(), plain.fg(glyph_color)),
        (" ".to_owned(), plain),
    ];
    // The pad, glyph and space above, plus one more space of padding held in
    // reserve on the right (decision 3), are never available to the text.
    let text_max = inner.saturating_sub(slots.len() + 1);
    for grapheme in truncate(text, text_max).graphemes(true) {
        slots.push((grapheme.to_owned(), plain));
        if UnicodeWidthStr::width(grapheme) == 2 {
            slots.push((String::new(), plain));
        }
    }
    while slots.len() < inner {
        slots.push((" ".to_owned(), plain));
    }
    slots
}

fn glyph_and_color(kind: &RowKind<'_>, app: &App) -> (&'static str, Color) {
    match kind {
        RowKind::Project { status, .. } => (
            theme::status_glyph(*status, app.spinner_frame),
            theme::status_color(*status),
        ),
        RowKind::Window { info, .. } => (
            theme::status_glyph(info.status, app.spinner_frame),
            theme::status_color(info.status),
        ),
        RowKind::Subagent { info } => (
            theme::subagent_glyph(info, app.spinner_frame),
            theme::subagent_color(info),
        ),
    }
}

/// The focused window's box uses the focused border style; every other box
/// uses the plain one (decision 12).
fn border_style(row: &Row<'_>, app: &App) -> Style {
    match &row.kind {
        RowKind::Window { info, .. } if app.focused == Some(info.id) => theme::border_focused(),
        _ => theme::border(),
    }
}

/// The selected node is highlighted the way the selected row is today: only
/// while tree navigation is active, matching `ui::overview`'s own rule
/// (decision 12).
fn is_selected(row: &Row<'_>, app: &App) -> bool {
    app.tree_input.is_some() && app.tree.selected.as_ref() == Some(&row.key)
}

/// Where one parent-to-children edge runs, in canvas coordinates.
///
/// Everything the edge needs is here, so the painting below is a walk over
/// rows and columns with no rectangle arithmetic left in it.
struct EdgeGeometry {
    /// The parent's right border column, and the row it leaves on: the middle
    /// of that border.
    parent_x: u16,
    parent_row: u16,
    /// The children's shared left border column — one tier, one width, one `x`
    /// (decision 3).
    child_x: u16,
    /// The middle of the `TIER_GAP` columns, where the vertical bus lives.
    bus_x: u16,
    /// The row each child connects on, in the order the layout placed them.
    child_rows: Vec<u16>,
}

impl EdgeGeometry {
    /// `None` when the edge cannot be drawn: no children still on the canvas,
    /// or no gap column between the two tiers to draw in.
    fn new(parent: Rect, children: &[Rect]) -> Option<Self> {
        let parent_x = parent.x.saturating_add(parent.width).checked_sub(1)?;
        let child_x = children.iter().map(|child| child.x).min()?;
        if child_x <= parent_x.saturating_add(1) {
            return None;
        }
        Some(EdgeGeometry {
            parent_x,
            parent_row: parent.y.saturating_add(parent.height / 2),
            child_x,
            bus_x: parent_x.saturating_add((child_x - parent_x) / 2),
            child_rows: children
                .iter()
                .map(|child| child.y.saturating_add(child.height / 2))
                .collect(),
        })
    }
}

/// Which part of the bus one of its cells is, before the parent's own run is
/// taken into account.
#[derive(Clone, Copy)]
enum BusCell {
    /// The first child's row, where the bus starts.
    Top,
    /// A child's row between the two ends.
    Child,
    /// The last child's row, where the bus stops.
    Bottom,
    /// A row the bus merely passes through.
    Plain,
}

/// Draws one parent-to-children connection in the `TIER_GAP` columns
/// (decision 13).
fn paint_edge(grid: &mut Grid, placed: &HashMap<&NodeKey, Rect>, edge: &Edge) {
    let Some(parent) = placed.get(&edge.parent).copied() else {
        return;
    };
    let children: Vec<Rect> = edge
        .children
        .iter()
        .filter_map(|child| placed.get(child).copied())
        .collect();
    let Some(geometry) = EdgeGeometry::new(parent, &children) else {
        return;
    };

    match geometry.child_rows.as_slice() {
        // One visible child: the layout centres the parent on it, so the two
        // middle rows are the same one and the edge is a single straight run
        // with no bus column at all.
        [row] => paint_straight_run(grid, &geometry, *row),
        // The bus's ends are the first and last child's rows, in the order the
        // layout placed them — which is top to bottom (decision 6).
        [first, .., last] => paint_bus(grid, &geometry, *first, *last),
        [] => {}
    }
}

/// One horizontal line from the parent's border to the child's, junctions
/// included.
fn paint_straight_run(grid: &mut Grid, geometry: &EdgeGeometry, row: u16) {
    for x in geometry.parent_x.saturating_add(1)..geometry.child_x {
        grid.place_edge_cell(x, row, "─");
    }
    paint_junctions(grid, geometry, row);
}

/// A vertical bus in the middle gap column from `top` to `bottom`, the
/// parent's run into it, and one run out of it into each child.
///
/// The bus is not stretched to reach the parent's row: the layout centres a
/// parent inside its children's span, so that row lies between the two ends by
/// construction.
fn paint_bus(grid: &mut Grid, geometry: &EdgeGeometry, top: u16, bottom: u16) {
    for x in geometry.parent_x.saturating_add(1)..geometry.bus_x {
        grid.place_edge_cell(x, geometry.parent_row, "─");
    }
    for row in &geometry.child_rows {
        for x in geometry.bus_x.saturating_add(1)..geometry.child_x {
            grid.place_edge_cell(x, *row, "─");
        }
    }
    for row in top..=bottom {
        let cell = bus_cell(row, top, bottom, &geometry.child_rows);
        grid.place_edge_cell(
            geometry.bus_x,
            row,
            bus_glyph(cell, row == geometry.parent_row),
        );
    }

    paint_junctions(grid, geometry, geometry.parent_row);
}

/// Turns the borders an edge meets into junctions: `├` where it leaves the
/// parent on `parent_row`, `┤` where it arrives at each child (decision 13).
fn paint_junctions(grid: &mut Grid, geometry: &EdgeGeometry, parent_row: u16) {
    grid.overwrite_text(geometry.parent_x, parent_row, "├");
    for row in &geometry.child_rows {
        grid.overwrite_text(geometry.child_x, *row, "┤");
    }
}

/// Which part of the bus the cell at `row` is. `Top` wins a tie so a bus with
/// one row reads as its own head rather than its own foot.
fn bus_cell(row: u16, top: u16, bottom: u16, child_rows: &[u16]) -> BusCell {
    if row == top {
        BusCell::Top
    } else if row == bottom {
        BusCell::Bottom
    } else if child_rows.contains(&row) {
        BusCell::Child
    } else {
        BusCell::Plain
    }
}

/// The glyph for one bus cell, chosen by the arms that actually meet in it:
/// what the bus needs at that row, the arm going right to a child on it, and
/// the arm going left when the parent's run arrives on it (decision 13).
///
/// | cell | arms without the parent | with it |
/// |---|---|---|
/// | top end | down, right — `┌` | left, down, right — `┬` |
/// | a child between the ends | up, down, right — `├` | all four — `┼` |
/// | bottom end | up, right — `└` | left, up, right — `┴` |
/// | a row the bus passes through | up, down — `│` | left, up, down — `┤` |
///
/// The corners are sharp, not rounded like the boxes': `└` is already what the
/// sidebar's guides mean by "last child", and that idiom wins.
fn bus_glyph(cell: BusCell, parent_arrives: bool) -> &'static str {
    match (cell, parent_arrives) {
        (BusCell::Top, false) => "┌",
        (BusCell::Top, true) => "┬",
        (BusCell::Child, false) => "├",
        (BusCell::Child, true) => "┼",
        (BusCell::Bottom, false) => "└",
        (BusCell::Bottom, true) => "┴",
        (BusCell::Plain, false) => "│",
        (BusCell::Plain, true) => "┤",
    }
}

#[cfg(test)]
mod tests;
