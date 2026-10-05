use crate::{
    app::App,
    theme::{self, Glyph, Role, glyph},
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
    let p = app.palette();
    let (frame, ascii) = (app.spinner_frame, p.ascii);
    let muted = theme::role(Role::Muted, p);
    let look = |(glyph, role): (&'static str, Role)| Span::styled(glyph, theme::role(role, p));
    let guides = theme::guides(&row.guides, ascii);
    let focus_mark = |focused: bool| {
        Span::styled(
            if focused {
                glyph(Glyph::Focus, ascii)
            } else {
                " "
            },
            theme::role(Role::Accent, p),
        )
    };
    let (prefix, name, branch, rights) = match &row.kind {
        RowKind::Project {
            name,
            status,
            counts,
            collapsed,
            ..
        } => {
            let mark = look(theme::status_look(*status, frame, ascii));
            let fold = if *collapsed {
                Glyph::Collapsed
            } else {
                Glyph::Expanded
            };
            let counts = theme::fold(&counts_text(*counts), ascii);
            (
                vec![Span::styled(
                    format!("{guides}{} ", glyph(fold, ascii)),
                    bold,
                )],
                Name::Text(Span::styled(name.clone(), bold)),
                None,
                vec![
                    vec![mark.clone(), Span::styled(format!("  {counts}"), muted)],
                    vec![mark],
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
                    muted,
                )]);
            }
            rights.push(vec![Span::styled(format!("{tag} {elapsed:>3}"), muted)]);
            rights.push(vec![Span::styled(tag, muted)]);
            (
                vec![
                    Span::raw(guides),
                    focus_mark(focused),
                    Span::raw(if *collapsed && *has_subagents {
                        glyph(Glyph::Collapsed, ascii)
                    } else {
                        " "
                    }),
                    look(theme::status_look(info.status, frame, ascii)),
                    Span::raw(format!(" {position:>pos_width$} ")),
                ],
                Name::Text(Span::styled(
                    info.name.clone(),
                    if focused { bold } else { Style::default() },
                )),
                branch_text(info, app),
                rights,
            )
        }
        RowKind::Subagent { info, .. } => (
            vec![
                Span::raw(guides),
                look(theme::subagent_look(info, frame, ascii)),
                Span::raw(" "),
            ],
            Name::Text(Span::raw(tree::subagent_label(info))),
            None,
            vec![
                vec![Span::styled(info.tool.clone().unwrap_or_default(), muted)],
                vec![],
            ],
        ),
        RowKind::Run {
            run,
            orchestrator,
            position,
        } => {
            let focused = orchestrator.is_some_and(|window| app.focused == Some(window.id));
            let (merged, total) = tree::run_progress(run);
            let position = position.map_or_else(String::new, |position| position.to_string());
            // Milestone 9.0.7 review ruling: a tick only for progress the data proves.
            let tick = if merged > 0 { Role::Done } else { Role::Muted };
            (
                vec![
                    Span::raw(guides),
                    focus_mark(focused),
                    Span::raw(" "),
                    look(theme::run_look(run.state, ascii)),
                    Span::raw(format!(" {position:>pos_width$} ")),
                ],
                // Decision 28: the run's one name; a blank goal names the run by its
                // id, as `tree::run_title` does.
                Name::Run {
                    goal: tree::run_title(run).to_owned(),
                    id: run.run_id.clone(),
                    style: if focused { bold } else { Style::default() },
                },
                None,
                vec![
                    vec![
                        Span::styled(format!("{merged}/{total} "), muted),
                        Span::styled(glyph(Glyph::Passed, ascii), theme::role(tick, p)),
                    ],
                    vec![],
                ],
            )
        }
        // Milestone 9.3 decision 32: `◌ orchestrator · idle · after <h4>`, muted whole,
        // in its window's place; `idle_text` cleans the run id, folded here.
        // No right-hand text: the row's own words take the room.
        RowKind::IdleOrchestrator { idle, window } => {
            let focused = app.focused == Some(window.id);
            let text = theme::fold(&tree::idle_text(idle), ascii);
            (
                vec![
                    Span::raw(guides),
                    focus_mark(focused),
                    Span::styled(format!(" {} ", glyph(Glyph::NotStarted, ascii)), muted),
                ],
                Name::Text(Span::styled(text, muted)),
                None,
                vec![vec![]],
            )
        }
        // The run view's rows (task M8c.4) are drawn only on its canvas, never here.
        RowKind::Planner { .. }
        | RowKind::Scout { .. }
        | RowKind::DesignAgent { .. }
        | RowKind::Task { .. }
        | RowKind::Stage { .. }
        | RowKind::Round { .. }
        | RowKind::AgentRound { .. } => (
            vec![Span::raw(guides)],
            Name::Text(Span::raw(crate::graph::content_text_in(row, ascii))),
            None,
            vec![vec![]],
        ),
    };
    let fit = Fit {
        width: usize::from(width),
        selected,
        muted,
        p,
    };
    fit_line(prefix, name, branch, rights, fit)
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
/// How `fit_line` fits a row: its width, whether it is selected, the branch marker's
/// muted style, and the palette (whose `ascii` picks the cut marks).
struct Fit {
    width: usize,
    selected: bool,
    muted: Style,
    p: theme::Palette,
}

/// A row's name: drawn as given, or a run's goal and id, named once by
/// `kit::run_name_in` in the room `fit_line` leaves it (decision 28), so the short id is
/// never the part cut.
enum Name {
    Text(Span<'static>),
    Run {
        goal: String,
        id: String,
        style: Style,
    },
}

fn fit_line(
    mut prefix: Vec<Span<'static>>,
    name: Name,
    branch: Option<String>,
    rights: Vec<Vec<Span<'static>>>,
    fit: Fit,
) -> Line<'static> {
    let Fit {
        width,
        selected,
        muted,
        p,
    } = fit;
    let ascii = p.ascii;
    let named = match &name {
        Name::Text(span) => !span.content.is_empty(),
        Name::Run { .. } => true,
    };
    let prefix_width = spans_width(&prefix);
    let right = rights
        .into_iter()
        .find(|right| {
            let right_width = spans_width(right);
            prefix_width + usize::from(named) + right_width + usize::from(right_width > 0) <= width
        })
        .unwrap_or_default();
    let right_width = spans_width(&right);
    let available = width.saturating_sub(prefix_width + right_width + usize::from(right_width > 0));
    let mut name = match name {
        Name::Text(span) => span,
        Name::Run { goal, id, style } => {
            let room = u16::try_from(available).unwrap_or(u16::MAX);
            Span::styled(crate::ui::kit::run_name_in(&goal, &id, room, p), style)
        }
    };

    let name_full = UnicodeWidthStr::width(name.content.as_ref());
    let name_floor = available.min(name_full).min(NAME_FLOOR);
    let branch_span = branch.as_deref().and_then(|branch| {
        let budget = available.saturating_sub(name_floor);
        (budget >= 4).then(|| {
            let text = truncate_in(branch, budget - 3, ascii);
            Span::styled(format!(" [{text}]"), muted)
        })
    });
    let branch_width = branch_span
        .as_ref()
        .map(|span| UnicodeWidthStr::width(span.content.as_ref()))
        .unwrap_or(0);

    let name_width = available.saturating_sub(branch_width);
    name.content = truncate_in(&name.content, name_width, ascii).into();
    prefix.push(name);
    if let Some(branch_span) = branch_span {
        prefix.push(branch_span);
    }
    // Extremely narrow terminals and deep nesting can truncate even the guides.
    let mut remaining = width.saturating_sub(right_width);
    for span in &mut prefix {
        span.content = truncate_in(&span.content, remaining, ascii).into();
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
    fit(text, width, Some("…"))
}

/// [`truncate`], with the ASCII mark `...` when `ascii` (milestone 9.0.7 decision 5),
/// or `.` below four columns.
pub(crate) fn truncate_in(text: &str, width: usize, ascii: bool) -> String {
    match (ascii, width >= 4) {
        (false, _) => truncate(text, width),
        (true, true) => fit(text, width, Some("...")),
        (true, false) => fit(text, width, Some(".")),
    }
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
    fit(text, width, None)
}

fn fit(text: &str, width: usize, ellipsis: Option<&str>) -> String {
    if UnicodeWidthStr::width(text) <= width {
        return text.to_owned();
    }
    if width == 0 {
        return String::new();
    }
    // The ellipsis needs columns of its own; a cut with no mark keeps them all.
    let mark = ellipsis.map_or(0, UnicodeWidthStr::width).min(width);
    let budget = width - mark;
    let mut result = String::new();
    for grapheme in text.graphemes(true) {
        if UnicodeWidthStr::width(result.as_str()) + UnicodeWidthStr::width(grapheme) > budget {
            break;
        }
        result.push_str(grapheme);
    }
    if let Some(mark) = ellipsis {
        result.push_str(mark);
    } else if result.is_empty() {
        // A first grapheme wider than the whole width: take it anyway, so the
        // caller advances. Overflowing a column by one cell is clipped; not
        // advancing never ends.
        result.extend(text.graphemes(true).next());
    }
    result
}
