use crate::{
    app::App,
    theme,
    tree::{self, Row, RowKind, RuntimeCounts},
};
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthStr;

#[cfg(test)]
#[path = "tree_view_tests.rs"]
mod tests;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TreeGeometry {
    pub list: Rect,
    pub first: usize,
    pub count: usize,
}

pub fn geometry(list: Rect, rows_len: usize, top: usize) -> TreeGeometry {
    let height = usize::from(list.height);
    let first = top.min(rows_len.saturating_sub(height));
    TreeGeometry {
        list,
        first,
        count: height.min(rows_len - first),
    }
}

impl TreeGeometry {
    pub fn index_at(&self, column: u16, row: u16) -> Option<usize> {
        if !self.list.contains((column, row).into()) {
            return None;
        }
        let offset = usize::from(row - self.list.y);
        (offset < self.count).then_some(self.first + offset)
    }
}

pub fn narrow_line(
    app: &App,
    row: &Row<'_>,
    width: u16,
    pos_width: usize,
    selected: bool,
) -> Line<'static> {
    let bold = Style::default().add_modifier(Modifier::BOLD);
    let (prefix, name, rights) = match &row.kind {
        RowKind::Project {
            name,
            status,
            counts,
            collapsed,
            ..
        } => {
            let glyph = Span::styled(
                theme::status_glyph(*status, app.spinner_frame),
                Style::default().fg(theme::status_color(*status)),
            );
            (
                vec![Span::styled(
                    format!("{}{} ", row.guides, if *collapsed { "▸" } else { "▾" }),
                    bold,
                )],
                Span::styled(name.clone(), bold),
                vec![
                    vec![
                        glyph.clone(),
                        Span::styled(format!("  {}", counts_text(*counts)), theme::muted()),
                    ],
                    vec![glyph],
                ],
            )
        }
        RowKind::Window {
            info,
            position,
            has_subagents,
            collapsed,
        } => {
            let focused = app.focused == Some(info.id);
            let tag = tree::runtime_tag(info.runtime);
            let elapsed = tree::format_elapsed(app.elapsed_secs(info));
            let model = info
                .model
                .as_deref()
                .map(|model| tree::short_model(info.runtime, model));
            let mut rights = Vec::new();
            if let Some(model) = model {
                rights.push(vec![Span::styled(
                    format!("{tag} {model} {elapsed:>3}"),
                    theme::muted(),
                )]);
            }
            rights.push(vec![Span::styled(
                format!("{tag} {elapsed:>3}"),
                theme::muted(),
            )]);
            rights.push(vec![Span::styled(tag, theme::muted())]);
            (
                vec![
                    Span::raw(row.guides.clone()),
                    Span::styled(
                        if focused { "▎" } else { " " },
                        Style::default().fg(theme::ACCENT),
                    ),
                    Span::raw(if *collapsed && *has_subagents {
                        "▸"
                    } else {
                        " "
                    }),
                    Span::styled(
                        theme::status_glyph(info.status, app.spinner_frame),
                        Style::default().fg(theme::status_color(info.status)),
                    ),
                    Span::raw(format!(" {position:>pos_width$} ")),
                ],
                Span::styled(
                    info.name.clone(),
                    if focused { bold } else { Style::default() },
                ),
                rights,
            )
        }
        RowKind::Subagent { info, .. } => (
            vec![
                Span::raw(row.guides.clone()),
                Span::styled(
                    theme::subagent_glyph(info, app.spinner_frame),
                    Style::default().fg(theme::subagent_color(info)),
                ),
                Span::raw(" "),
            ],
            Span::raw(subagent_label(info)),
            vec![
                vec![Span::styled(
                    info.tool.clone().unwrap_or_default(),
                    theme::muted(),
                )],
                vec![],
            ],
        ),
    };
    fit_line(prefix, name, rights, usize::from(width), selected)
}

/// The text a sub-agent row shows in its name column: `kind: label` when a
/// label was set, `kind` alone otherwise.
fn subagent_label(info: &proto::SubagentInfo) -> String {
    match info.label.as_deref() {
        Some(label) => format!("{}: {label}", info.kind),
        None => info.kind.clone(),
    }
}

pub fn counts_text(counts: RuntimeCounts) -> String {
    [
        ("cl", counts.claude),
        ("cx", counts.codex),
        ("sh", counts.shell),
    ]
    .into_iter()
    .filter(|(_, count)| *count > 0)
    .map(|(tag, count)| format!("{tag} {count}"))
    .collect::<Vec<_>>()
    .join(" · ")
}

/// Column widths from the visible tree, after filtering and collapse.
///
/// `name` and `subagent_name` are the total width of guides *plus* the row's
/// own text (decision 27): a row's guides eat into its own budget, so the
/// text after the name column — the model, the status, the rest — starts at
/// the same offset whatever the row's depth.
pub struct WideColumns {
    name: usize,
    subagent_name: usize,
    model: usize,
    position: usize,
}

impl WideColumns {
    pub fn from_rows(rows: &[Row<'_>]) -> Self {
        let mut columns = Self {
            name: 0,
            subagent_name: 0,
            model: 1,
            position: 1,
        };
        for row in rows {
            let guide_width = UnicodeWidthStr::width(row.guides.as_str());
            match &row.kind {
                RowKind::Window { info, position, .. } => {
                    let name_width = UnicodeWidthStr::width(info.name.as_str()).min(24);
                    columns.name = columns.name.max(guide_width + name_width);
                    columns.model = columns
                        .model
                        .max(UnicodeWidthStr::width(info.model.as_deref().unwrap_or("-")).min(28));
                    columns.position = columns.position.max(position.to_string().len());
                }
                RowKind::Subagent { info, .. } => {
                    // Unlike the window name, this is never capped: sub-agent
                    // labels were shown in full before guides took a column,
                    // and nothing here bounds their depth (decision 25).
                    let name_width = UnicodeWidthStr::width(subagent_label(info).as_str());
                    columns.subagent_name = columns.subagent_name.max(guide_width + name_width);
                }
                RowKind::Project { .. } => {}
            }
        }
        columns
    }
}

pub fn wide_line(
    app: &App,
    row: &Row<'_>,
    width: u16,
    columns: &WideColumns,
    selected: bool,
) -> Line<'static> {
    let bold = Style::default().add_modifier(Modifier::BOLD);
    let spans = match &row.kind {
        RowKind::Project {
            root,
            name,
            status,
            counts,
            collapsed,
        } => {
            let root = super::terminal::shorten_home(root);
            let root = if root == "~/" { "~" } else { &root };
            return fit_line(
                vec![Span::styled(
                    format!("{}{} ", row.guides, if *collapsed { "▸" } else { "▾" }),
                    bold,
                )],
                Span::styled(format!("{name}  {root}"), bold),
                vec![vec![
                    Span::styled(
                        format!(
                            "{} {}",
                            theme::status_glyph(*status, app.spinner_frame),
                            status.label()
                        ),
                        Style::default().fg(theme::status_color(*status)),
                    ),
                    Span::styled(format!("  {}", counts_text(*counts)), theme::muted()),
                ]],
                usize::from(width),
                selected,
            );
        }
        RowKind::Window {
            info,
            position,
            has_subagents,
            collapsed,
        } => {
            let focused = app.focused == Some(info.id);
            let position_width = columns.position;
            let guide_width = UnicodeWidthStr::width(row.guides.as_str());
            let name = padded(&info.name, columns.name.saturating_sub(guide_width));
            let model = padded(info.model.as_deref().unwrap_or("-"), columns.model);
            let elapsed = tree::format_elapsed(app.elapsed_secs(info));
            vec![
                Span::raw(row.guides.clone()),
                Span::styled(
                    if focused { "▎" } else { " " },
                    Style::default().fg(theme::ACCENT),
                ),
                Span::raw(if *collapsed && *has_subagents {
                    "▸"
                } else {
                    " "
                }),
                Span::styled(
                    theme::status_glyph(info.status, app.spinner_frame),
                    Style::default().fg(theme::status_color(info.status)),
                ),
                Span::raw(format!(" {position:>position_width$} ")),
                Span::styled(name, if focused { bold } else { Style::default() }),
                Span::styled(
                    format!(
                        "  {:<6}  {model}  {:<9}  {elapsed:>4}  {}",
                        info.runtime.label(),
                        info.status.label(),
                        info.tool.as_deref().unwrap_or("")
                    ),
                    theme::muted(),
                ),
            ]
        }
        RowKind::Subagent { info, .. } => {
            let guide_width = UnicodeWidthStr::width(row.guides.as_str());
            let label = padded(
                &subagent_label(info),
                columns.subagent_name.saturating_sub(guide_width),
            );
            let (state, duration) = match info.state {
                proto::SubagentState::Running => ("running", app.age_secs(info.started_secs)),
                proto::SubagentState::Done => (
                    "done",
                    info.ended_secs
                        .map_or(0, |ended| info.started_secs.saturating_sub(ended)),
                ),
                proto::SubagentState::Failed => (
                    "failed",
                    info.ended_secs
                        .map_or(0, |ended| info.started_secs.saturating_sub(ended)),
                ),
            };
            vec![
                Span::raw(row.guides.clone()),
                Span::styled(
                    theme::subagent_glyph(info, app.spinner_frame),
                    Style::default().fg(theme::subagent_color(info)),
                ),
                Span::raw(format!(" {label}")),
                Span::styled(
                    format!(
                        "  {}  {state}  {}  {}",
                        info.model.as_deref().unwrap_or("-"),
                        tree::format_elapsed(duration),
                        info.tool.as_deref().unwrap_or("")
                    ),
                    theme::muted(),
                ),
            ]
        }
    };
    fit_spans(spans, usize::from(width), selected)
}

fn padded(text: &str, width: usize) -> String {
    let text = truncate(text, width);
    let padding = width.saturating_sub(UnicodeWidthStr::width(text.as_str()));
    format!("{text}{}", " ".repeat(padding))
}

fn fit_spans(spans: Vec<Span<'static>>, width: usize, selected: bool) -> Line<'static> {
    let mut remaining = width;
    let mut fitted = Vec::new();
    for mut span in spans {
        span.content = truncate(&span.content, remaining).into();
        remaining = remaining.saturating_sub(UnicodeWidthStr::width(span.content.as_ref()));
        fitted.push(span);
    }
    fitted.push(Span::raw(" ".repeat(remaining)));
    let line = Line::from(fitted);
    if selected {
        line.style(Style::default().add_modifier(Modifier::REVERSED))
    } else {
        line
    }
}

fn spans_width(spans: &[Span<'_>]) -> usize {
    spans
        .iter()
        .map(|span| UnicodeWidthStr::width(span.content.as_ref()))
        .sum()
}

/// Preserve right-hand fields while leaving room for the row prefix and at least an ellipsis.
fn fit_line(
    mut prefix: Vec<Span<'static>>,
    mut name: Span<'static>,
    rights: Vec<Vec<Span<'static>>>,
    width: usize,
    selected: bool,
) -> Line<'static> {
    let prefix_width = spans_width(&prefix);
    let right = rights
        .into_iter()
        .find(|right| {
            let right_width = spans_width(right);
            prefix_width
                + usize::from(!name.content.is_empty())
                + right_width
                + usize::from(right_width > 0)
                <= width
        })
        .unwrap_or_default();
    let right_width = spans_width(&right);
    let name_width =
        width.saturating_sub(prefix_width + right_width + usize::from(right_width > 0));
    name.content = truncate(&name.content, name_width).into();
    prefix.push(name);
    // Extremely narrow terminals and deep nesting can truncate even the guides.
    let mut remaining = width.saturating_sub(right_width);
    for span in &mut prefix {
        span.content = truncate(&span.content, remaining).into();
        remaining = remaining.saturating_sub(UnicodeWidthStr::width(span.content.as_ref()));
    }
    prefix.push(Span::raw(" ".repeat(remaining)));
    prefix.extend(right);
    let line = Line::from(prefix);
    if selected {
        line.style(Style::default().add_modifier(Modifier::REVERSED))
    } else {
        line
    }
}

fn truncate(text: &str, width: usize) -> String {
    if UnicodeWidthStr::width(text) <= width {
        return text.to_owned();
    }
    if width == 0 {
        return String::new();
    }
    let mut result = String::new();
    for grapheme in text.graphemes(true) {
        if UnicodeWidthStr::width(result.as_str()) + UnicodeWidthStr::width(grapheme) >= width {
            break;
        }
        result.push_str(grapheme);
    }
    result.push('…');
    result
}
