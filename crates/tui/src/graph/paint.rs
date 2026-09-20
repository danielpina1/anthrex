//! The painter: fills a character grid the size of the viewport with every
//! placed node's box, offset by the pan, then converts the grid to ratatui
//! `Line`s (decision 9). That keeps painting testable by exact strings, with
//! no terminal involved.
//!
//! Boxes only. Edges live in the `TIER_GAP` columns between tiers and are the
//! next task's job (decision 13); this module never writes into them.

use super::{Layout, Pan};
use crate::app::App;
use crate::theme;
use crate::tree::{Row, RowKind};
use crate::ui::tree_view::truncate;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthStr;

/// Fills a character grid the size of `area` with every node `layout` places,
/// offset by `pan`, and converts it to lines.
///
/// A box that only partly overlaps the viewport is clipped one cell at a
/// time rather than dropped whole: each cell is placed independently, so
/// whatever falls inside `area` survives even when the box's own origin does
/// not.
pub fn paint(layout: &Layout, area: Rect, pan: Pan, app: &App) -> Vec<Line<'static>> {
    let mut grid = Grid::new(area, pan);
    for row in app.rows() {
        if let Some(node) = layout.node(&row.key) {
            paint_node(&mut grid, node.rect, &row, app);
        }
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

    /// Writes one cell at a canvas coordinate. A coordinate above or left of
    /// `pan`, or past the grid's own width or height, is silently dropped —
    /// that is the clipping this module promises.
    fn place(&mut self, canvas_x: u16, canvas_y: u16, text: String, style: Style) {
        let Some(x) = canvas_x.checked_sub(self.pan.x) else {
            return;
        };
        let Some(y) = canvas_y.checked_sub(self.pan.y) else {
            return;
        };
        if x >= self.width || y >= self.height {
            return;
        }
        let index = usize::from(y) * usize::from(self.width) + usize::from(x);
        self.cells[index] = Cell { text, style };
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
    for cell in cells {
        match spans.last_mut() {
            Some(last) if last.style == cell.style => {
                let mut content = last.content.to_string();
                content.push_str(&cell.text);
                last.content = content.into();
            }
            _ => spans.push(Span::styled(cell.text.clone(), cell.style)),
        }
    }
    spans
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

#[cfg(test)]
mod tests;
