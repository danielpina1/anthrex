//! Milestone 9.0.6 decisions 33-35: the Profile screen, drawn over the body. A frame
//! titled `profile · <project>`, the tab row, the tab's lines (scrolled to keep the
//! selected row in view, cuts marked), then the daemon's last text: its refusal in
//! `Failed` (decision 37's among them). A page draws as a kit dialog over it, and the
//! screen's border is then muted (decision 5). Every string a profile, a check, the
//! scout or the daemon wrote passes `safe_text` here or in the kit. Pure: `&App` in.

use crate::app::profile_screen::{ProfileScreen, ProfileTab, Shown, Side};
use crate::app::{App, region::KeyRegion};
use crate::inspector::run_format::format_duration;
use crate::profile_view::{ENV_ADD, Row, check_cell};
use crate::safe_text::{multi_line, one_line};
use crate::theme::{Glyph, Palette, Role, glyph, role};
use crate::ui::kit::{self, Hint, cut, wrap_words};
use proto::ProposalState;
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::Modifier;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Clear, Paragraph};
use unicode_width::UnicodeWidthStr;

/// The key column's width: the longest key (`check_timeout_secs`) and a space.
const KEY_W: usize = 19;

fn ellipsis(p: Palette) -> &'static str {
    if p.ascii { "..." } else { "…" }
}

fn dot(p: Palette) -> &'static str {
    if p.ascii { "-" } else { "·" }
}

fn hint(key: &str, word: &str, priority: u8) -> Hint {
    Hint {
        key: key.to_string(),
        word: word.to_string(),
        priority,
    }
}

/// The project's directory name, as the title names it.
fn project_name(s: &ProfileScreen) -> String {
    s.dir
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| s.dir.display().to_string())
}

/// The status bar's hints while the screen has the keys (decision 6).
pub(crate) fn hints(s: &ProfileScreen) -> Vec<Hint> {
    if s.page.is_some() {
        return vec![hint("esc", "back", 9)];
    }
    let ready = matches!(s.proposal, Side::Ready(_));
    let mut out = match s.tab {
        ProfileTab::Status => vec![
            hint("d", "detect", 7),
            hint("x", "reject", 4),
            hint("s", "store", 3),
            hint("tab", "profile", 5),
        ],
        ProfileTab::Profile => vec![
            hint("j/k", "move", 5),
            hint("e", "edit", 7),
            hint("u", "unset", 6),
            hint("p", if s.proposed { "stored" } else { "proposal" }, 6),
            hint("⏎", "output", 3),
            hint("d", "detect", 2),
            hint("tab", "status", 4),
        ],
    };
    if ready {
        out.insert(1, hint("c", "confirm", 6));
    }
    out.push(hint("esc", "back", 9));
    out
}

/// `stored`, `stale`, `unparseable` and `proposal` (Interfaces "Profile status rows").
fn status_rows(app: &App, s: &ProfileScreen, width: u16, p: Palette) -> Vec<Line<'static>> {
    let Some(status) = &s.status else {
        // No reply will come for the last `Status` (minor 2): why, until one does.
        if let Some(why) = &s.status_failed {
            let text = cut(&one_line(why), usize::from(width), ellipsis(p));
            return vec![Line::styled(text, role(Role::Failed, p))];
        }
        return vec![Line::styled(
            format!("loading{}", ellipsis(p)),
            role(Role::Muted, p),
        )];
    };
    let mut rows = Vec::new();
    let stored = match (status.source, status.confirmed_at) {
        (proto::ProfileSource::Stored, Some(at)) => {
            format!("confirmed {} ago", format_duration(app.run_age(at)))
        }
        (proto::ProfileSource::Stored, None) => "stored".to_string(),
        _ => "none".to_string(),
    };
    rows.push(("stored".to_string(), stored));
    if !status.stale.is_empty() {
        rows.push(("stale".to_string(), status.stale.join(", ")));
    }
    if let Some(text) = &status.unparseable {
        rows.push(("unparseable".to_string(), text.clone()));
    }
    rows.push(("proposal".to_string(), proposal_text(app, status)));
    let mut out = kit::labelled_rows(&rows, width, p);
    out.push(Line::default());
    out.push(store_line(s, p));
    out
}

/// Interfaces "Profile status rows": the proposal's state.
fn proposal_text(app: &App, status: &proto::ProfileStatus) -> String {
    match status.proposal.as_ref().map(|r| (&r.state, r.started_at)) {
        None => "none".to_string(),
        Some((ProposalState::Preparing | ProposalState::Scouting, at)) => {
            format!("scout running {}", format_duration(app.run_age(at)))
        }
        Some((ProposalState::Verifying, _)) => "verifying".to_string(),
        Some((ProposalState::Ready, _)) => "ready".to_string(),
        Some((ProposalState::Failed { reason }, _)) => format!("failed: {reason}"),
    }
}

/// `s  store once verification passes  ‹ off ›`.
fn store_line(s: &ProfileScreen, p: Palette) -> Line<'static> {
    let value = if s.store_on_pass { "on" } else { "off" };
    Line::from(vec![
        Span::styled("s ", role(Role::Accent, p)),
        Span::styled("store once verification passes  ", role(Role::Muted, p)),
        Span::raw(kit::choice_in(value, p)),
    ])
}

/// `text` padded with spaces to `width` columns.
fn pad(text: &str, width: usize) -> String {
    format!("{text}{}", " ".repeat(width.saturating_sub(text.width())))
}

/// One key row: selection bar, mark, key, value, check cell.
fn key_row(row: &Row, selected: bool, width: usize, p: Palette) -> Line<'static> {
    let sel = if selected {
        glyph(Glyph::Selection, p.ascii)
    } else {
        " "
    };
    let mark = row.mark.map_or(" ", |m| m.glyph(p.ascii));
    let key = if row.key == ENV_ADD {
        "env.<NAME>".to_string()
    } else {
        one_line(&row.key)
    };
    let key = pad(&cut(&key, KEY_W - 1, ellipsis(p)), KEY_W);
    let cell = row.check.as_ref().map(|c| (check_cell(c, p.ascii), c.ok));
    let cell_w = cell.as_ref().map_or(0, |(text, _)| text.width() + 2);
    let room = width.saturating_sub(3 + KEY_W + cell_w);
    let (value, value_style) = match (&row.value, row.key == ENV_ADD) {
        (_, true) => ("add a variable".to_string(), role(Role::Muted, p)),
        (Some(v), _) => (one_line(v), ratatui::style::Style::default()),
        (None, _) => ("unset".to_string(), role(Role::Muted, p)),
    };
    let value = cut(&value, room, ellipsis(p));
    let key_style = if selected {
        ratatui::style::Style::default().add_modifier(Modifier::BOLD)
    } else {
        role(Role::Muted, p)
    };
    let mut spans = vec![
        Span::styled(sel.to_string(), role(Role::Accent, p)),
        Span::raw(format!("{mark} ")),
        Span::styled(key, key_style),
        Span::styled(value.clone(), value_style),
    ];
    if let Some((text, ok)) = cell {
        let gap = room.saturating_sub(value.width()) + 2;
        spans.push(Span::raw(" ".repeat(gap)));
        spans.push(Span::styled(
            text,
            role(if ok { Role::Done } else { Role::Failed }, p),
        ));
    }
    Line::from(spans)
}

/// Agent- or tool-written lines, one by one, sanitised and cut to `width`, indented.
fn tail_lines(text: &str, indent: usize, width: usize, p: Palette) -> Vec<Line<'static>> {
    multi_line(text)
        .split('\n')
        .map(|line| {
            let line = cut(&one_line(line), width.saturating_sub(indent), ellipsis(p));
            Line::styled(
                format!("{}{line}", " ".repeat(indent)),
                role(Role::Muted, p),
            )
        })
        .collect()
}

/// The Profile tab's lines, and the index of the selected row's line.
fn profile_rows(
    app: &App,
    s: &ProfileScreen,
    width: u16,
    p: Palette,
) -> (Vec<Line<'static>>, usize) {
    let w = usize::from(width);
    let bold = ratatui::style::Style::default().add_modifier(Modifier::BOLD);
    let mut out = vec![if s.proposed {
        Line::from(vec![
            Span::styled("proposal  ", bold),
            Span::styled(
                format!(
                    "+ added  {} removed  ~ changed",
                    crate::profile_view::Mark::Removed.glyph(p.ascii)
                ),
                role(Role::Muted, p),
            ),
        ])
    } else {
        Line::styled("stored profile", bold)
    }];
    // A detection under way: its state, not the proposal it replaces.
    if s.proposed
        && s.in_progress()
        && let Some(status) = &s.status
    {
        out.push(Line::styled(
            one_line(&proposal_text(app, status)),
            role(Role::Working, p),
        ));
        return (out, 0);
    }
    let shown: &Shown = match s.viewed() {
        Side::Loading => {
            out.push(Line::styled(
                format!("loading{}", ellipsis(p)),
                role(Role::Muted, p),
            ));
            return (out, 0);
        }
        Side::Failed(why) => {
            let text = cut(&one_line(why), w, ellipsis(p));
            out.push(Line::styled(text, role(Role::Failed, p)));
            return (out, 0);
        }
        Side::Absent(_) => {
            let text = if s.proposed {
                "no proposal"
            } else {
                "no stored profile"
            };
            out.push(Line::styled(text, role(Role::Muted, p)));
            return (out, 0);
        }
        Side::Ready(shown) => shown,
    };
    if shown.profile.is_none() {
        out.push(Line::styled(
            "this profile does not read back; c shows its text",
            role(Role::Failed, p),
        ));
    }
    let mut at = 0;
    let mut group = "";
    let rows = s.rows();
    for (i, row) in rows.iter().enumerate() {
        if row.group != group {
            group = row.group;
            out.push(Line::styled(group.to_string(), bold));
        }
        if i == s.selected {
            at = out.len();
        }
        out.push(key_row(row, i == s.selected, w, p));
        if s.expanded.as_deref() == Some(row.key.as_str())
            && let Some(check) = &row.check
        {
            out.extend(tail_lines(&check.tail, 6, w, p));
        }
    }
    let last = s.selected + 1 == rows.len();
    if !shown.dropped.is_empty() {
        out.push(Line::styled("dropped", bold));
        for d in &shown.dropped {
            let head = format!("{}  {}", one_line(&d.key), one_line(&d.command));
            out.push(Line::raw(format!(
                "  {}",
                cut(&head, w.saturating_sub(2), ellipsis(p))
            )));
            for part in wrap_words(&one_line(&d.reason), w.saturating_sub(4).max(1)) {
                out.push(Line::styled(
                    format!("    {part}"),
                    role(Role::Attention, p),
                ));
            }
        }
    }
    // On the last row, the view runs to the end, so the dropped commands show.
    if last {
        at = out.len() - 1;
    }
    (out, at)
}

/// The tab row: the current tab in bold, the other muted.
fn tab_row(s: &ProfileScreen, p: Palette) -> Line<'static> {
    let style = |on: bool| {
        if on {
            ratatui::style::Style::default().add_modifier(Modifier::BOLD | Modifier::UNDERLINED)
        } else {
            role(Role::Muted, p)
        }
    };
    Line::from(vec![
        Span::styled("status", style(s.tab == ProfileTab::Status)),
        Span::raw("  "),
        Span::styled("profile", style(s.tab == ProfileTab::Profile)),
    ])
}

/// The daemon's last text: a refusal in `Failed`, a `Done` muted.
fn footer(s: &ProfileScreen, width: u16, p: Palette) -> Vec<Line<'static>> {
    let (text, r) = match (&s.error, &s.message) {
        (Some(e), _) => (e, Role::Failed),
        (None, Some(m)) => (m, Role::Muted),
        _ => return vec![],
    };
    let w = usize::from(width).max(1);
    let mut lines = wrap_words(&one_line(text), w);
    if lines.len() > 3 {
        lines.truncate(3);
        // What is cut is marked (principle 6).
        let last = format!("{} {}", lines[2], ellipsis(p));
        lines[2] = cut(&last, w, ellipsis(p));
    }
    lines
        .into_iter()
        .map(|l| Line::styled(l, role(r, p)))
        .collect()
}

/// The tab's fixed head (the Profile tab's view line), its lines, and the selected
/// line's index (0 on the Status tab).
fn tab_lines(
    app: &App,
    s: &ProfileScreen,
    width: u16,
) -> (Vec<Line<'static>>, Vec<Line<'static>>, usize) {
    let p = app.palette();
    match s.tab {
        ProfileTab::Status => (vec![], status_rows(app, s, width, p), 0),
        ProfileTab::Profile => {
            let (mut lines, at) = profile_rows(app, s, width, p);
            let head = lines.remove(0);
            (vec![head], lines, at.saturating_sub(1))
        }
    }
}

/// Everything the screen shows, unscrolled: the tab row, the tab's lines, the footer.
#[cfg(test)]
pub(crate) fn body_lines(app: &App, s: &ProfileScreen, width: u16) -> Vec<Line<'static>> {
    let (head, lines, _) = tab_lines(app, s, width);
    let mut out = vec![tab_row(s, app.palette())];
    out.extend(head);
    out.extend(lines);
    out.extend(footer(s, width, app.palette()));
    out
}

/// Draws the screen over `area` (the body), and its page over that.
pub fn render(frame: &mut Frame, app: &App, s: &ProfileScreen, area: Rect) {
    let p = app.palette();
    let title = format!("profile {} {}", dot(p), project_name(s));
    // Decision 5: a page or any modal over the screen has the one accented border.
    let keys_here = s.page.is_none() && app.key_region() == KeyRegion::Screen;
    let block = kit::screen_frame(&title, keys_here, p);
    let inner = block.inner(area);
    frame.render_widget(Clear, area);
    frame.render_widget(block, area);
    if inner.width == 0 || inner.height == 0 {
        return;
    }
    let foot = footer(s, inner.width, p);
    let (head, lines, at) = tab_lines(app, s, inner.width);
    let rows = usize::from(inner.height)
        .saturating_sub(1 + head.len())
        .saturating_sub(foot.len());
    let mut all = vec![tab_row(s, p)];
    all.extend(head);
    all.extend(kit::window(lines, at, rows, p));
    all.extend(foot);
    frame.render_widget(Paragraph::new(all), inner);
    if let Some(page) = &s.page {
        render_page(frame, s, page, area, p);
    }
}

#[path = "profile_pages.rs"]
mod pages;
#[cfg(test)]
pub(crate) use pages::page_lines;
use pages::render_page;

#[cfg(test)]
#[path = "profile_tests.rs"]
pub(crate) mod tests;
