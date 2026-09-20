use crate::app::{App, TreeInput};
use crate::theme;
use proto::{GitOperation, GitState, Head};
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use unicode_width::UnicodeWidthStr;

const HINTS: [(&str, &str); 5] = [
    ("C-b ?", "help"),
    ("C-b c", "new shell"),
    ("C-b t", "tree"),
    ("C-b j/k", "switch"),
    ("C-b d", "detach"),
];

pub fn render(frame: &mut Frame, app: &App, area: Rect) {
    let mut spans = Vec::new();
    if app.keymap.pending() {
        spans.push(Span::styled(
            " PREFIX ",
            Style::default()
                .fg(Color::Black)
                .bg(theme::ACCENT)
                .add_modifier(Modifier::BOLD),
        ));
        spans.push(Span::raw(" "));
    } else if let Some(input) = app.tree_input {
        spans.push(Span::styled(
            match input {
                TreeInput::Navigate => " TREE ",
                TreeInput::Filter => " FILTER ",
            },
            Style::default()
                .fg(Color::Black)
                .bg(theme::ACCENT)
                .add_modifier(Modifier::BOLD),
        ));
        spans.push(Span::raw(" "));
    } else if !app.connected {
        spans.push(Span::styled(
            " DISCONNECTED ",
            Style::default()
                .fg(Color::Black)
                .bg(Color::Red)
                .add_modifier(Modifier::BOLD),
        ));
        spans.push(Span::raw(" "));
    } else {
        spans.push(Span::raw(" "));
    }

    let toast_width = app
        .toast_text()
        .map(|text| (UnicodeWidthStr::width(text) as u16 + 1).min(area.width))
        .unwrap_or(0);

    match app.tree_input {
        Some(TreeInput::Navigate) => spans.push(Span::styled(
            "j/k move  ⏎ focus  space fold  / filter  esc back",
            theme::muted(),
        )),
        Some(TreeInput::Filter) => spans.push(Span::styled(
            format!("/{}", app.tree.filter),
            theme::muted(),
        )),
        None => match app.focused_git() {
            None => push_hints(&mut spans, HINTS.len()),
            Some(state) => {
                let badge_width = spans_width(&spans);
                let available = area
                    .width
                    .saturating_sub(badge_width)
                    .saturating_sub(toast_width);
                // Hints drop from the right, one at a time, before the git segment gives up
                // any of its own parts (decision 20).
                let full_git_width = spans_width(&git_spans(state, usize::MAX));
                let mut hint_count = HINTS.len();
                while hint_count > 0 && hints_width(hint_count) + full_git_width > available {
                    hint_count -= 1;
                }
                push_hints(&mut spans, hint_count);
                let git_budget = available.saturating_sub(hints_width(hint_count));
                spans.extend(git_spans(state, usize::from(git_budget)));
            }
        },
    }
    frame.render_widget(Paragraph::new(Line::from(spans)), area);

    if let Some(text) = app.toast_text() {
        let width = (UnicodeWidthStr::width(text) as u16 + 1).min(area.width);
        let right = Rect {
            x: area.x + area.width - width,
            width,
            ..area
        };
        let toast = Span::styled(
            text.to_string(),
            Style::default()
                .fg(theme::ACCENT)
                .add_modifier(Modifier::BOLD),
        );
        frame.render_widget(Paragraph::new(Line::from(toast)), right);
    }
}

/// Pushes the first `count` key hints (of `HINTS`'s 5) onto `spans`, in the styling shared by
/// the git-present and git-absent render paths.
fn push_hints(spans: &mut Vec<Span<'static>>, count: usize) {
    for (key, what) in HINTS.into_iter().take(count) {
        spans.push(Span::styled(key, Style::default().fg(theme::ACCENT)));
        spans.push(Span::styled(format!(" {what}  "), theme::muted()));
    }
}

fn spans_width(spans: &[Span<'_>]) -> u16 {
    spans
        .iter()
        .map(|s| UnicodeWidthStr::width(s.content.as_ref()) as u16)
        .sum()
}

fn hints_width(count: usize) -> u16 {
    HINTS
        .iter()
        .take(count)
        .map(|(key, what)| {
            UnicodeWidthStr::width(*key) as u16 + UnicodeWidthStr::width(*what) as u16 + 3
        })
        .sum()
}

/// One droppable part of the git segment, right of the head. `priority` is the order parts
/// are dropped in as the budget shrinks: the lowest priority goes first.
struct Part {
    text: String,
    style: Style,
    priority: u8,
}

fn head_text(head: &Head) -> String {
    match head {
        Head::Branch(name) | Head::Unborn(name) => name.clone(),
        Head::Detached(oid) => format!("@{oid}"),
    }
}

fn operation_name(op: GitOperation) -> &'static str {
    match op {
        GitOperation::Merge => "merge",
        GitOperation::Rebase => "rebase",
        GitOperation::CherryPick => "cherry-pick",
        GitOperation::Revert => "revert",
        GitOperation::Bisect => "bisect",
    }
}

/// Builds the full (untruncated) list of parts after the head, in decision 19's colours.
/// Priorities implement decision 20's drop order (operation, untracked, divergence, dirty)
/// exactly; conflicts, the clean tick and `(stale)` are not named by that decision, so they
/// are ranked around it — kept longer than dirty, since an active conflict or a good status
/// is worth more than the counts feeding it.
///
/// Controller ruling (not in decision 20): `(stale)` shares conflicts' priority rather than
/// being the first thing dropped. It is not another datum competing with dirty/untracked/
/// divergence for space — it is a trust flag on all of them. Dropping it first would let a
/// narrow terminal show a confident `main ●3 ?1 ⇡2⇣1` with nothing marking the read as
/// possibly stale, which is worse than showing `main (stale)` with the counts gone.
fn build_parts(state: &GitState) -> Vec<Part> {
    let mut parts = Vec::new();
    if state.conflicts > 0 {
        parts.push(Part {
            text: format!("⚠{}", state.conflicts),
            style: Style::default().fg(Color::Red),
            priority: 5,
        });
    }
    if state.dirty > 0 {
        parts.push(Part {
            text: format!("●{}", state.dirty),
            style: theme::muted(),
            priority: 4,
        });
    }
    if state.untracked > 0 {
        parts.push(Part {
            text: format!("?{}", state.untracked),
            style: theme::muted(),
            priority: 2,
        });
    }
    if state.ahead > 0 || state.behind > 0 {
        let mut text = String::new();
        if state.ahead > 0 {
            text.push_str(&format!("⇡{}", state.ahead));
        }
        if state.behind > 0 {
            text.push_str(&format!("⇣{}", state.behind));
        }
        parts.push(Part {
            text,
            style: theme::muted(),
            priority: 3,
        });
    }
    if let Some(op) = state.operation {
        parts.push(Part {
            text: operation_name(op).to_string(),
            style: Style::default().fg(Color::Red),
            priority: 1,
        });
    }
    if parts.is_empty() {
        parts.push(Part {
            text: "✓".to_string(),
            style: Style::default().fg(Color::Green),
            priority: 6,
        });
    }
    if state.stale {
        parts.push(Part {
            text: "(stale)".to_string(),
            style: theme::muted(),
            priority: 5, // matches conflicts — see the controller ruling above.
        });
    }
    parts
}

/// The spans for a worktree's git state that fit in `budget` columns. Parts drop right to
/// left per decision 20 as the budget shrinks; below the head's own width, nothing is
/// returned at all.
pub fn git_spans(state: &GitState, budget: usize) -> Vec<Span<'static>> {
    let head = head_text(&state.head);
    let head_width = UnicodeWidthStr::width(head.as_str());
    if head_width > budget {
        return vec![];
    }
    let head_span = Span::styled(head, Style::default().add_modifier(Modifier::BOLD));

    if let Head::Unborn(_) = state.head {
        let suffix = " (unborn)";
        return if head_width + UnicodeWidthStr::width(suffix) <= budget {
            vec![head_span, Span::styled(suffix, theme::muted())]
        } else {
            vec![head_span]
        };
    }

    let mut parts = build_parts(state);
    loop {
        let used: usize = parts
            .iter()
            .map(|p| 1 + UnicodeWidthStr::width(p.text.as_str()))
            .sum();
        if head_width + used <= budget {
            break;
        }
        let Some(drop_at) = parts
            .iter()
            .enumerate()
            .min_by_key(|(_, p)| p.priority)
            .map(|(i, _)| i)
        else {
            break;
        };
        parts.remove(drop_at);
    }

    let mut spans = vec![head_span];
    for part in parts {
        spans.push(Span::styled(format!(" {}", part.text), part.style));
    }
    spans
}

#[cfg(test)]
#[path = "statusbar_tests.rs"]
mod tests;
