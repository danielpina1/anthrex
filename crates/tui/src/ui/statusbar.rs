use crate::app::{App, Link, TreeInput};
use crate::theme;
use proto::{GitOperation, GitState, Head};
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use unicode_width::UnicodeWidthStr;

/// The status bar's key hints, keyed to `prefix_label` (decision 38: every hint takes
/// the prefix from `app.settings.prefix_label`, never a hard-coded `C-b`).
fn hints(prefix_label: &str) -> [(String, &'static str); 5] {
    [
        (format!("{prefix_label} ?"), "help"),
        (format!("{prefix_label} c"), "new shell"),
        (format!("{prefix_label} t"), "tree"),
        (format!("{prefix_label} j/k"), "switch"),
        (format!("{prefix_label} d"), "detach"),
    ]
}

/// The red ` DISCONNECTED ` badge, common to both `Link::Reconnecting` and
/// `Link::Lost` (decision 34); each pushes its own status text right after it.
fn push_disconnected(spans: &mut Vec<Span<'static>>) {
    spans.push(Span::styled(
        " DISCONNECTED ",
        Style::default()
            .fg(Color::Black)
            .bg(Color::Red)
            .add_modifier(Modifier::BOLD),
    ));
    spans.push(Span::raw(" "));
}

pub fn render(frame: &mut Frame, app: &App, area: Rect) {
    let accent = app.settings.accent;
    let hints = hints(&app.settings.prefix_label);
    let mut spans = Vec::new();
    if app.keymap.pending() {
        spans.push(Span::styled(
            " PREFIX ",
            Style::default()
                .fg(Color::Black)
                .bg(accent)
                .add_modifier(Modifier::BOLD),
        ));
        spans.push(Span::raw(" "));
    } else if app.plan_review.is_some() {
        // Milestone 9.0.5 decision 13: the review is over whatever tree mode is on.
        spans.push(Span::styled(
            " REVIEW ",
            Style::default()
                .fg(Color::Black)
                .bg(accent)
                .add_modifier(Modifier::BOLD),
        ));
        spans.push(Span::raw(" "));
    } else if app.alerts_focus.is_some() {
        // Milestone 9.0.5 decision 21: the Alerts box has the keys.
        spans.push(Span::styled(
            " ALERTS ",
            Style::default()
                .fg(Color::Black)
                .bg(accent)
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
                .bg(accent)
                .add_modifier(Modifier::BOLD),
        ));
        spans.push(Span::raw(" "));
    } else {
        match &app.link {
            Link::Connected => spans.push(Span::raw(" ")),
            // Decision 34: the persistent status while a retry is scheduled or in
            // flight.
            Link::Reconnecting { attempts, .. } => {
                push_disconnected(&mut spans);
                spans.push(Span::styled(
                    format!("reconnecting (attempt {attempts})"),
                    theme::muted(),
                ));
                spans.push(Span::raw(" "));
            }
            // Decision 34: the 30 s window elapsed; only a fresh `C-b r` tries again.
            Link::Lost { .. } => {
                push_disconnected(&mut spans);
                spans.push(Span::styled(
                    format!("{} r to reconnect", app.settings.prefix_label),
                    theme::muted(),
                ));
                spans.push(Span::raw(" "));
            }
        }
    }

    // Decision 20: with the sidebar hidden, the alerts' count in the top priority's
    // colour, right after the mode badge.
    if !app.sidebar_visible {
        let all = crate::app::alerts(app);
        if let Some(top) = all.first() {
            spans.push(Span::styled(
                format!("⚑ {}", all.len()),
                Style::default()
                    .fg(theme::alert_color(top.priority))
                    .add_modifier(Modifier::BOLD),
            ));
            spans.push(Span::raw(" "));
        }
    }

    let toast_width = app
        .toast_text()
        .map(|text| toast_columns(text, area.width))
        .unwrap_or(0);

    match app.tree_input {
        _ if app.plan_review.is_some() => {
            spans.push(Span::styled(review_hint(app), theme::muted()));
        }
        _ if app.alerts_focus.is_some() => {
            spans.push(Span::styled("j/k move  ⏎ go  esc back", theme::muted()));
        }
        Some(TreeInput::Navigate) => spans.push(Span::styled(navigate_hint(app), theme::muted())),
        Some(TreeInput::Filter) => spans.push(Span::styled(
            format!("/{}", app.tree.filter),
            theme::muted(),
        )),
        None => match app.focused_git() {
            None => push_hints(&mut spans, &hints, hints.len(), accent),
            Some(state) => {
                let badge_width = spans_width(&spans);
                let available = area
                    .width
                    .saturating_sub(badge_width)
                    .saturating_sub(toast_width);
                // Hints drop from the right, one at a time, before the git segment gives up
                // any of its own parts (decision 20).
                let full_git_width = spans_width(&git_spans(state, usize::MAX));
                let mut hint_count = hints.len();
                while hint_count > 0 && hints_width(&hints, hint_count) + full_git_width > available
                {
                    hint_count -= 1;
                }
                push_hints(&mut spans, &hints, hint_count, accent);
                let git_budget = available.saturating_sub(hints_width(&hints, hint_count));
                spans.extend(git_spans(state, usize::from(git_budget)));
            }
        },
    }
    frame.render_widget(Paragraph::new(Line::from(spans)), area);

    if let Some(text) = app.toast_text() {
        let width = toast_columns(text, area.width);
        let right = Rect {
            x: area.x + area.width - width,
            width,
            ..area
        };
        let toast = Span::styled(
            text.to_string(),
            Style::default().fg(accent).add_modifier(Modifier::BOLD),
        );
        frame.render_widget(Paragraph::new(Line::from(toast)), right);
    }
}

/// Tree navigation's hint: the project tree's, or the run view's two (milestone 8c,
/// Interfaces "Status bar and overview title"), the plan gate's while the run awaits
/// approval.
fn navigate_hint(app: &App) -> String {
    let Some(view) = &app.run_view else {
        return "j/k move  ⏎ focus  space fold  / filter  esc back".to_string();
    };
    let label = crate::app::filter_label(view.filter);
    let run = app.runs.runs.iter().find(|run| run.run_id == view.run_id);
    let state = run.map(|run| run.state);
    // Milestone 9: a planning run's submit, and a run's awaiting holds.
    let holds = run.is_some_and(|run| crate::tree::awaiting_holds(run).next().is_some());
    if state == Some(proto::RunState::AwaitingApproval) {
        format!(
            "a approve  x reject  e edit  d remove  p review  ⏎ open  f filter: {label}  esc back"
        )
    } else if state == Some(proto::RunState::Planning) {
        format!("s submit  j/k move  ⏎ open  f filter: {label}  esc back")
    } else if holds {
        format!("a approve hold  x reject hold  p review  ⏎ open  f filter: {label}  esc back")
    } else {
        format!("j/k move  h/l tier  ⏎ open  space fold  f filter: {label}  / find  esc back")
    }
}

/// Milestone 9.0.5 decision 13: the plan review's keys, at the gate or for a hold.
fn review_hint(app: &App) -> &'static str {
    match app.plan_review.as_ref().map(|review| &review.target) {
        Some(crate::app::ReviewTarget::Hold(_)) => {
            "a approve hold  x reject hold  j/k task  PgUp/PgDn scroll  esc back"
        }
        _ => "a approve  x reject  e edit  d drop  j/k task  PgUp/PgDn scroll  esc back",
    }
}

/// Pushes the first `count` key hints onto `spans`, in the styling shared by the
/// git-present and git-absent render paths.
fn push_hints(
    spans: &mut Vec<Span<'static>>,
    hints: &[(String, &'static str)],
    count: usize,
    accent: Color,
) {
    for (key, what) in hints.iter().take(count) {
        spans.push(Span::styled(key.clone(), Style::default().fg(accent)));
        spans.push(Span::styled(format!(" {what}  "), theme::muted()));
    }
}

fn spans_width(spans: &[Span<'_>]) -> u16 {
    spans
        .iter()
        .map(|s| UnicodeWidthStr::width(s.content.as_ref()) as u16)
        .sum()
}

fn hints_width(hints: &[(String, &'static str)], count: usize) -> u16 {
    hints
        .iter()
        .take(count)
        .map(|(key, what)| {
            UnicodeWidthStr::width(key.as_str()) as u16 + UnicodeWidthStr::width(*what) as u16 + 3
        })
        .sum()
}

/// One droppable part of the git segment, right of the head. `priority` is the order parts
/// are dropped in as the budget shrinks: the lowest priority goes first.
pub(crate) struct Part {
    pub(crate) text: String,
    style: Style,
    priority: u8,
}

/// Where a worktree's `HEAD` points. The inspector's `branch` field reads this
/// too, so a detached head is written `@<oid>` in exactly one place.
pub(crate) fn head_text(head: &Head) -> String {
    match head {
        Head::Branch(name) | Head::Unborn(name) => name.clone(),
        Head::Detached(oid) => format!("@{oid}"),
    }
}

/// What is uncommitted in a worktree — conflicts, dirty, untracked, in that
/// order — in the one vocabulary the client has for them. Empty when there is
/// nothing uncommitted.
///
/// The bar below ranks these by priority and drops them as its budget shrinks;
/// the inspector's `changes` field joins their texts and has room for all
/// three. Two renderings, one set of glyphs.
pub(crate) fn change_parts(state: &GitState) -> Vec<Part> {
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
    parts
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
/// exactly; conflicts, the clean tick, `(stale)` and `(unborn)` are not named by that
/// decision, so they are ranked around it — kept longer than dirty, since an active
/// conflict or a good status is worth more than the counts feeding it.
///
/// Controller ruling (not in decision 20): `(stale)` shares conflicts' priority rather than
/// being the first thing dropped. It is not another datum competing with dirty/untracked/
/// divergence for space — it is a trust flag on all of them. Dropping it first would let a
/// narrow terminal show a confident `main ●3 ?1 ⇡2⇣1` with nothing marking the read as
/// possibly stale, which is worse than showing `main (stale)` with the counts gone.
///
/// Controller ruling (not in decision 20): `(unborn)` is an ordinary part of the highest
/// priority, not a separate rendering path. The spec's `main (unborn)` table row is an
/// example, not an exhaustive rule: a fresh repository an agent has just scaffolded has
/// untracked files worth counting, and a failing probe against one is just as stale as
/// against any other — the earlier early-return could only ever render `main (unborn)`,
/// silently dropping both. Being the highest priority makes it the last thing dropped,
/// since it qualifies the head itself.
fn build_parts(state: &GitState) -> Vec<Part> {
    let mut parts = Vec::new();
    let unborn = matches!(state.head, Head::Unborn(_));
    if unborn {
        parts.push(Part {
            text: "(unborn)".to_string(),
            style: theme::muted(),
            priority: 7,
        });
    }
    parts.extend(change_parts(state));
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
    // `GitState::is_clean` is the single definition of cleanliness (it counts an
    // in-progress operation, which deriving it from "no parts so far" would only
    // accidentally agree with). An unborn head has no commit to be clean against, so
    // `(unborn)` stands in place of the tick and says more than it would.
    if state.is_clean() && !unborn {
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

/// The toast's columns: its text plus one space, clipped to `bar` — computed in `usize`,
/// so a toast wider than `u16::MAX` cannot overflow (review M8c.2 M1).
fn toast_columns(text: &str, bar: u16) -> u16 {
    let clipped = UnicodeWidthStr::width(text)
        .saturating_add(1)
        .min(usize::from(bar));
    u16::try_from(clipped).unwrap_or(bar)
}
