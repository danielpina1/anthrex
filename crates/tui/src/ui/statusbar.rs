use crate::app::{App, Link, ToastLevel};
use crate::theme::{self, Glyph, Role, role};
use crate::ui::kit;
use crate::ui::statusbar_modes::{self, Body};
use proto::{GitOperation, GitState, Head};
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use unicode_width::UnicodeWidthStr;

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

/// The columns the bar keeps between the hints and the git segment.
const GAP: u16 = 2;

pub fn render(frame: &mut Frame, app: &App, area: Rect) {
    let palette = app.palette();
    // Milestone 9.0.7 (task 2 note 12): the `Accent` role, never the raw setting.
    let accent = role(Role::Accent, palette);
    let muted = role(Role::Muted, palette);
    let mut spans = Vec::new();
    if let Some(badge) = statusbar_modes::badge(app) {
        let style = Style {
            bg: accent.fg,
            ..Style::default()
                .fg(Color::Black)
                .add_modifier(Modifier::BOLD)
        };
        spans.push(Span::styled(badge, style));
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
                    muted,
                ));
                spans.push(Span::raw(" "));
            }
            // Decision 34: the 30 s window elapsed; only a fresh `C-b r` tries again.
            Link::Lost { .. } => {
                push_disconnected(&mut spans);
                spans.push(Span::styled(
                    format!("{} r to reconnect", app.settings.prefix_label),
                    muted,
                ));
                spans.push(Span::raw(" "));
            }
        }
    }

    // Decision 20, and milestone 9.0.7 decision 32: with the sidebar hidden,
    // `⚑ <n> <prefix> a` right after the mode badge, the mark and count in the top
    // alert's role, the key in the accent. It is measured before the hints, so no
    // hint drop reaches it.
    if !app.sidebar_visible {
        let all = crate::app::alerts(app);
        if let Some(top) = all.first() {
            spans.push(Span::styled(
                format!(
                    "{} {}",
                    theme::alert_glyph(top.priority, palette.ascii),
                    all.len()
                ),
                theme::alert_style(top.priority, palette),
            ));
            spans.push(Span::raw(" "));
            spans.push(Span::styled(
                format!(
                    "{} a",
                    crate::safe_text::one_line(&app.settings.prefix_label)
                ),
                accent,
            ));
            spans.push(Span::raw("  "));
        }
    }

    let toast_width = app
        .toast_text()
        .map(|text| toast_columns(text, area.width))
        .unwrap_or(0);

    if statusbar_modes::filtering(app) {
        spans.push(Span::styled(format!("/{}", app.tree.filter), muted));
    } else {
        let body = statusbar_modes::body(app);
        let used = spans_width(&spans);
        let available = area.width.saturating_sub(used).saturating_sub(toast_width);
        spans.extend(body_spans(app, &body, available, palette));
    }
    frame.render_widget(Paragraph::new(Line::from(spans)), area);

    if let Some(text) = app.toast_text() {
        let width = toast_columns(text, area.width);
        let right = Rect {
            x: area.x + area.width - width,
            width,
            ..area
        };
        let style = match app.toast_level() {
            Some(ToastLevel::Error) => role(Role::Failed, palette).add_modifier(Modifier::BOLD),
            Some(ToastLevel::Warn) => role(Role::Attention, palette),
            _ => accent.add_modifier(Modifier::BOLD),
        };
        let toast = Span::styled(text.to_string(), style);
        frame.render_widget(Paragraph::new(Line::from(toast)), right);
    }
}

/// The hint line and, in the default and prefix bars, the focused worktree's git
/// segment, in `available` columns. Hints drop whole through `kit::hints` before the git
/// segment gives up any of its own parts (decision 20); `esc` never drops.
fn body_spans(
    app: &App,
    body: &Body,
    available: u16,
    palette: theme::Palette,
) -> Vec<Span<'static>> {
    let mut out = Vec::new();
    let mut remaining = available;
    if let Some((label, glyph)) = &body.lead {
        let glyph = theme::fold(glyph, palette.ascii);
        let lead = format!("{label} {glyph} ");
        remaining = remaining.saturating_sub(UnicodeWidthStr::width(lead.as_str()) as u16);
        out.push(Span::styled(label.clone(), role(Role::Accent, palette)));
        out.push(Span::styled(
            format!(" {glyph} "),
            role(Role::Muted, palette),
        ));
    }
    let git = body.git.then(|| app.focused_git()).flatten();
    let full_git_width = git.map_or(0, |state| {
        spans_width(&git_spans_in(state, usize::MAX, palette))
    });
    let reserved = if git.is_some() {
        full_git_width + GAP
    } else {
        0
    };
    let line = kit::hints_joined(
        remaining.saturating_sub(reserved),
        &body.hints,
        body.separator,
        palette,
    );
    let line_width = line.width() as u16;
    out.extend(line.spans);
    if let Some(state) = git {
        let gap = if out.is_empty() { 0 } else { GAP };
        let budget = remaining.saturating_sub(line_width).saturating_sub(gap);
        let parts = git_spans_in(state, usize::from(budget), palette);
        if !parts.is_empty() {
            out.push(Span::raw(" ".repeat(usize::from(gap))));
        }
        out.extend(parts);
    }
    out
}

fn spans_width(spans: &[Span<'_>]) -> u16 {
    spans
        .iter()
        .map(|s| UnicodeWidthStr::width(s.content.as_ref()) as u16)
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
pub(crate) fn change_parts(state: &GitState, p: theme::Palette) -> Vec<Part> {
    let mut parts = Vec::new();
    if state.conflicts > 0 {
        parts.push(Part {
            text: format!(
                "{}{}",
                theme::glyph(Glyph::Warning, p.ascii),
                state.conflicts
            ),
            style: role(Role::Failed, p),
            priority: 5,
        });
    }
    if state.dirty > 0 {
        parts.push(Part {
            text: format!("{}{}", theme::glyph(Glyph::Live, p.ascii), state.dirty),
            style: role(Role::Muted, p),
            priority: 4,
        });
    }
    if state.untracked > 0 {
        parts.push(Part {
            text: format!("?{}", state.untracked),
            style: role(Role::Muted, p),
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
fn build_parts(state: &GitState, p: theme::Palette) -> Vec<Part> {
    let mut parts = Vec::new();
    let muted = role(Role::Muted, p);
    let unborn = matches!(state.head, Head::Unborn(_));
    if unborn {
        parts.push(Part {
            text: "(unborn)".to_string(),
            style: muted,
            priority: 7,
        });
    }
    parts.extend(change_parts(state, p));
    if state.ahead > 0 || state.behind > 0 {
        let (up, down) = if p.ascii { ("^", "v") } else { ("⇡", "⇣") };
        let mut text = String::new();
        if state.ahead > 0 {
            text.push_str(&format!("{up}{}", state.ahead));
        }
        if state.behind > 0 {
            text.push_str(&format!("{down}{}", state.behind));
        }
        parts.push(Part {
            text,
            style: muted,
            priority: 3,
        });
    }
    if let Some(op) = state.operation {
        parts.push(Part {
            text: operation_name(op).to_string(),
            style: role(Role::Failed, p),
            priority: 1,
        });
    }
    // `GitState::is_clean` is the single definition of cleanliness (it counts an
    // in-progress operation, which deriving it from "no parts so far" would only
    // accidentally agree with). An unborn head has no commit to be clean against, so
    // `(unborn)` stands in place of the tick and says more than it would.
    if state.is_clean() && !unborn {
        parts.push(Part {
            text: theme::glyph(Glyph::Passed, p.ascii).to_string(),
            style: role(Role::Done, p),
            priority: 6,
        });
    }
    if state.stale {
        parts.push(Part {
            text: "(stale)".to_string(),
            style: muted,
            priority: 5, // matches conflicts — see the controller ruling above.
        });
    }
    parts
}

/// The spans for a worktree's git state that fit in `budget` columns. Parts drop right to
/// left per decision 20 as the budget shrinks; below the head's own width, nothing is
/// returned at all.
#[cfg(test)]
pub fn git_spans(state: &GitState, budget: usize) -> Vec<Span<'static>> {
    git_spans_in(state, budget, theme::Palette::PLAIN)
}

/// [`git_spans`] in the palette's roles, its marks in ASCII when `p.ascii`.
pub fn git_spans_in(state: &GitState, budget: usize, p: theme::Palette) -> Vec<Span<'static>> {
    let head = head_text(&state.head);
    let head_width = UnicodeWidthStr::width(head.as_str());
    if head_width > budget {
        return vec![];
    }
    let head_span = Span::styled(head, Style::default().add_modifier(Modifier::BOLD));

    let mut parts = build_parts(state, p);
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
#[path = "statusbar_kit_tests.rs"]
mod kit_tests;
#[cfg(test)]
#[path = "statusbar_polish_tests.rs"]
mod polish_tests;
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
