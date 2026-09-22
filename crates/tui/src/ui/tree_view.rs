use crate::{
    app::App,
    theme,
    tree::{self, Row, RowKind, RuntimeCounts},
};
use proto::WindowInfo;
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

/// Decision 37's one derivation: the head `App.git` holds for the window's worktree
/// root, and `WindowInfo.branch` when it holds none. `None` for a window that is not in
/// a worktree this daemon made — `WindowInfo.branch` is what records that, exactly as
/// decision 35's remove-confirm checkbox already reads it, so this is the only other
/// place `WindowInfo.branch` is read directly.
pub fn branch_text(window: &WindowInfo, app: &App) -> Option<String> {
    let fallback = window.branch.as_ref()?;
    let live = window
        .worktree
        .as_ref()
        .and_then(|root| app.git.get(root))
        .map(|state| crate::ui::statusbar::head_text(&state.head));
    Some(live.unwrap_or_else(|| fallback.clone()))
}

pub fn narrow_line(
    app: &App,
    row: &Row<'_>,
    width: u16,
    pos_width: usize,
    selected: bool,
) -> Line<'static> {
    let bold = Style::default().add_modifier(Modifier::BOLD);
    let (prefix, name, branch, rights) = match &row.kind {
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
                None,
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
                        Style::default().fg(app.settings.accent),
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
                branch_text(info, app),
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
            Span::raw(tree::subagent_label(info)),
            None,
            vec![
                vec![Span::styled(
                    info.tool.clone().unwrap_or_default(),
                    theme::muted(),
                )],
                vec![],
            ],
        ),
    };
    fit_line(prefix, name, branch, rights, usize::from(width), selected)
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

/// Minimum columns decision 37 leaves the name once a worktree branch marker is
/// competing for the same space; the branch is what shrinks past this point.
const NAME_FLOOR: usize = 8;

/// Preserve right-hand fields while leaving room for the row prefix and at least an
/// ellipsis. `branch` is decision 37's sidebar marker, `Some` only for a window row in a
/// worktree this daemon made: it sacrifices before the name does, shrinking with an
/// ellipsis and finally dropped once fewer than 4 columns remain for it (`[` + at least
/// one character + `…` + `]`), while the name keeps only `NAME_FLOOR` columns for itself
/// until the branch is gone, rather than taking everything it wants first.
fn fit_line(
    mut prefix: Vec<Span<'static>>,
    mut name: Span<'static>,
    branch: Option<String>,
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
    let available = width.saturating_sub(prefix_width + right_width + usize::from(right_width > 0));

    let name_full = UnicodeWidthStr::width(name.content.as_ref());
    let name_floor = available.min(name_full).min(NAME_FLOOR);
    let branch_span = branch.as_deref().and_then(|branch| {
        let budget = available.saturating_sub(name_floor);
        (budget >= 4).then(|| {
            let text = truncate(branch, budget - 3);
            Span::styled(format!(" [{text}]"), theme::muted())
        })
    });
    let branch_width = branch_span
        .as_ref()
        .map(|span| UnicodeWidthStr::width(span.content.as_ref()))
        .unwrap_or(0);

    let name_width = available.saturating_sub(branch_width);
    name.content = truncate(&name.content, name_width).into();
    prefix.push(name);
    if let Some(branch_span) = branch_span {
        prefix.push(branch_span);
    }
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

/// Cuts `text` to `width` display columns, grapheme-safe, appending `…` when
/// it does not fit. Shared with the graph painter, which truncates box
/// content the same way (decision 11).
pub(crate) fn truncate(text: &str, width: usize) -> String {
    fit(text, width, true)
}

/// `truncate` without the ellipsis: the longest prefix of `text` that fits
/// `width` display columns. Used where the text continues somewhere else — the
/// inspector wraps a field onto the next row rather than ending it — so a mark
/// saying it was cut would be a lie.
///
/// Above a width of zero this always takes at least one grapheme, even one too
/// wide to fit. A caller walking a string by repeated cuts has to make
/// progress; returning nothing leaves it exactly where it was, which is an
/// infinite loop rather than a narrow column.
pub(crate) fn cut(text: &str, width: usize) -> String {
    fit(text, width, false)
}

fn fit(text: &str, width: usize, ellipsis: bool) -> String {
    if UnicodeWidthStr::width(text) <= width {
        return text.to_owned();
    }
    if width == 0 {
        return String::new();
    }
    // The ellipsis needs a column of its own; a cut with no mark keeps them all.
    let budget = if ellipsis { width - 1 } else { width };
    let mut result = String::new();
    for grapheme in text.graphemes(true) {
        if UnicodeWidthStr::width(result.as_str()) + UnicodeWidthStr::width(grapheme) > budget {
            break;
        }
        result.push_str(grapheme);
    }
    if ellipsis {
        result.push('…');
    } else if result.is_empty() {
        // A first grapheme wider than the whole width: take it anyway, so the
        // caller advances. Overflowing a column by one cell is clipped; not
        // advancing never ends.
        result.extend(text.graphemes(true).next());
    }
    result
}
