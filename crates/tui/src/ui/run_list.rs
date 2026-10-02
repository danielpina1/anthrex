//! Milestone 9.0.7 decision 22: below 30 rows of interior the run view's canvas is a
//! compact list, one row a node of `App::nav_rows`, in their order: `<glyph> <text>`
//! indented two columns a depth below the root, and for a task the aligned columns
//! `<S|M|L> <mode>`, its state word (decision 12's) and `after <deps>` (decision 19).
//!
//! The window is stateless: [`first_row`] keeps the selection in view, centred where
//! it can, and the renderer and the mouse ([`row_at`]) both read it, so a click lands
//! on the row drawn under it (Review focus 3). Pure: no I/O, `&App` only.

use crate::app::App;
use crate::graph::{content_text_in, paint::style::node_glyph, run_text::origin_tag};
use crate::inspector::{
    run_format::{size_letter, test_mode_text},
    state_word,
};
use crate::safe_text::one_line;
use crate::theme::{self, Glyph, Role};
use crate::tree::{Row, RowKind};
use crate::ui::tree_view::truncate_in;
use ratatui::{
    Frame,
    layout::Rect,
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::Paragraph,
};
use unicode_width::UnicodeWidthStr;

/// The run view draws this list while its frame's interior is under this many rows.
pub const RUN_LIST_BELOW: u16 = 30;

/// The title columns a task keeps before its deps are cut (decision 22).
const MIN_TITLE: usize = 8;
/// The state word's width once it is cut, before the size and mode go.
const STATE_CUT: usize = 12;
/// The columns between two columns.
const GAP: usize = 2;

/// The index of the row drawn on the list's first row: the selection centred where it
/// can be, else as near the middle as the ends allow. With room for marks (three rows
/// or more), the selection never lands on a mark's row.
pub fn first_row(len: usize, selected: usize, height: usize) -> usize {
    if len <= height || height == 0 {
        return 0;
    }
    let half = if height < 3 { 0 } else { height / 2 };
    selected.min(len - 1).saturating_sub(half).min(len - height)
}

/// Whether the first and the last row are `↑`/`↓ n more` marks (each takes the row of
/// the node it hides). Under three rows there are none: the rows go to the nodes.
fn marks(len: usize, first: usize, height: usize) -> (bool, bool) {
    if height < 3 || len <= height {
        return (false, false);
    }
    (first > 0, first + height < len)
}

/// The index into the rows of the node drawn at `(column, row)` in `area`, or `None`
/// off the list, on a mark, or below the last row.
pub fn row_at(area: Rect, len: usize, selected: usize, column: u16, row: u16) -> Option<usize> {
    if !area.contains((column, row).into()) {
        return None;
    }
    let height = usize::from(area.height);
    let first = first_row(len, selected, height);
    let at = usize::from(row - area.y);
    let (up, down) = marks(len, first, height);
    if (up && at == 0) || (down && at + 1 == height) {
        return None;
    }
    let index = first + at;
    (index < len).then_some(index)
}

/// Draws `rows` as the list in `area` (the run view's canvas).
pub fn render(frame: &mut Frame, app: &App, area: Rect, rows: &[Row<'_>]) {
    let (height, width) = (usize::from(area.height), usize::from(area.width));
    let p = app.palette();
    let at = app.tree.selected_index(rows).unwrap_or(0);
    // Shown as `graph::paint` shows a box's: only while tree navigation is on.
    let shown = app
        .tree_input
        .is_some()
        .then(|| app.tree.selected_index(rows))
        .flatten();
    let first = first_row(rows.len(), at, height);
    let (up, down) = marks(rows.len(), first, height);
    let columns = Columns::of(rows, width, p.ascii);
    let below = rows
        .len()
        .saturating_sub((first + height).saturating_sub(1));
    let above = first + 1;
    let (up_mark, down_mark) = super::kit::scroll_marks(above, below, p.ascii);
    let muted = theme::role(Role::Muted, p);
    let lines: Vec<Line<'static>> = (0..height.min(rows.len()))
        .map(|r| match (r == 0 && up, r + 1 == height && down) {
            (true, _) => Line::styled(up_mark.clone().unwrap_or_default(), muted),
            (_, true) => Line::styled(down_mark.clone().unwrap_or_default(), muted),
            _ => {
                let index = first + r;
                row_line(&rows[index], app, &columns, width, shown == Some(index))
            }
        })
        .collect();
    frame.render_widget(Paragraph::new(lines), area);
}

/// A task's columns, measured once over every task row so they align (decision 22).
struct Columns {
    /// The column at which every task's title ends (its padding included).
    label_end: usize,
    /// `<S|M|L> <mode>`'s width, `None` once the column is dropped.
    size: Option<usize>,
    state: usize,
    /// The room for `after <deps>`: the widest, or what is left once cut.
    deps: usize,
}

/// A task row's parts, sanitised and folded, before they are cut.
struct TaskParts {
    indent: usize,
    id: String,
    title: String,
    size: String,
    state: String,
    deps: String,
}

fn task_parts(row: &Row<'_>, ascii: bool) -> Option<TaskParts> {
    let RowKind::Task { run, task } = &row.kind else {
        return None;
    };
    let text = |s: &str| theme::fold(&one_line(s), ascii);
    let title = format!("{}{}", task.title.trim(), origin_tag(task.origin));
    // Final fix wave M4: the plan review's formatter, implicit deps marked.
    let deps = text(&crate::inspector::run_format::after_text(task));
    Some(TaskParts {
        indent: indent(row),
        id: text(&task.id),
        title: text(title.trim()),
        size: format!(
            "{} {}",
            size_letter(task.size),
            test_mode_text(task.test_mode)
        ),
        state: text(&state_word(run, task)),
        deps,
    })
}

fn indent(row: &Row<'_>) -> usize {
    usize::from(row.depth).saturating_mul(2)
}

fn width_of(text: &str) -> usize {
    UnicodeWidthStr::width(text)
}

impl Columns {
    /// The title is cut first (to `MIN_TITLE` columns), then the state word (to
    /// `STATE_CUT`), then the size and mode go, and only then are the deps cut. What
    /// the later cuts free goes back to the state word, then to the titles.
    fn of(rows: &[Row<'_>], width: usize, ascii: bool) -> Self {
        let tasks: Vec<TaskParts> = rows.iter().filter_map(|r| task_parts(r, ascii)).collect();
        let max = |f: &dyn Fn(&TaskParts) -> usize| tasks.iter().map(f).max().unwrap_or(0);
        let head = |t: &TaskParts, title: usize| {
            let title = title.min(width_of(&t.title));
            t.indent + 2 + width_of(&t.id) + usize::from(title > 0) + title
        };
        let want = max(&|t| head(t, usize::MAX));
        let least = max(&|t| head(t, MIN_TITLE));
        let deps = max(&|t| width_of(&t.deps));
        let state = max(&|t| width_of(&t.state));
        let mut columns = Columns {
            label_end: want,
            size: Some(max(&|t| width_of(&t.size))),
            state,
            deps,
        };
        let rest = |c: &Columns| {
            c.size.map_or(0, |s| GAP + s) + GAP + c.state + if deps > 0 { GAP + deps } else { 0 }
        };
        columns.label_end = width.saturating_sub(rest(&columns)).clamp(least, want);
        if columns.label_end + rest(&columns) > width {
            columns.state = columns.state.min(STATE_CUT);
        }
        if columns.label_end + rest(&columns) > width {
            columns.size = None;
        }
        // What the cuts after the title freed goes back to the state word first, up to
        // its whole width, then to the titles: narrowing never lengthens a title while
        // the state word is cut (fix round 1, I3).
        let others = rest(&columns) - columns.state;
        let room = width.saturating_sub(least + others);
        columns.state = room.clamp(columns.state, state);
        columns.label_end = width.saturating_sub(rest(&columns)).clamp(least, want);
        let used = columns.label_end + rest(&columns) - deps;
        columns.deps = deps.min(width.saturating_sub(used));
        columns
    }
}

/// One node's row: the bar in the first column while it is the shown selection, the
/// rest reversed (decision 20).
fn row_line(
    row: &Row<'_>,
    app: &App,
    columns: &Columns,
    width: usize,
    selected: bool,
) -> Line<'static> {
    let p = app.palette();
    let (glyph, role) = node_glyph(row, app);
    let muted = theme::role(Role::Muted, p);
    let plain = Style::default();
    let mut spans = vec![Span::styled(glyph, theme::role(role, p)), Span::raw(" ")];
    let indent = indent(row);
    match task_parts(row, p.ascii) {
        Some(task) => {
            let room = columns.label_end.saturating_sub(indent + 2);
            let label = if task.title.is_empty() {
                task.id
            } else {
                format!("{} {}", task.id, task.title)
            };
            let pad = |text: String, to: usize| {
                let text = truncate_in(&text, to, p.ascii);
                let fill = to.saturating_sub(width_of(&text));
                format!("{text}{}", " ".repeat(fill))
            };
            spans.push(Span::styled(pad(label, room), plain));
            if let Some(size) = columns.size {
                spans.push(Span::raw(" ".repeat(GAP)));
                spans.push(Span::styled(pad(task.size, size), muted));
            }
            spans.push(Span::raw(" ".repeat(GAP)));
            spans.push(Span::styled(pad(task.state, columns.state), plain));
            if !task.deps.is_empty() && columns.deps > 0 {
                spans.push(Span::raw(" ".repeat(GAP)));
                spans.push(Span::styled(
                    truncate_in(&task.deps, columns.deps, p.ascii),
                    muted,
                ));
            }
        }
        None => {
            let text = theme::fold(&content_text_in(row, p.ascii), p.ascii);
            let room = width.saturating_sub(indent + 2);
            spans.push(Span::styled(truncate_in(&text, room, p.ascii), plain));
        }
    }
    if !selected {
        spans.insert(0, Span::raw(" ".repeat(indent)));
        return Line::from(spans);
    }
    // The bar takes the indent's first column; a root, with none, moves one column.
    let used: usize = spans.iter().map(|s| s.content.width()).sum();
    let fill = width.saturating_sub(indent.max(1) + used);
    spans.push(Span::raw(" ".repeat(fill)));
    // In the accent only where the keys are (decision 1), `Muted` under a modal.
    let keys_here = app.key_region() == crate::app::region::KeyRegion::Overview;
    let colour = if keys_here { Role::Accent } else { Role::Muted };
    let mut out = vec![
        Span::styled(
            theme::glyph(Glyph::Selection, p.ascii),
            theme::role(colour, p),
        ),
        Span::raw(" ".repeat(indent.saturating_sub(1))),
    ];
    out.extend(spans.into_iter().map(|span| {
        let style = span.style.add_modifier(Modifier::REVERSED);
        Span::styled(span.content, style)
    }));
    Line::from(out)
}

#[cfg(test)]
#[path = "run_list_tests.rs"]
pub(crate) mod tests;

#[cfg(test)]
#[path = "run_list_mouse_tests.rs"]
mod mouse_tests;
