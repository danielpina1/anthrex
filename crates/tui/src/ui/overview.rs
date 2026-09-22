//! The full-screen graph `C-b T` opens: the agents drawn as boxes joined by
//! lines, on a canvas that pans, with the selected node spelled out in full
//! along the bottom (spec §4.2).
//!
//! The rows themselves carry only a status glyph and a name, so everything the
//! old aligned columns showed — the model, the state, the elapsed time — lives
//! below the canvas: in the inspector panel (milestone 4.7), or in milestone
//! 4.6's single line when the panel is toggled off or the terminal is too
//! short for it.

use super::tree_view;
use crate::graph::{self, Pan, paint::paint, viewport::GraphGeometry};
use crate::inspector::{self, INSPECTOR_HEIGHT, MIN_INTERIOR_FOR_PANEL};
use crate::tree::{self, Row, RowKind};
use crate::{app::App, theme};
use ratatui::{
    Frame,
    layout::Rect,
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, Paragraph},
};

/// Splits the overview's interior into the graph canvas and the rect below it:
/// the inspector panel when `inspector_visible` and the interior has the rows
/// for one, and milestone 4.6's single line otherwise (decisions 1 and 6).
///
/// A short terminal loses the panel, never the canvas: the panel is only ever
/// carved out of an interior with six rows of canvas left above it.
pub fn areas(main: Rect, inspector_visible: bool) -> (Rect, Rect) {
    let inner = super::inset(main);
    let footer_height = if inspector_visible && inner.height >= MIN_INTERIOR_FOR_PANEL {
        INSPECTOR_HEIGHT
    } else {
        inner.height.min(1)
    };
    let canvas = Rect {
        height: inner.height - footer_height,
        ..inner
    };
    let footer = Rect {
        y: inner.y + canvas.height,
        height: footer_height,
        ..inner
    };
    (canvas, footer)
}

/// One frame of the overview: where it sits on screen, the graph laid out on
/// the canvas, and the pan to draw it at.
///
/// The renderer and every mouse gesture read this one function, so a click is
/// hit-tested against exactly the geometry the frame under it was drawn with.
pub struct View {
    pub canvas: Rect,
    /// The rect below the canvas: the inspector panel, or the single line.
    pub footer: Rect,
    pub layout: graph::Layout,
    pub pan: Pan,
}

impl View {
    pub fn geometry(&self) -> GraphGeometry {
        GraphGeometry {
            area: self.canvas,
            pan: self.pan,
        }
    }

    /// Whether the rect below the canvas is the panel. It is the panel exactly
    /// when `areas` gave it the panel's height, so what is drawn there can
    /// never disagree with the height it was drawn into.
    fn shows_panel(&self) -> bool {
        self.footer.height >= INSPECTOR_HEIGHT
    }
}

pub fn view(app: &App, main: Rect) -> View {
    view_of(app, main, &app.rows())
}

/// `view` for a caller that has the visible rows in hand already, so one frame
/// or one gesture builds that list once instead of once per reader.
pub fn view_of(app: &App, main: Rect, rows: &[Row<'_>]) -> View {
    let (canvas, footer) = areas(main, app.inspector_visible);
    let layout = graph::layout(rows);
    // The stored pan can outlive the canvas it was clamped against — a
    // narrowed terminal, or rows that vanished — so it is clamped on the way
    // out as well as when it is set.
    let pan = app.graph_pan.clamped(layout.size, canvas);
    View {
        canvas,
        footer,
        layout,
        pan,
    }
}

pub fn render(frame: &mut Frame, app: &App, area: Rect) {
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(if app.modal.is_none() {
            theme::border_focused(app.settings.accent)
        } else {
            theme::border()
        })
        .title(Line::from(Span::styled(
            " tree overview ",
            theme::title(app.settings.accent),
        )));
    frame.render_widget(block, area);

    // One row build for the whole frame: the layout, the painter and the
    // footer all read this list, and a frame is drawn at least ten times a
    // second.
    let rows = app.rows();
    let view = view_of(app, area, &rows);
    let lines = paint(&view.layout, view.canvas, view.pan, &rows, app);
    frame.render_widget(Paragraph::new(lines), view.canvas);

    // What stands below the canvas is whatever `areas` made room for: the
    // panel when it gave the rect the panel's height, and the single line
    // otherwise (decisions 1, 6 and 7).
    match (view.shows_panel(), selected_row(app, &rows)) {
        (true, Some(row)) => inspector::render(frame, &inspector::inspect(row, app), view.footer),
        // A panel with nothing to inspect is left blank rather than drawn as an
        // empty box: the panel is one node spelled out, and there is no node.
        (true, None) => {}
        (false, row) => {
            let line = row.map(|row| footer_line(row, app)).unwrap_or_default();
            frame.render_widget(Paragraph::new(line), view.footer);
        }
    }
}

/// The row the overview's selection names, if it is still on screen.
fn selected_row<'a, 'b>(app: &App, rows: &'a [Row<'b>]) -> Option<&'a Row<'b>> {
    let key = app.tree.selected.as_ref()?;
    rows.iter().find(|row| &row.key == key)
}

/// The selected node in full: its label untruncated, then its model, its state
/// and how long it has been in it (decision 18).
///
/// Only the line's own width cuts anything here, which is why the box above can
/// afford to elide: whatever a box hides, this line shows. The panel replaces
/// it (decision 1); it is what `i` turns back on and what a short terminal
/// keeps (decisions 6 and 7).
fn footer_line(row: &Row<'_>, app: &App) -> Line<'static> {
    let bold = Style::default().add_modifier(Modifier::BOLD);
    let (glyph, label, fields) = footer_parts(row, app);
    Line::from(vec![
        glyph,
        Span::raw(" "),
        Span::styled(label, bold),
        Span::styled(fields, theme::muted()),
    ])
}

/// One node's footer: its status glyph in its status colour, the text it is
/// known by, and the fields that follow it.
fn footer_parts(row: &Row<'_>, app: &App) -> (Span<'static>, String, String) {
    match &row.kind {
        RowKind::Project {
            root,
            name,
            status,
            counts,
            ..
        } => {
            let root = super::terminal::shorten_home(root);
            let root = if root == "~/" { "~" } else { &root };
            (
                Span::styled(
                    theme::status_glyph(*status, app.spinner_frame),
                    Style::default().fg(theme::status_color(*status)),
                ),
                name.clone(),
                format!(
                    "  {root}  {}  {}",
                    status.label(),
                    tree_view::counts_text(*counts)
                ),
            )
        }
        RowKind::Window { info, position, .. } => (
            Span::styled(
                theme::status_glyph(info.status, app.spinner_frame),
                Style::default().fg(theme::status_color(info.status)),
            ),
            format!("{position} {}", info.name),
            format!(
                "  {}  {}  {}  {}{}",
                info.runtime.label(),
                info.model.as_deref().unwrap_or("-"),
                info.status.label(),
                tree::format_elapsed(app.elapsed_secs(info)),
                info.tool
                    .as_deref()
                    .map(|tool| format!("  {tool}"))
                    .unwrap_or_default()
            ),
        ),
        RowKind::Subagent { info } => {
            let (state, duration) = match info.state {
                proto::SubagentState::Running => ("running", app.age_secs(info.started_secs)),
                proto::SubagentState::Done => ("done", finished_secs(info)),
                proto::SubagentState::Failed => ("failed", finished_secs(info)),
            };
            (
                Span::styled(
                    theme::subagent_glyph(info, app.spinner_frame),
                    Style::default().fg(theme::subagent_color(info)),
                ),
                tree::subagent_label(info),
                format!(
                    "  {}  {state}  {}{}",
                    info.model.as_deref().unwrap_or("-"),
                    tree::format_elapsed(duration),
                    info.tool
                        .as_deref()
                        .map(|tool| format!("  {tool}"))
                        .unwrap_or_default()
                ),
            )
        }
    }
}

/// How long a sub-agent that has stopped ran for. Both fields are ages, so the
/// run is the difference between them, and a missing end reads as zero.
fn finished_secs(info: &proto::SubagentInfo) -> u64 {
    info.ended_secs
        .map_or(0, |ended| info.started_secs.saturating_sub(ended))
}
