//! Milestone 9.8 (MR §5.1, decisions 35 and 36): the Settings screen's `models`
//! section, drawn as the spec's table. The scope line (` models · scope ‹ everywhere ›
//! / this repo (<basename>)`, `tab: limits` on the right) and a blank row; the header
//! and one row per role: a marker column (`▸` selected, `●` overridden in `this repo`),
//! the role in 22 columns, the model in 24, the effort in 9, the fallback; brainstorm's
//! two models alone (preflight F17); an inherited row dimmed and closed by
//! ` (everywhere)`. Below: a blank row, each row's warnings in `Attention`, the
//! screen's problems and notes, its last save, and the keys, dropped from the right
//! until they fit. Every row is cut to the interior with `…`; the table scrolls to keep
//! the selection in view. The state cleaned every catalog string. Pure: `&App` in.

use crate::app::App;
use crate::app::models_table::{RowKey, Scope, TableRow};
use crate::app::settings_screen::SettingsScreen;
use crate::theme::{Palette, Role, dot, ellipsis, fold, role};
use crate::ui::kit::{self, cut};
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use unicode_width::UnicodeWidthStr;

/// The role's, the model's and the effort's columns (preflight T11).
pub const ROLE_W: usize = 22;
pub const MODEL_W: usize = 24;
pub const EFFORT_W: usize = 9;
/// What closes an inherited row in `this repo`.
pub const EVERYWHERE: &str = " (everywhere)";
/// The keys under the table, kept from the left.
const KEYS: [(&str, &str); 6] = [
    ("⏎", "choose model"),
    ("e", "effort"),
    ("f", "if-it-struggles"),
    ("x", "reset"),
    ("w", "save"),
    ("esc", "back"),
];

/// `text` with the table's and the picker's glyphs in ASCII when the palette asks.
pub(crate) fn glyphs(text: &str, p: Palette) -> String {
    if !p.ascii {
        return text.to_string();
    }
    let mut out = fold(text, true);
    for (from, to) in [
        ('▸', ">"),
        ('▾', "v"),
        ('●', "*"),
        ('⚠', "!"),
        ('─', "-"),
        ('✗', "x"),
    ] {
        out = out.replace(from, to);
    }
    out
}

/// `text` padded to `width`, and at least one space after it.
fn cell(text: &str, width: usize) -> String {
    let width = width.max(text.width() + 1);
    format!("{text}{}", " ".repeat(width - text.width()))
}

/// The scope line: the two scopes, the current one between `‹ ›`, `this repo (none)`
/// dimmed with no project, and `tab: limits` on the right when it fits.
fn scope_line(s: &SettingsScreen, width: usize, p: Palette) -> Line<'static> {
    let m = &s.models;
    let name = (m.project.as_ref())
        .and_then(|d| d.file_name())
        .map_or_else(|| "none".to_string(), |n| n.to_string_lossy().to_string());
    let repo = format!("this repo ({})", crate::safe_text::one_line(&name));
    let (everywhere, repo) = match m.scope {
        Scope::Everywhere => (kit::choice_in("everywhere", p), repo),
        Scope::Repo => ("everywhere".to_string(), kit::choice_in(&repo, p)),
    };
    let repo_style = if m.project.is_none() {
        role(Role::Muted, p)
    } else {
        Style::default()
    };
    let left = format!(" models {} scope ", dot(p));
    let used = left.width() + everywhere.width() + 3 + repo.width();
    let right = "tab: limits";
    let mut spans = vec![
        Span::raw(left),
        Span::raw(everywhere),
        Span::raw(" / "),
        Span::styled(repo, repo_style),
    ];
    if used + 1 + right.width() <= width {
        spans.push(Span::raw(" ".repeat(width - used - right.width())));
        spans.push(Span::styled(right, role(Role::Muted, p)));
    }
    Line::from(spans)
}

/// The header row.
fn header(width: usize, p: Palette) -> Line<'static> {
    let text = format!(
        " {}{}{}IF IT STRUGGLES",
        cell("ROLE", ROLE_W),
        cell("MODEL", MODEL_W),
        cell("EFFORT", EFFORT_W)
    );
    Line::styled(cut(&text, width, ellipsis(p)), role(Role::Muted, p))
}

/// One role's row: its marker, then its cells, cut to `width`.
fn row_line(r: &TableRow, selected: bool, keys: bool, width: usize, p: Palette) -> Line<'static> {
    let mut text = cell(&r.role, ROLE_W);
    match r.key {
        // Preflight F17: brainstorm's two models, no effort or fallback column.
        RowKey::Brainstorm => text.push_str(&r.model),
        RowKey::Role(_) => {
            text.push_str(&cell(&r.model, MODEL_W));
            text.push_str(&cell(&r.effort, EFFORT_W));
            text.push_str(&r.fallback);
        }
    }
    let mut text = text.trim_end().to_string();
    if r.inherited {
        text.push_str(EVERYWHERE);
    }
    let style = if r.inherited {
        role(Role::Muted, p)
    } else {
        Style::default()
    };
    let (mark, mark_style) = match (selected, r.overridden) {
        (true, _) => ("▸", role(kit::bar_role(keys), p)),
        (false, true) => ("●", style),
        (false, false) => (" ", style),
    };
    let rest = cut(&glyphs(&text, p), width.saturating_sub(1), ellipsis(p));
    Line::from(vec![
        Span::styled(glyphs(mark, p), mark_style),
        Span::styled(rest, style),
    ])
}

/// The keys under the table, entries dropped from the right until they fit.
fn keys_line(width: usize, p: Palette) -> Line<'static> {
    let mut n = KEYS.len();
    let text_of = |n: usize| -> String {
        let parts: Vec<String> = KEYS[..n]
            .iter()
            .map(|(k, w)| format!("{} {w}", fold(k, p.ascii)))
            .collect();
        format!(" {}", parts.join("   "))
    };
    while n > 0 && text_of(n).width() > width {
        n -= 1;
    }
    if n == 0 {
        return Line::default();
    }
    let mut spans = vec![Span::raw(" ")];
    for (i, (k, w)) in KEYS[..n].iter().enumerate() {
        if i > 0 {
            spans.push(Span::raw("   "));
        }
        spans.push(Span::styled(fold(k, p.ascii), role(Role::Accent, p)));
        spans.push(Span::styled(format!(" {w}"), role(Role::Muted, p)));
    }
    Line::from(spans)
}

/// Each row's warnings: `⚠ <role>: <warning>`.
fn warning_lines(rows: &[TableRow], width: usize, p: Palette) -> Vec<Line<'static>> {
    let mut out = Vec::new();
    for r in rows {
        for w in &r.warnings {
            let text = format!("⚠ {}: {}", r.role.trim(), w.trim_start_matches("⚠ "));
            let text = cut(&glyphs(&text, p), width, ellipsis(p));
            out.push(Line::styled(text, role(Role::Attention, p)));
        }
    }
    out
}

/// The section's interior, `height` rows of `width` columns. `notes` (what blocks `w`,
/// what only warns) take what the table leaves; `outcome` (the last save) and the keys
/// always show.
pub(crate) fn lines(
    app: &App,
    s: &SettingsScreen,
    (width, height): (u16, u16),
    keys: bool,
    notes: Vec<Line<'static>>,
    outcome: Vec<Line<'static>>,
) -> Vec<Line<'static>> {
    let p = app.palette();
    let (w, h) = (usize::from(width), usize::from(height));
    let m = &s.models;
    let rows = m.rows(&app.catalogs);
    let table: Vec<Line<'static>> = (rows.iter().enumerate())
        .map(|(i, r)| row_line(r, i == m.selected, keys, w, p))
        .collect();
    let outcome: Vec<Line<'static>> = outcome.into_iter().take(h / 3).collect();
    // The scope line, a blank, the header; the blank under the table; the outcome and
    // the keys.
    let fixed = 3 + 1 + outcome.len() + 1;
    let room = h.saturating_sub(fixed);
    let shown = kit::window(table, m.selected, room.max(1), p);
    let mut out = vec![scope_line(s, w, p), Line::default(), header(w, p)];
    out.extend(shown);
    out.push(Line::default());
    let mut below = warning_lines(&rows, w, p);
    below.extend(notes);
    let left = h.saturating_sub(out.len() + outcome.len() + 1);
    out.extend(capped(below, left, p));
    out.extend(std::iter::repeat_n(
        Line::default(),
        h.saturating_sub(out.len() + outcome.len() + 1),
    ));
    out.extend(outcome);
    out.push(keys_line(w, p));
    out.truncate(h);
    out
}

/// `lines` cut to `rows`, what is cut marked on the last row.
fn capped(mut lines: Vec<Line<'static>>, rows: usize, p: Palette) -> Vec<Line<'static>> {
    if lines.len() <= rows {
        return lines;
    }
    let hidden = lines.len() - rows + 1;
    lines.truncate(rows.saturating_sub(1));
    if rows > 0 {
        let (_, more) = kit::scroll_marks(0, hidden, p.ascii);
        lines.push(Line::styled(more.unwrap_or_default(), role(Role::Muted, p)));
    }
    lines
}

#[cfg(test)]
#[path = "models_table_tests.rs"]
mod tests;
