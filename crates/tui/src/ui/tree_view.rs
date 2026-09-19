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
                    format!(
                        "{}{} ",
                        " ".repeat(row.indent.into()),
                        if *collapsed { "▸" } else { "▾" }
                    ),
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
                    Span::raw(" ".repeat(row.indent.saturating_sub(2).into())),
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
        RowKind::Subagent { info, guides, .. } => {
            let label = match info.label.as_deref() {
                Some(label) => format!("{}: {label}", info.kind),
                None => info.kind.clone(),
            };
            (
                vec![
                    Span::raw(format!("{}{guides}", " ".repeat(row.indent.into()))),
                    Span::styled(
                        theme::subagent_glyph(info, app.spinner_frame),
                        Style::default().fg(theme::subagent_color(info)),
                    ),
                    Span::raw(" "),
                ],
                Span::raw(label),
                vec![
                    vec![Span::styled(
                        info.tool.clone().unwrap_or_default(),
                        theme::muted(),
                    )],
                    vec![],
                ],
            )
        }
    };
    fit_line(prefix, name, rights, usize::from(width), selected)
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
