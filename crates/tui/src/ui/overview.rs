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
use crate::app::{App, region::KeyRegion};
use crate::graph::{self, Pan, paint::paint, viewport::GraphGeometry};
use crate::inspector::{self, INSPECTOR_HEIGHT, MIN_INTERIOR_FOR_PANEL, RUN_CANVAS_MIN};
use crate::theme;
use crate::tree::{self, Row, RowKind};
use ratatui::{
    Frame,
    layout::Rect,
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::Paragraph,
};

/// Splits the overview's interior into the graph canvas and the rect below it:
/// the inspector panel when `inspector_visible` and the interior has the rows
/// for one, and milestone 4.6's single line otherwise (decisions 1 and 6).
///
/// A short terminal loses the panel, never the canvas: the panel is only ever
/// carved out of an interior with six rows of canvas left above it.
///
/// `panel` is the run view's: the selected node's content rows (`inspector::panel_rows`),
/// and the panel takes them and its borders, at least milestone 4.7's eight rows and at
/// most what leaves the canvas `RUN_CANVAS_MIN` (milestone 9.0.7 decision 17), so the
/// canvas changes height as the selection moves. `None` is the project overview's
/// eight.
pub fn areas(main: Rect, inspector_visible: bool, panel: Option<u16>) -> (Rect, Rect) {
    let inner = super::inset(main);
    let footer_height = if !inspector_visible || inner.height < MIN_INTERIOR_FOR_PANEL {
        inner.height.min(1)
    } else {
        match panel {
            Some(rows) => rows
                .saturating_add(2)
                .clamp(INSPECTOR_HEIGHT, inner.height - RUN_CANVAS_MIN),
            None => INSPECTOR_HEIGHT,
        }
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

/// The run view's `panel` for [`areas`]: the selected node's content rows at the
/// panel's interior width (the overview's less the panel's borders and padding); `None`
/// outside the run view. With nothing selected the panel keeps its least height.
fn panel_of(app: &App, main: Rect, rows: &[Row<'_>]) -> Option<u16> {
    app.run_view.as_ref()?;
    let width = super::inset(main).width.saturating_sub(4);
    Some(selected_row(app, rows).map_or(0, |row| {
        inspector::panel_rows(&inspector::inspect(row, app), width)
    }))
}

/// [`areas`] as the frame splits it: the run view's panel sized by its selected node.
/// The reducer's viewport and the task panel's page read this, so they agree with
/// what is drawn (Review focus 3).
pub(crate) fn areas_of(app: &App, main: Rect) -> (Rect, Rect) {
    let rows = app.nav_rows();
    areas(main, app.inspector_visible, panel_of(app, main, &rows))
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
    /// Whether `footer` is the panel (`areas` gave it a panel's height), so what is
    /// drawn there can never disagree with the height it was drawn into.
    pub panel: bool,
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
}

pub fn view(app: &App, main: Rect) -> View {
    view_of(app, main, &app.nav_rows())
}

/// `view` for a caller that has the visible rows in hand already, so one frame
/// or one gesture builds that list once instead of once per reader.
pub fn view_of(app: &App, main: Rect, rows: &[Row<'_>]) -> View {
    let (canvas, footer) = areas(main, app.inspector_visible, panel_of(app, main, rows));
    let layout = graph::layout(rows);
    // The stored pan can outlive the canvas it was clamped against — a
    // narrowed terminal, or rows that vanished — so it is clamped on the way
    // out as well as when it is set.
    let pan = app.graph_pan.clamped(layout.size, canvas);
    View {
        canvas,
        footer,
        panel: footer.height >= INSPECTOR_HEIGHT,
        layout,
        pan,
    }
}

pub fn render(frame: &mut Frame, app: &App, area: Rect) {
    let p = app.palette();
    let (title, right) = match &app.run_view {
        Some(view) => run_title(app, &view.run_id, area.width),
        None => ("tree overview".to_string(), None),
    };
    let keys_here = app.key_region() == KeyRegion::Overview;
    let mut block = super::kit::pane_frame(Line::from(title), keys_here, p);
    if let Some(right) = right {
        let right = format!(" {} ", theme::fold(&right, p.ascii));
        block = block.title_top(Line::from(right).right_aligned());
    }
    frame.render_widget(block, area);

    // One row build for the whole frame: the layout, the painter and the
    // footer all read this list, and a frame is drawn at least ten times a
    // second. The run view's rows while it is open (milestone 8c decision 11).
    let rows = app.nav_rows();
    let view = view_of(app, area, &rows);
    let lines = paint(&view.layout, view.canvas, view.pan, &rows, app);
    frame.render_widget(Paragraph::new(lines), view.canvas);

    // What stands below the canvas is whatever `areas` made room for: the
    // panel when it gave the rect the panel's height, and the single line
    // otherwise (decisions 1, 6 and 7).
    match (view.panel, selected_row(app, &rows)) {
        (true, Some(row)) => {
            let inspection = inspector::inspect(row, app);
            inspector::render_in(frame, &inspection, view.footer, app.palette());
        }
        // A panel with nothing to inspect is left blank rather than drawn as an
        // empty box: the panel is one node spelled out, and there is no node.
        (true, None) => {}
        (false, row) => {
            let line = row.map(|row| footer_line(row, app)).unwrap_or_default();
            frame.render_widget(Paragraph::new(line), view.footer);
        }
    }
}

/// Milestone 9.0.7 decision 21: ` run · <run name> `, and right-aligned in the top border
/// ` <m>/<n> merged · <age> ` (`<age>` since approval, else creation, on the daemon's
/// clock) or ` planning · <age> `. The name is cut to what the right text leaves; the
/// right text goes first when even a short name would not fit beside it.
fn run_title(app: &App, run_id: &str, width: u16) -> (String, Option<String>) {
    let Some(run) = app.runs.runs.iter().find(|run| run.run_id == run_id) else {
        return (format!("run {}", crate::safe_text::one_line(run_id)), None);
    };
    let since = |at: u64| tree::format_elapsed(app.run_age(at));
    let right = if run.state == proto::RunState::Planning {
        format!("planning · {}", since(run.created_at))
    } else {
        let (merged, total) = tree::run_progress(run);
        let at = run.approved_at.unwrap_or(run.created_at);
        format!("{merged}/{total} merged · {}", since(at))
    };
    // The corners, the title's own spaces and `run · `, the right text's two spaces,
    // and a column between the two.
    let chrome = 2 + 2 + 6 + 1;
    let right_width = u16::try_from(right.chars().count() + 2).unwrap_or(u16::MAX);
    let (room, right) = match width.checked_sub(chrome + right_width) {
        Some(room) if room >= MIN_NAME_ROOM => (room, Some(right)),
        _ => (width.saturating_sub(chrome), None),
    };
    let name = super::kit::run_name_in(&run.goal, &run.run_id, room, app.palette());
    (format!("run · {name}"), right)
}

/// The columns a run's name keeps before the title drops its right-hand text: the
/// short id and its separator, and a few of the goal's.
const MIN_NAME_ROOM: u16 = 16;

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
    let p = app.palette();
    Line::from(vec![
        glyph,
        Span::raw(" "),
        Span::styled(theme::fold(&label, p.ascii), bold),
        Span::styled(
            theme::fold(&fields, p.ascii),
            theme::role(theme::Role::Muted, p),
        ),
    ])
}

/// One node's footer: its status glyph in its status colour, the text it is
/// known by, and the fields that follow it.
pub(super) fn footer_parts(row: &Row<'_>, app: &App) -> (Span<'static>, String, String) {
    let (frame, ascii) = (app.spinner_frame, app.palette().ascii);
    let look = |look| crate::inspector::look(look, app);
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
                look(theme::status_look(*status, frame, ascii)),
                name.clone(),
                format!(
                    "  {root}  {}  {}",
                    status.label(),
                    tree_view::counts_text(*counts)
                ),
            )
        }
        RowKind::Window { info, position, .. } => (
            look(theme::status_look(info.status, frame, ascii)),
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
                look(theme::subagent_look(info, frame, ascii)),
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
        // Every run kind's single line is its inspection's title: the glyph, the
        // name, then two spaces and the right-hand text (milestone 8c, Interfaces
        // "The single line"). The glyph is the canvas's own, by construction.
        RowKind::Run { .. }
        | RowKind::Planner { .. }
        | RowKind::Scout { .. }
        | RowKind::Task { .. }
        | RowKind::Stage { .. }
        | RowKind::AgentRound { .. } => {
            let inspection = inspector::inspect(row, app);
            let right = inspection
                .right
                .map(|right| format!("  {right}"))
                .unwrap_or_default();
            (inspection.glyph, inspection.name, right)
        }
    }
}

/// How long a sub-agent that has stopped ran for. Both fields are ages, so the
/// run is the difference between them, and a missing end reads as zero.
fn finished_secs(info: &proto::SubagentInfo) -> u64 {
    info.ended_secs
        .map_or(0, |ended| info.started_secs.saturating_sub(ended))
}

#[cfg(test)]
#[path = "overview_polish_tests.rs"]
mod polish_tests;
