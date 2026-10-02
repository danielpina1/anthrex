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
use crate::tree::{self, Row};
use ratatui::{
    Frame,
    layout::Rect,
    text::{Line, Span},
    widgets::Paragraph,
};
use unicode_width::UnicodeWidthStr;

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
    let inspection = selected_row(app, rows).map(|row| inspector::inspect(row, app));
    panel_for(app, main, inspection.as_ref())
}

/// [`panel_of`] for a selected node already inspected: the frame inspects it once and
/// both sizes and draws the panel from that (final fix wave, task 9's deferred minor).
fn panel_for(app: &App, main: Rect, inspection: Option<&inspector::Inspection>) -> Option<u16> {
    app.run_view.as_ref()?;
    let width = super::inset(main).width.saturating_sub(4);
    Some(inspection.map_or(0, |i| inspector::panel_rows(i, width)))
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
    /// Decision 22: the canvas draws `run_list` instead of the graph.
    pub list: bool,
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
    view_sized(app, main, rows, panel_of(app, main, rows))
}

/// [`view_of`] with the run view's panel rows in hand (`panel_for`).
fn view_sized(app: &App, main: Rect, rows: &[Row<'_>], panel: Option<u16>) -> View {
    let (canvas, footer) = areas(main, app.inspector_visible, panel);
    let layout = graph::layout(rows);
    // The stored pan can outlive the canvas it was clamped against — a
    // narrowed terminal, or rows that vanished — so it is clamped on the way
    // out as well as when it is set.
    let pan = app.graph_pan.clamped(layout.size, canvas);
    View {
        canvas,
        footer,
        panel: footer.height >= INSPECTOR_HEIGHT,
        list: app.run_view.is_some() && super::inset(main).height < super::run_list::RUN_LIST_BELOW,
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
        // Muted, as the sidebar's marks: unstyled it would take the border's accent.
        let right = format!(" {} ", theme::fold(&right, p.ascii));
        let muted = theme::role(theme::Role::Muted, p);
        block = block.title_top(Line::from(Span::styled(right, muted)).right_aligned());
    }
    frame.render_widget(block, area);

    // One row build for the whole frame: the layout, the painter and the
    // footer all read this list, and a frame is drawn at least ten times a
    // second. The run view's rows while it is open (milestone 8c decision 11).
    let rows = app.nav_rows();
    let selected = selected_row(app, &rows);
    let inspection = selected.map(|row| inspector::inspect(row, app));
    let view = view_sized(app, area, &rows, panel_for(app, area, inspection.as_ref()));
    if view.list {
        super::run_list::render(frame, app, view.canvas, &rows);
    } else {
        let lines = paint(&view.layout, view.canvas, view.pan, &rows, app);
        frame.render_widget(Paragraph::new(lines), view.canvas);
    }

    // What stands below the canvas is whatever `areas` made room for: the
    // panel when it gave the rect the panel's height, and the single line
    // otherwise (decisions 1, 6 and 7).
    match (view.panel, selected) {
        (true, Some(_)) => {
            if let Some(inspection) = &inspection {
                inspector::render_in(frame, inspection, view.footer, app.palette());
            }
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
/// right text goes first when even a short name would not fit beside it. Milestone 9.2
/// decision 36: a delivering `pr` run's reads ` <m>/<n> merged · delivering · <age> `,
/// else ` delivering · <age> ` (review finding I1: never wider than the plain form,
/// `delivering` being as wide as `<m>/<n> merged` with one-digit counts and narrower
/// past them, so the state word shows wherever the plain form would).
fn run_title(app: &App, run_id: &str, width: u16) -> (String, Option<String>) {
    let Some(run) = app.runs.runs.iter().find(|run| run.run_id == run_id) else {
        return (format!("run {}", crate::safe_text::one_line(run_id)), None);
    };
    let since = |at: u64| tree::format_elapsed(app.run_age(at));
    let rights = if run.state == proto::RunState::Planning {
        vec![format!("planning · {}", since(run.created_at))]
    } else {
        let (merged, total) = tree::run_progress(run);
        let at = run.approved_at.unwrap_or(run.created_at);
        let plain = format!("{merged}/{total} merged · {}", since(at));
        match crate::inspector::delivering(run) {
            true => vec![
                format!("{merged}/{total} merged · delivering · {}", since(at)),
                format!("delivering · {}", since(at)),
            ],
            false => vec![plain],
        }
    };
    // The corners, the title's own spaces and `run · `, the right text's two spaces,
    // and a column between the two.
    let chrome = 2 + 2 + 6 + 1;
    let fits = |right: &String| {
        let right_width = u16::try_from(right.width() + 2).unwrap_or(u16::MAX);
        width
            .checked_sub(chrome + right_width)
            .filter(|room| *room >= MIN_NAME_ROOM)
    };
    let (room, right) = match rights
        .into_iter()
        .find_map(|r| fits(&r).map(|room| (room, r)))
    {
        Some((room, right)) => (room, Some(right)),
        None => (width.saturating_sub(chrome), None),
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

#[path = "overview_footer.rs"]
mod footer;
use footer::footer_line;
#[cfg(test)]
pub(super) use footer::footer_parts;

#[cfg(test)]
#[path = "overview_polish_tests.rs"]
mod polish_tests;

#[cfg(test)]
#[path = "overview_title_tests.rs"]
mod title_tests;
