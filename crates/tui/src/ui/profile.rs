//! Milestone 9.10 decisions 27-31 (SP §4.4): the Profile screen, drawn over the body.
//! A frame titled `profile · <project>`; its first row the status line, right-aligned;
//! then the review card (`profile_card.rs`) or the profile: bold section heads, one row
//! per key (the bar, the label, the value, the check cell in one column, the key dimmed
//! at the right; under 60 columns the key column goes, then the cells), the `Advanced`
//! line; below the list, the selected row's hint or its ✗ line, and the daemon's last
//! text. A page draws as a kit dialog over it, and the screen's border is then muted
//! (9.0.6 decision 5). Every string a profile, a check, the scout or the daemon wrote
//! passes `safe_text`; the whole is folded with `theme::fold` in ASCII. Pure: `&App` in.

use crate::app::profile_screen::{ADVANCED_ROW, ProfileScreen, Shown, Side};
use crate::app::{App, region::KeyRegion};
use crate::profile_view::{ENV_ADD, Row};
use crate::profile_words;
use crate::safe_text::{multi_line, one_line};
use crate::theme::{Glyph, Palette, Role, dot, ellipsis, fold, glyph, role};
use crate::ui::kit::{self, Hint, cut, wrap_words};
use proto::{ProposalOrigin, ProposalState, RowEditState};
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Clear, Paragraph};
use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthStr;

/// The label's columns (a longer label pushes its value right by what it overflows).
const LABEL_W: usize = 13;
/// Where a value starts: the bar, two spaces, the label.
const VALUE_AT: usize = 3 + LABEL_W;
/// The cell's columns: `checking...` in ASCII.
const CELL_W: usize = 11;
/// The key column: the longest key (`check_timeout_secs`).
const KEY_W: usize = 18;
const GAP: usize = 2;
/// Under these interior widths the key column, then the cells' column, are dropped.
const KEYS_FROM: usize = 60;
const CELLS_FROM: usize = 40;

pub(super) fn hint(key: &str, word: &str, priority: u8) -> Hint {
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

/// Whether the selected row has a ✗ row edit (decision 31: `s` and `r` are offered).
fn failed_selected(s: &ProfileScreen) -> bool {
    s.rows()
        .get(s.selected)
        .and_then(|r| s.edit_of(&r.key))
        .is_some_and(|e| matches!(e, RowEditState::Failed { .. }))
}

/// The status bar's hints while the screen has the keys (9.0.6 decision 6), in
/// milestone 9.10 decision 29's order, dropped from the right (`esc` last of all).
pub(crate) fn hints(s: &ProfileScreen) -> Vec<Hint> {
    if s.page.is_some() {
        return vec![hint("esc", "back", 9)];
    }
    let mut list: Vec<(&str, &str)> = if s.showing_card() {
        vec![
            ("⏎", "use this"),
            ("e", "edit"),
            ("a", "advanced"),
            ("o", "output"),
            ("x", "discard"),
        ]
    } else {
        let mut list = vec![
            ("⏎", "open"),
            ("e", "edit"),
            ("u", "unset"),
            ("a", "advanced"),
            ("d", s.detect_title()),
            ("o", "output"),
        ];
        if s.review_proposal() {
            list.push(("x", "discard proposal"));
        }
        list
    };
    if failed_selected(s) {
        list.extend([("s", "save anyway"), ("r", "revert")]);
    }
    let mut out: Vec<Hint> = (list.iter().enumerate())
        .map(|(i, (k, w))| hint(k, w, 8u8.saturating_sub(i as u8)))
        .collect();
    let esc = if s.showing_card() { "later" } else { "back" };
    out.push(hint("esc", esc, 9));
    out
}

/// The profile listed now: the proposal while the card shows, else the stored one.
pub(super) fn listed(s: &ProfileScreen) -> Option<&Shown> {
    match if s.showing_card() {
        &s.proposal
    } else {
        &s.stored
    } {
        Side::Ready(shown) => Some(shown),
        _ => None,
    }
}

/// `text` sanitised, then folded for the palette.
pub(super) fn plain(text: &str, p: Palette) -> String {
    fold(&one_line(text), p.ascii)
}

/// `text` padded with spaces to `width` columns.
pub(super) fn pad(text: &str, width: usize) -> String {
    format!("{text}{}", " ".repeat(width.saturating_sub(text.width())))
}

/// `text` cut to its last `max` columns, starting with the ellipsis when cut.
fn cut_left(text: &str, max: usize, ell: &str) -> String {
    if text.width() <= max {
        return text.to_string();
    }
    let room = max.saturating_sub(ell.width());
    let mut kept = Vec::new();
    let mut used = 0;
    for g in text.graphemes(true).rev() {
        if used + g.width() > room {
            break;
        }
        used += g.width();
        kept.push(g);
    }
    kept.reverse();
    if max >= ell.width() {
        format!("{ell}{}", kept.concat())
    } else {
        kept.concat()
    }
}

/// Decision 23's role for a status line: `Done` ready, `Working` setting up or
/// re-checking, `Attention` review and out of date, `Failed` unreadable.
fn status_role(line: &str) -> Role {
    if line.starts_with("Can't") {
        Role::Failed
    } else if line.starts_with("Setting up") || line.ends_with("re-checking") {
        Role::Working
    } else if line.starts_with("Needs review") || line.starts_with("Out of date") {
        Role::Attention
    } else if line.starts_with("Ready") {
        Role::Done
    } else {
        Role::Muted
    }
}

/// The first row: the status line, right-aligned, cut from its left.
fn status_line(app: &App, s: &ProfileScreen, width: usize, p: Palette) -> Line<'static> {
    let (text, r) = match (&s.status, &s.status_failed) {
        (Some(status), _) => {
            let line = profile_words::status_line(status, app.run_now());
            (plain(&line, p), status_role(&line))
        }
        // No reply will come for the last `Status` (minor 2): why, until one does.
        (None, Some(why)) => (plain(why, p), Role::Failed),
        (None, None) => (format!("loading{}", ellipsis(p)), Role::Muted),
    };
    let text = cut_left(&text, width, ellipsis(p));
    let lead = " ".repeat(width.saturating_sub(text.width()));
    Line::from(vec![Span::raw(lead), Span::styled(text, role(r, p))])
}

/// What a row draws, before layout.
pub(super) struct Parts {
    /// `Delivery`: the bold head at column 1, its value and key on the same line.
    pub head: bool,
    pub label: String,
    pub value: String,
    pub value_role: Option<Role>,
    pub cell: Option<(String, Role)>,
    pub key: String,
}

/// One row: the bar, the label, the value (cut), the cell at its column, the key
/// dimmed at the right; the key column goes under 60 columns, then the cells'.
fn row_line(parts: &Parts, selected: bool, keys: bool, width: usize, p: Palette) -> Line<'static> {
    let ell = ellipsis(p);
    let bar = if selected {
        glyph(Glyph::Selection, p.ascii)
    } else {
        " "
    };
    let (show_key, show_cell) = (width >= KEYS_FROM, width >= CELLS_FROM);
    let right = usize::from(show_cell) * (GAP + CELL_W) + usize::from(show_key) * (GAP + KEY_W);
    let label = plain(&parts.label, p);
    let mut spans = vec![Span::styled(bar.to_string(), role(kit::bar_role(keys), p))];
    let lead = if parts.head {
        let label = cut(&label, VALUE_AT - 2, ell);
        spans.push(Span::styled(
            pad(&label, VALUE_AT - 1),
            Style::default().add_modifier(Modifier::BOLD),
        ));
        VALUE_AT
    } else {
        let label = cut(&label, width.saturating_sub(3), ell);
        let label_w = LABEL_W.max(label.width() + 1);
        spans.push(Span::raw(format!("  {}", pad(&label, label_w))));
        3 + label_w
    };
    let room = width.saturating_sub(lead + right);
    let value = cut(&plain(&parts.value, p), room, ell);
    let value_style = parts.value_role.map_or(Style::default(), |r| role(r, p));
    let padded = if right > 0 { pad(&value, room) } else { value };
    spans.push(Span::styled(padded, value_style));
    if show_cell {
        let (cell, r) = parts.cell.clone().unwrap_or((String::new(), Role::Muted));
        spans.push(Span::raw(" ".repeat(GAP)));
        let cell = cut(&fold(&cell, p.ascii), CELL_W, ell);
        let cell = if show_key { pad(&cell, CELL_W) } else { cell };
        spans.push(Span::styled(cell, role(r, p)));
    }
    if show_key {
        spans.push(Span::raw(" ".repeat(GAP)));
        let key = if parts.key == ENV_ADD {
            "env".to_string()
        } else {
            plain(&parts.key, p)
        };
        spans.push(Span::styled(cut(&key, KEY_W, ell), role(Role::Muted, p)));
    }
    Line::from(spans)
}

/// A row's cell on the profile: `checking…` while its row edit verifies, `✗` after a
/// ✗, else its check's `✓` / `✗` (decision 31).
fn profile_cell(s: &ProfileScreen, row: &Row, p: Palette) -> Option<(String, Role)> {
    match s.edit_of(&row.key) {
        Some(RowEditState::Verifying) => Some(("checking…".into(), Role::Working)),
        Some(RowEditState::Failed { .. }) => {
            Some((glyph(Glyph::Failed, p.ascii).into(), Role::Failed))
        }
        None => row.check.as_ref().map(|c| {
            let (g, r) = if c.ok {
                (Glyph::Passed, Role::Done)
            } else {
                (Glyph::Failed, Role::Failed)
            };
            (glyph(g, p.ascii).to_string(), r)
        }),
    }
}

/// A value as a row shows it: delivery in words (decision 26), `—` for none.
pub(super) fn value_text(
    key: &str,
    value: Option<&str>,
    shown: Option<&Shown>,
) -> (String, Option<Role>) {
    if key == "delivery.mode" {
        let delivery = shown.and_then(|s| s.profile.as_ref()?.delivery.as_ref());
        return (profile_words::delivery_text(delivery), None);
    }
    if key == ENV_ADD {
        return (String::new(), None);
    }
    match value {
        Some(v) => (v.to_string(), None),
        None => ("—".into(), Some(Role::Muted)),
    }
}

/// Agent- or tool-written lines, one by one, sanitised and cut to `width`, indented.
pub(super) fn tail_lines(
    text: &str,
    indent: usize,
    width: usize,
    p: Palette,
) -> Vec<Line<'static>> {
    multi_line(text)
        .split('\n')
        .map(|line| {
            let line = cut(&plain(line, p), width.saturating_sub(indent), ellipsis(p));
            Line::styled(
                format!("{}{line}", " ".repeat(indent)),
                role(Role::Muted, p),
            )
        })
        .collect()
}

/// The output `o` shows under a row: a ✗ row edit's, else its check's.
fn output_of(s: &ProfileScreen, row: &Row) -> Option<String> {
    if let Some(RowEditState::Failed { tail, .. }) = s.edit_of(&row.key) {
        return Some(tail.clone());
    }
    row.check.as_ref().map(|c| c.tail.clone())
}

/// `Advanced ▸     <summary>` folded, `Advanced ▾` open (decision 29).
fn advanced_line(
    s: &ProfileScreen,
    selected: bool,
    keys: bool,
    width: usize,
    p: Palette,
) -> Line<'static> {
    let bar = if selected {
        glyph(Glyph::Selection, p.ascii)
    } else {
        " "
    };
    let (mark, summary) = match (s.advanced, p.ascii) {
        (true, false) => ("▾", ""),
        (true, true) => ("v", ""),
        (false, false) => ("▸", profile_words::ADVANCED_SUMMARY),
        (false, true) => (">", profile_words::ADVANCED_SUMMARY),
    };
    let head = format!("Advanced {mark}");
    let mut spans = vec![
        Span::styled(bar.to_string(), role(kit::bar_role(keys), p)),
        Span::styled(head.clone(), Style::default().add_modifier(Modifier::BOLD)),
    ];
    if !summary.is_empty() {
        let room = width.saturating_sub(1 + head.width() + 5);
        spans.push(Span::raw("     "));
        spans.push(Span::styled(
            cut(summary, room, ellipsis(p)),
            role(Role::Muted, p),
        ));
    }
    Line::from(spans)
}

/// The listed rows' lines (section heads, rows, outputs, the Advanced line), and the
/// index of the selected row's line.
fn list_lines(
    app: &App,
    s: &ProfileScreen,
    width: usize,
    p: Palette,
) -> (Vec<Line<'static>>, usize) {
    let card = s.showing_card();
    let side = if card { &s.proposal } else { &s.stored };
    let shown = match side {
        Side::Loading => {
            let text = format!("loading{}", ellipsis(p));
            return (vec![Line::styled(text, role(Role::Muted, p))], 0);
        }
        Side::Failed(why) => {
            let text = cut(&plain(why, p), width, ellipsis(p));
            return (vec![Line::styled(text, role(Role::Failed, p))], 0);
        }
        // No stored profile: the status line says what to do.
        Side::Absent(_) => return (vec![], 0),
        Side::Ready(shown) => shown,
    };
    let mut out = Vec::new();
    if shown.profile.is_none() {
        out.push(Line::styled(
            "this profile does not read back",
            role(Role::Failed, p),
        ));
    }
    let keys = app.key_region() == KeyRegion::Screen;
    let bold = Style::default().add_modifier(Modifier::BOLD);
    let mut at = 0;
    let mut section = "";
    for (i, row) in s.rows().iter().enumerate() {
        let selected = i == s.selected;
        if row.key == ADVANCED_ROW {
            at = if selected { out.len() } else { at };
            out.push(advanced_line(s, selected, keys, width, p));
            section = "";
            continue;
        }
        let head = row.key == "delivery.mode" && row.section == "Delivery";
        if row.section != section && !head {
            out.push(Line::styled(format!(" {}", row.section), bold));
        }
        section = row.section;
        let parts = if card {
            card::parts(s, row, head, p)
        } else {
            let (value, value_role) = value_text(&row.key, row.value.as_deref(), Some(shown));
            Parts {
                head,
                label: row.label.clone(),
                value,
                value_role,
                cell: profile_cell(s, row, p),
                key: row.key.clone(),
            }
        };
        at = if selected { out.len() } else { at };
        out.push(row_line(&parts, selected, keys, width, p));
        if s.expanded.as_deref() == Some(row.key.as_str())
            && let Some(tail) = output_of(s, row)
        {
            out.extend(tail_lines(&tail, 6, width, p));
        }
    }
    if card {
        out.extend(card::dropped_lines(shown, width, p));
        // On the last row, the view runs to the end, so the dropped commands show.
        if s.selected + 1 >= s.rows().len() {
            at = out.len().saturating_sub(1);
        }
    }
    (out, at)
}

/// Below the list: the selected row's hint (`<label> — <hint>`, muted), or its ✗ line
/// (decision 31).
fn hint_line(s: &ProfileScreen, width: usize, p: Palette) -> Option<Line<'static>> {
    listed(s)?;
    let row = s.rows().into_iter().nth(s.selected)?;
    if row.key == ADVANCED_ROW {
        return None;
    }
    let (text, r) = match s.edit_of(&row.key) {
        Some(RowEditState::Failed { reason, .. }) => (
            format!(
                "couldn't verify: {} · o output · s save anyway · r revert",
                one_line(reason)
            ),
            Role::Failed,
        ),
        _ => {
            let hint = profile_words::hint(&row.key);
            if hint.is_empty() {
                return None;
            }
            (format!("{} — {hint}", one_line(&row.label)), Role::Muted)
        }
    };
    let text = cut(&fold(&text, p.ascii), width.saturating_sub(3), ellipsis(p));
    Some(Line::styled(format!("   {text}"), role(r, p)))
}

/// Decision 23: a failed set-up's reason, the screen's error row.
fn setup_failure(s: &ProfileScreen) -> Option<String> {
    let proposal = s.status.as_ref()?.proposal.as_ref()?;
    match (&proposal.state, &proposal.origin) {
        (_, ProposalOrigin::Edit { .. }) => None,
        (ProposalState::Failed { reason }, _) => Some(format!("setting up failed: {reason}")),
        _ => None,
    }
}

/// The daemon's last text: a refusal (or a failed set-up) in `Failed`, a `Done` muted.
fn footer(s: &ProfileScreen, width: usize, p: Palette) -> Vec<Line<'static>> {
    let (text, r) = match (&s.error, setup_failure(s), &s.message) {
        (Some(e), _, _) => (e.clone(), Role::Failed),
        (None, Some(failed), _) => (failed, Role::Failed),
        (None, None, Some(m)) => (m.clone(), Role::Muted),
        _ => return vec![],
    };
    let w = width.max(1);
    let mut lines = wrap_words(&plain(&text, p), w);
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

/// The screen's parts: its fixed head (the status line, the card's title), its list
/// (scrolled), the selected line's index in it, and its fixed foot (the hint line,
/// the daemon's text).
struct Body {
    head: Vec<Line<'static>>,
    list: Vec<Line<'static>>,
    at: usize,
    foot: Vec<Line<'static>>,
}

fn body(app: &App, s: &ProfileScreen, width: u16) -> Body {
    let p = app.palette();
    let w = usize::from(width);
    let mut head = vec![status_line(app, s, w, p), Line::default()];
    if s.showing_card() {
        head.push(card::title(s, w, p));
        head.push(Line::default());
    }
    let (list, at) = list_lines(app, s, w, p);
    let mut foot = Vec::new();
    if let Some(line) = hint_line(s, w, p) {
        foot.extend([Line::default(), line]);
    }
    foot.extend(footer(s, w, p));
    Body {
        head,
        list,
        at,
        foot,
    }
}

/// Everything the screen shows, unscrolled: the head, the list, the foot.
#[cfg(test)]
pub(crate) fn body_lines(app: &App, s: &ProfileScreen, width: u16) -> Vec<Line<'static>> {
    let b = body(app, s, width);
    let mut out = b.head;
    out.extend(b.list);
    out.extend(b.foot);
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
    let b = body(app, s, inner.width);
    let rows = usize::from(inner.height)
        .saturating_sub(b.head.len())
        .saturating_sub(b.foot.len());
    let mut all = b.head;
    all.extend(kit::window(b.list, b.at, rows, p));
    all.extend(b.foot);
    frame.render_widget(Paragraph::new(all), inner);
    if let Some(page) = &s.page {
        render_page(
            frame,
            s,
            page,
            area,
            app.key_region() == KeyRegion::Screen,
            p,
        );
    }
}

#[path = "profile_card.rs"]
mod card;
#[path = "profile_pages.rs"]
mod pages;
pub(crate) use pages::list_width;
#[cfg(test)]
pub(crate) use pages::page_lines;
use pages::render_page;

#[cfg(test)]
#[path = "profile_tests.rs"]
pub(crate) mod tests;

#[cfg(test)]
#[path = "profile_card_tests.rs"]
mod card_tests;

#[cfg(test)]
#[path = "profile_rows_tests.rs"]
mod rows_tests;

#[cfg(test)]
#[path = "profile_pages_tests.rs"]
mod pages_tests;
