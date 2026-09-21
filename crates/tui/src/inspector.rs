//! The node inspector: everything anthrex knows about the selected node, in a
//! bordered panel below the graph overview's canvas (spec §3).
//!
//! Two halves, tested separately (decision 13). `inspect` projects a `Row` and
//! the `App` behind it into an `Inspection` — labels and values, and nothing
//! that knows about a terminal. `render` lays that `Inspection` out. The
//! projection is asserted by exact field lists, the renderer by exact rendered
//! strings, the way the graph's painter is.

use crate::app::App;
use crate::theme;
use crate::tree::{self, NodeKey, Row, RowKind, RuntimeCounts};
use crate::ui::statusbar::git_spans;
use crate::ui::terminal::shorten_home;
use crate::ui::tree_view::{counts_text, truncate};
use proto::{GitState, Head, Status, SubagentInfo, SubagentState, WindowInfo};
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Borders, Padding, Paragraph};
use std::collections::BTreeSet;
use std::path::Path;
use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthStr;

/// The panel's own height: a border, the title row, five field rows, a border
/// (decision 2).
pub const INSPECTOR_HEIGHT: u16 = 8;

/// Below this many rows of overview interior the panel gives way to milestone
/// 4.6's single line, so a short terminal loses the inspector and never the
/// canvas (decision 6).
pub const MIN_INTERIOR_FOR_PANEL: u16 = INSPECTOR_HEIGHT + 6;

/// The two spaces between a label and its value — spec §2's `model  opus`.
const LABEL_GAP: usize = 2;

/// The two spaces between one column and the next (decision 4).
const GUTTER: usize = 2;

/// One labelled value. `wrap` marks the one field a column may not elide: the
/// sub-agent's task, which the panel exists to show whole (decision 5).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Field {
    pub label: &'static str,
    pub value: String,
    pub wrap: bool,
}

/// One node, projected: its status glyph in its status colour, the name it is
/// known by, and its fields in the order decisions 8 to 10 give them.
#[derive(Debug, Clone, PartialEq)]
pub struct Inspection {
    pub glyph: Span<'static>,
    pub name: String,
    pub fields: Vec<Field>,
}

fn field(label: &'static str, value: impl Into<String>) -> Field {
    Field {
        label,
        value: value.into(),
        wrap: false,
    }
}

/// Everything anthrex knows about one node. Pure: it reads `app`, and computes
/// strings (decision 13).
pub fn inspect(row: &Row<'_>, app: &App) -> Inspection {
    match &row.kind {
        RowKind::Project {
            root,
            name,
            status,
            counts,
            ..
        } => Inspection {
            glyph: status_span(*status, app),
            name: name.clone(),
            fields: project_fields(root, *status, *counts, app),
        },
        RowKind::Window { info, position, .. } => Inspection {
            glyph: status_span(info.status, app),
            name: format!("{position} {}", info.name),
            fields: window_fields(info, app),
        },
        RowKind::Subagent { info } => Inspection {
            glyph: Span::styled(
                theme::subagent_glyph(info, app.spinner_frame),
                Style::default().fg(theme::subagent_color(info)),
            ),
            name: tree::subagent_label(info),
            fields: subagent_fields(row, info, app),
        },
    }
}

fn status_span(status: Status, app: &App) -> Span<'static> {
    Span::styled(
        theme::status_glyph(status, app.spinner_frame),
        Style::default().fg(theme::status_color(status)),
    )
}

/// A project's path, status and runtime counts, and its git state only when
/// every window in it stands in one worktree (decision 8).
fn project_fields(root: &Path, status: Status, counts: RuntimeCounts, app: &App) -> Vec<Field> {
    let mut fields = vec![
        field("path", display_path(root)),
        field("status", status.label()),
        field("agents", counts_text(counts)),
    ];
    // Ordered and deduplicated: two windows in the same worktree are one
    // worktree, and the count below has to say so.
    let worktrees: BTreeSet<&Path> = app
        .windows
        .iter()
        .filter(|window| window.project == root)
        .filter_map(|window| window.worktree.as_deref())
        .collect();
    match worktrees.len() {
        0 => {}
        // Picking one of several arbitrarily would be a lie, so the field
        // degrades to a count and `changes` is left out entirely.
        1 => {
            if let Some(state) = worktrees.iter().next().and_then(|root| app.git.get(*root)) {
                fields.push(field("branch", head_text(&state.head)));
                fields.push(field("changes", changes_text(state)));
            }
        }
        count => fields.push(field("branch", format!("{count} worktrees"))),
    }
    fields
}

/// A window's runtime, model, status and timings, where it stands, and what it
/// has running under it (decision 9).
fn window_fields(info: &WindowInfo, app: &App) -> Vec<Field> {
    let mut fields = vec![
        field("runtime", info.runtime.label()),
        field("model", info.model.as_deref().unwrap_or("-")),
        field(
            "status",
            with_tool(info.status.label(), info.tool.as_deref()),
        ),
        field("for", tree::format_elapsed(app.elapsed_secs(info))),
        field("dir", display_path(&info.cwd)),
    ];
    if let Some(worktree) = info.worktree.as_deref()
        && worktree != info.cwd
    {
        fields.push(field("worktree", display_path(worktree)));
    }
    // Keyed by the worktree root, and absent state omits the field rather than
    // showing it empty (decision 12).
    if let Some(state) = info.worktree.as_deref().and_then(|root| app.git.get(root)) {
        fields.push(field("branch", git_text(state)));
    }
    if let Some(session) = info.session_id.as_deref() {
        fields.push(field("session", session));
    }
    if !info.subagents.is_empty() {
        let running = info
            .subagents
            .iter()
            .filter(|subagent| subagent.state == SubagentState::Running)
            .count();
        fields.push(field(
            "sub-agents",
            format!("{}, {running} running", info.subagents.len()),
        ));
    }
    fields
}

/// A sub-agent's kind, task, model, state, timing, parentage and depth
/// (decision 10). The task is the one field marked for wrapping.
fn subagent_fields(row: &Row<'_>, info: &SubagentInfo, app: &App) -> Vec<Field> {
    let (state, duration) = match info.state {
        SubagentState::Running => ("running", app.age_secs(info.started_secs)),
        SubagentState::Done => ("done", finished_secs(info)),
        SubagentState::Failed => ("failed", finished_secs(info)),
    };
    let mut fields = vec![field("kind", info.kind.clone())];
    if let Some(label) = info.label.as_deref() {
        fields.push(Field {
            label: "task",
            value: label.to_owned(),
            wrap: true,
        });
    }
    fields.push(field("model", info.model.as_deref().unwrap_or("-")));
    fields.push(field("state", with_tool(state, info.tool.as_deref())));
    fields.push(field("for", tree::format_elapsed(duration)));
    fields.push(field("spawned by", spawned_by(row, info, app)));
    // A window's own sub-agents sit at row depth 2, and one level below the
    // window is what "depth" means here.
    fields.push(field("depth", row.depth.saturating_sub(1).to_string()));
    fields
}

/// The parent sub-agent's label when `parent_id` names one still in the
/// window's list, and the owning window otherwise — including when `parent_id`
/// names a sub-agent that has since gone (decision 11).
///
/// This is the field the milestone exists for: a sub-agent three levels down
/// looks exactly like one directly under its window.
fn spawned_by(row: &Row<'_>, info: &SubagentInfo, app: &App) -> String {
    let NodeKey::Subagent { window_id, .. } = &row.key else {
        return "-".to_owned();
    };
    let Some(window) = app.windows.iter().find(|window| window.id == *window_id) else {
        return "-".to_owned();
    };
    if let Some(parent) = info.parent_id.as_deref().and_then(|parent_id| {
        window
            .subagents
            .iter()
            .find(|subagent| subagent.id == parent_id)
    }) {
        return tree::subagent_label(parent);
    }
    // The same number the window's own box and the sidebar show, so the two
    // can be matched up by eye.
    match tree::agent_order(&app.rows())
        .iter()
        .position(|id| *id == window.id)
    {
        Some(index) => format!("{} {}", index + 1, window.name),
        None => window.name.clone(),
    }
}

/// How long a sub-agent that has stopped ran for. Both fields are ages, so the
/// run is the difference between them, and a missing end reads as zero.
fn finished_secs(info: &SubagentInfo) -> u64 {
    info.ended_secs
        .map_or(0, |ended| info.started_secs.saturating_sub(ended))
}

fn with_tool(label: &str, tool: Option<&str>) -> String {
    match tool {
        Some(tool) => format!("{label} · {tool}"),
        None => label.to_owned(),
    }
}

fn display_path(path: &Path) -> String {
    let text = shorten_home(path);
    if text == "~/" { "~".to_owned() } else { text }
}

/// Where a worktree's `HEAD` points, in the idiom the status bar already uses.
fn head_text(head: &Head) -> String {
    match head {
        Head::Branch(name) | Head::Unborn(name) => name.clone(),
        Head::Detached(oid) => format!("@{oid}"),
    }
}

/// What is uncommitted in a worktree, for the project's `changes` field.
fn changes_text(state: &GitState) -> String {
    let mut parts = Vec::new();
    if state.conflicts > 0 {
        parts.push(format!("⚠{}", state.conflicts));
    }
    if state.dirty > 0 {
        parts.push(format!("●{}", state.dirty));
    }
    if state.untracked > 0 {
        parts.push(format!("?{}", state.untracked));
    }
    if parts.is_empty() {
        "clean".to_owned()
    } else {
        parts.join(" ")
    }
}

/// A window's branch with its dirty and ahead/behind counts, built by the one
/// function that already decides what a worktree's git state reads as.
fn git_text(state: &GitState) -> String {
    git_spans(state, usize::MAX)
        .iter()
        .map(|span| span.content.as_ref())
        .collect()
}

/// Lays an `Inspection` into a rounded panel: the title row, then the fields in
/// columns below it.
pub fn render(frame: &mut Frame, inspection: &Inspection, area: Rect) {
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(theme::border())
        // One column in from the border on each side, so the fields do not sit
        // flush against it the way the graph's boxes never do.
        .padding(Padding::horizontal(1));
    let inner = block.inner(area);
    frame.render_widget(block, area);
    if inner.width == 0 || inner.height == 0 {
        return;
    }
    let lines = lines(
        inspection,
        usize::from(inner.width),
        usize::from(inner.height),
    );
    frame.render_widget(Paragraph::new(lines), inner);
}

/// The panel's interior, row by row: the title, the fields that fit in columns,
/// and the wrapping field across whatever rows are left.
///
/// Never more than `height` lines: fields past the last slot are dropped from
/// the end rather than spilling out of the panel (decision 2).
fn lines(inspection: &Inspection, width: usize, height: usize) -> Vec<Line<'static>> {
    let mut out = vec![title_line(inspection, width)];
    let rows = height.saturating_sub(1);
    if rows == 0 || width == 0 {
        return out;
    }

    // The wrapping field leaves the column flow: it takes the rows the other
    // fields do not, at the panel's full width, which is what "wraps across the
    // remaining rows" means (decision 5).
    let mut flow: Vec<&Field> = Vec::new();
    let mut wrapping: Option<&Field> = None;
    for field in &inspection.fields {
        if field.wrap && wrapping.is_none() && !field.value.is_empty() {
            wrapping = Some(field);
        } else {
            flow.push(field);
        }
    }
    // One row is always held back for it, so the field the panel exists for
    // survives even when the column flow would fill every row.
    let flow_rows = if wrapping.is_some() { rows - 1 } else { rows };

    let (columns, widths) = pack(&flow, flow_rows, width);
    let shown = &flow[..flow.len().min(flow_rows.saturating_mul(columns))];
    let used_rows = shown.len().div_ceil(columns);
    for row in 0..used_rows {
        out.push(flow_line(shown, row, columns, &widths));
    }
    if let Some(field) = wrapping {
        out.extend(wrap_lines(field, width, rows - used_rows));
    }
    out
}

/// The status glyph in its status colour, then the name in bold (decision 3).
fn title_line(inspection: &Inspection, width: usize) -> Line<'static> {
    let glyph_width = UnicodeWidthStr::width(inspection.glyph.content.as_ref());
    let name = truncate(&inspection.name, width.saturating_sub(glyph_width + 1));
    Line::from(vec![
        inspection.glyph.clone(),
        Span::raw(" "),
        Span::styled(name, Style::default().add_modifier(Modifier::BOLD)),
    ])
}

/// How much of its value a column must be able to say for opening another
/// column to be worth it. Below this the row is stubs and ellipses, which says
/// less than one fewer column would.
const MIN_VALUE_WIDTH: usize = 8;

/// How the fields pack into columns at this width: how many columns, and each
/// one's label and value width (decision 4).
///
/// Fields fill left to right and wrap to the next row, so column `j` holds
/// fields `j`, `j + columns`, `j + 2 * columns` and so on, and each column is
/// as wide as its own widest label and widest value. Columns are opened while
/// the width takes them, up to the number it takes to show every field: past
/// that a new column buys nothing and costs every other column the room it
/// took, which is how a window's `status` and `session` end up elided beside
/// three columns of blank rows.
///
/// A column that cannot have its full value is not a reason to close it: it is
/// elided instead (decision 5). Refusing any packing that needs an ellipsis
/// would let one long value — a session id, or the label of a deeply nested
/// parent — collapse the panel to a single column and push the fields below it
/// off the bottom, which is the opposite of what the panel is for. So a
/// packing is judged by whether every column can say `MIN_VALUE_WIDTH` of its
/// value (or all of it, when it is shorter), and the room left over is handed
/// out in fair shares, which gives the long values what the short ones do not
/// need.
fn pack(fields: &[&Field], rows: usize, width: usize) -> (usize, Vec<(usize, usize)>) {
    // One column renders even in a panel too narrow to have asked for it: the
    // label is cut to the panel and the value to whatever is left.
    let mut forced = natural_widths(&fields[..fields.len().min(rows)], 1);
    if let Some((label, value)) = forced.first_mut() {
        *label = (*label).min(width);
        *value = (*value).min(width.saturating_sub(*label + LABEL_GAP));
    }
    let mut best = (1, forced);
    if fields.is_empty() || rows == 0 {
        return best;
    }

    for columns in 1..=fields.len().div_ceil(rows) {
        let shown = &fields[..fields.len().min(rows.saturating_mul(columns))];
        let natural = natural_widths(shown, columns);
        let fixed: usize = natural
            .iter()
            .map(|(label, _)| label + LABEL_GAP)
            .sum::<usize>()
            + (columns - 1) * GUTTER;
        let floor: usize = natural
            .iter()
            .map(|(_, value)| (*value).min(MIN_VALUE_WIDTH))
            .sum();
        if fixed + floor > width {
            continue;
        }
        let wants: Vec<usize> = natural.iter().map(|(_, value)| *value).collect();
        let given = distribute(width - fixed, &wants);
        best = (
            columns,
            natural.iter().map(|(label, _)| *label).zip(given).collect(),
        );
    }
    best
}

/// Hands `budget` columns of value width out among `wants`, a fair share at a
/// time: no column takes more than it asked for, and what a short value leaves
/// behind goes to the long ones.
fn distribute(budget: usize, wants: &[usize]) -> Vec<usize> {
    let mut given = vec![0usize; wants.len()];
    let mut remaining = budget;
    loop {
        let needy: Vec<usize> = (0..wants.len())
            .filter(|index| given[*index] < wants[*index])
            .collect();
        if needy.is_empty() || remaining == 0 {
            return given;
        }
        let share = (remaining / needy.len()).max(1);
        let mut spent = 0;
        for index in needy {
            let take = (wants[index] - given[index])
                .min(share)
                .min(remaining - spent);
            given[index] += take;
            spent += take;
        }
        // The share is at least one and at least one column is needy, so this
        // only happens when the budget is exhausted — but it is what stops the
        // loop, so it is checked rather than assumed.
        if spent == 0 {
            return given;
        }
        remaining -= spent;
    }
}

/// Each column's widest label and widest value, in display columns
/// (decision 14).
fn natural_widths(fields: &[&Field], columns: usize) -> Vec<(usize, usize)> {
    let mut widths = vec![(0usize, 0usize); columns];
    for (index, field) in fields.iter().enumerate() {
        let (label, value) = &mut widths[index % columns];
        *label = (*label).max(UnicodeWidthStr::width(field.label));
        *value = (*value).max(UnicodeWidthStr::width(field.value.as_str()));
    }
    widths
}

/// One row of the column flow: each column's label, the gap, its value padded
/// to the column's width, and the gutter before the next.
fn flow_line(
    fields: &[&Field],
    row: usize,
    columns: usize,
    widths: &[(usize, usize)],
) -> Line<'static> {
    let mut spans = Vec::new();
    for (column, (label_width, value_width)) in widths.iter().copied().enumerate() {
        if column > 0 {
            spans.push(Span::raw(" ".repeat(GUTTER)));
        }
        let cell_width = label_width
            + if value_width > 0 {
                LABEL_GAP + value_width
            } else {
                0
            };
        match fields.get(row * columns + column) {
            Some(field) => {
                spans.push(Span::styled(
                    pad(&truncate(field.label, label_width), label_width),
                    theme::muted(),
                ));
                if value_width > 0 {
                    spans.push(Span::raw(" ".repeat(LABEL_GAP)));
                    spans.push(Span::raw(pad(
                        &truncate(&field.value, value_width),
                        value_width,
                    )));
                }
            }
            None => spans.push(Span::raw(" ".repeat(cell_width))),
        }
    }
    Line::from(spans)
}

/// The wrapping field across the rows the column flow left: its label on the
/// first line, its value wrapped to the panel's width under it.
fn wrap_lines(field: &Field, width: usize, rows: usize) -> Vec<Line<'static>> {
    let label_width = UnicodeWidthStr::width(field.label).min(width);
    let value_width = width.saturating_sub(label_width + LABEL_GAP);
    wrap_value(&field.value, value_width, rows)
        .into_iter()
        .enumerate()
        .map(|(index, chunk)| {
            let label = if index == 0 {
                Span::styled(
                    pad(&truncate(field.label, label_width), label_width),
                    theme::muted(),
                )
            } else {
                Span::raw(" ".repeat(label_width))
            };
            Line::from(vec![
                label,
                Span::raw(" ".repeat(LABEL_GAP)),
                Span::raw(chunk),
            ])
        })
        .collect()
}

/// Greedy word wrap to `width` display columns over at most `rows` lines. A
/// word longer than the width is broken; text past the last line is elided,
/// because the panel's height is fixed whatever the field would rather do.
fn wrap_value(text: &str, width: usize, rows: usize) -> Vec<String> {
    if width == 0 || rows == 0 {
        return Vec::new();
    }
    let mut lines: Vec<String> = Vec::new();
    let mut current = String::new();
    for word in text.split_whitespace() {
        let joined = UnicodeWidthStr::width(current.as_str()) + 1 + UnicodeWidthStr::width(word);
        if current.is_empty() {
            current.push_str(word);
        } else if joined <= width {
            current.push(' ');
            current.push_str(word);
        } else {
            lines.push(std::mem::take(&mut current));
            current.push_str(word);
        }
        while UnicodeWidthStr::width(current.as_str()) > width {
            let head = cut(&current, width);
            current = current[head.len()..].to_owned();
            lines.push(head);
        }
    }
    if !current.is_empty() {
        lines.push(current);
    }
    if lines.len() > rows {
        lines.truncate(rows);
        let last = lines.len() - 1;
        lines[last] = truncate(&format!("{}…", lines[last]), width);
    }
    lines
}

/// The longest prefix of `text` that fits `width` display columns, cut between
/// graphemes so a wide character is never split in half.
fn cut(text: &str, width: usize) -> String {
    let mut end = 0;
    for (index, grapheme) in text.grapheme_indices(true) {
        let candidate = index + grapheme.len();
        if UnicodeWidthStr::width(&text[..candidate]) > width {
            break;
        }
        end = candidate;
    }
    text[..end].to_owned()
}

/// `text` followed by enough spaces to fill `width` display columns.
fn pad(text: &str, width: usize) -> String {
    let used = UnicodeWidthStr::width(text);
    format!("{text}{}", " ".repeat(width.saturating_sub(used)))
}

#[cfg(test)]
mod tests;
