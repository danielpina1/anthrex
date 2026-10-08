//! Milestone 9.0.6 decision 36: the Settings screen, drawn over the body. A frame titled
//! `settings · <config path>`, the section row, the section's lines (scrolled to keep the
//! selected row in view, cuts marked), then what blocks `w` (`Failed`), what only warns
//! (`Attention`) and the last save's outcome. A dialog (the custom model, the discard
//! question, or any modal over the screen) mutes the screen's border (decision 5). Every
//! model name, note, path and daemon problem passes `safe_text` here or in the kit.
//! Pure: `&App` in.

use crate::app::replies::NOT_SENT;
use crate::app::settings_screen::{
    DISCARD_ASK, LINK_LOST, MODELS_SAVED, SAVED, SaveOutcome, SettingsPage, SettingsScreen,
    SettingsSection, hard_stop_calls, hard_stop_minutes,
};
use crate::app::{App, region::KeyRegion};
use crate::safe_text::one_line;
use crate::theme::{Glyph, Palette, Role, dot, dot_sep, ellipsis, glyph, role};
use crate::ui::kit::{self, Hint, cut, wrap_words};
use crate::ui::{model_picker, models_table};
use proto::settings::{
    BUDGET_MIN, MAX_BOUNCES_RANGE, MAX_READERS_RANGE, MAX_WRITERS_RANGE, STALL_AFTER_SECS_RANGE,
    key,
};
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Clear, Paragraph};
use unicode_width::UnicodeWidthStr;

/// Interfaces "Settings default mark".
const DEFAULT_MARK: &str = "(default)";

fn hint(key: &str, word: &str, priority: u8) -> Hint {
    Hint {
        key: key.to_string(),
        word: word.to_string(),
        priority,
    }
}

fn pad(text: &str, width: usize) -> String {
    format!("{text}{}", " ".repeat(width.saturating_sub(text.width())))
}

/// The status bar's hints while the screen has the keys (decision 6).
pub(crate) fn hints(s: &SettingsScreen, ascii: bool) -> Vec<Hint> {
    if s.page.is_some() || !s.loaded {
        return vec![hint("esc", "back", 9)];
    }
    let arrows = if ascii { "left/right" } else { "←/→" };
    let mut out = match s.section {
        SettingsSection::Models if s.models.picker.is_some() => {
            return vec![
                hint("⏎", "select", 9),
                hint("r", "refresh", 7),
                hint("j/k", "move", 6),
                hint("esc", "cancel", 9),
            ];
        }
        SettingsSection::Models => vec![
            hint("⏎", "choose model", 7),
            hint("e", "effort", 6),
            hint("f", "if-it-struggles", 3),
            hint("x", "reset", 3),
            hint(arrows, "scope", 2),
        ],
        SettingsSection::Limits => vec![hint("0-9", "edit", 7)],
    };
    out.extend([
        hint("w", "save", 8),
        hint("j/k", "move", 4),
        hint("tab", "section", 5),
        hint("esc", "back", 9),
    ]);
    out
}

/// The section row: the current section in bold, the others muted.
fn section_row(s: &SettingsScreen, p: Palette) -> Line<'static> {
    let mut spans = Vec::new();
    for (i, section) in SettingsSection::ALL.iter().enumerate() {
        if i > 0 {
            spans.push(Span::raw("  "));
        }
        let style = if *section == s.section {
            Style::default().add_modifier(Modifier::BOLD | Modifier::UNDERLINED)
        } else {
            role(Role::Muted, p)
        };
        spans.push(Span::styled(section.name(), style));
    }
    Line::from(spans)
}

/// The selection bar and its space, or two spaces. The bar is the accent only while
/// the screen holds the keys (`keys`, decision 1), `Muted` under a modal.
fn sel(on: bool, keys: bool, p: Palette) -> Span<'static> {
    let text = if on {
        format!("{} ", glyph(Glyph::Selection, p.ascii))
    } else {
        "  ".to_string()
    };
    Span::styled(text, role(kit::bar_role(keys), p))
}

/// A limit's allowed range, from the ranges `config` itself reads.
fn range_of(k: &str) -> String {
    let between = |a: u64, b: u64| format!("{a} to {b}");
    match k {
        key::STALL_AFTER_SECS => between(
            *STALL_AFTER_SECS_RANGE.start(),
            *STALL_AFTER_SECS_RANGE.end(),
        ),
        key::MAX_WRITERS => between(
            (*MAX_WRITERS_RANGE.start()).into(),
            (*MAX_WRITERS_RANGE.end()).into(),
        ),
        key::MAX_READERS => between(
            (*MAX_READERS_RANGE.start()).into(),
            (*MAX_READERS_RANGE.end()).into(),
        ),
        key::MAX_BOUNCES => between(
            (*MAX_BOUNCES_RANGE.start()).into(),
            (*MAX_BOUNCES_RANGE.end()).into(),
        ),
        _ => format!("at least {BUDGET_MIN}"),
    }
}

/// `<label>  <value>  <range>`, budgets with their hard stop; a refused value in `Failed`.
fn limit_lines(s: &SettingsScreen, keys: bool, p: Palette) -> (Vec<Line<'static>>, usize) {
    let problems = s.problems();
    let mark_w = |k: &str| {
        if s.is_default(k) {
            1 + DEFAULT_MARK.width()
        } else {
            0
        }
    };
    // The value, a cursor cell, the default mark, two spaces.
    let value_w = s
        .limits
        .iter()
        .map(|f| f.text.width() + 1 + mark_w(f.key))
        .max()
        .unwrap_or(0)
        + 2;
    let (mut out, mut selected) = (Vec::new(), 0);
    for (i, f) in s.limits.iter().enumerate() {
        let refused = problems
            .iter()
            .any(|m| m.starts_with(&format!("{}:", f.key)));
        let style = if refused {
            role(Role::Failed, p)
        } else {
            Style::default()
        };
        let mut spans = vec![
            sel(i == s.selected, keys, p),
            Span::styled(pad(f.label, 18), role(Role::Muted, p)),
            Span::styled(f.text.clone(), style),
        ];
        let cursor = if i == s.selected {
            Style::default().add_modifier(Modifier::REVERSED)
        } else {
            Style::default()
        };
        spans.push(Span::styled(" ", cursor));
        let mut used = f.text.width() + 1;
        if s.is_default(f.key) {
            spans.push(Span::styled(
                format!(" {DEFAULT_MARK}"),
                role(Role::Muted, p),
            ));
            used += mark_w(f.key);
        }
        spans.push(Span::raw(" ".repeat(value_w - used)));
        let mut tail = pad(&range_of(f.key), 13);
        let n: u64 = f.text.parse().unwrap_or(0);
        if !refused && n > 0 {
            if f.key.ends_with("tool_calls") {
                tail.push_str(&hard_stop_calls(n));
            } else if f.key.ends_with("minutes") {
                tail.push_str(&hard_stop_minutes(n));
            }
        }
        spans.push(Span::styled(
            tail.trim_end().to_string(),
            role(Role::Muted, p),
        ));
        out.push(Line::from(spans));
        if i == s.selected {
            selected = out.len() - 1;
        }
        // Ruling RH-5: a class's refit, unless config sets it, on a line of its own under
        // the class's budget (its rows have no room left at 80 columns), in the value
        // column; it is not a row the keys select.
        let class = match f.key {
            key::BUDGET_S_MINUTES => s.refit_note("S"),
            key::BUDGET_M_MINUTES => s.refit_note("M"),
            _ => None,
        };
        if let Some(note) = class {
            // Past the selection mark (2) and the label (18).
            let text = format!("{}{note}", " ".repeat(2 + 18));
            out.push(Line::styled(text, role(Role::Muted, p)));
        }
    }
    (out, selected)
}

/// The `limits` section's lines and the selected line's index; `loading…` before the
/// first doc. The `models` section draws whole (`models_table::lines`).
fn section_lines(s: &SettingsScreen, keys: bool, p: Palette) -> (Vec<Line<'static>>, usize) {
    if !s.loaded {
        let text = format!("loading{}", ellipsis(p));
        return (vec![Line::styled(text, role(Role::Muted, p))], 0);
    }
    limit_lines(s, keys, p)
}

/// `glyph text`, wrapped under itself, in `r`.
fn marked(g: Glyph, text: &str, width: usize, r: Role, p: Palette) -> Vec<Line<'static>> {
    let lead = format!("{} ", glyph(g, p.ascii));
    let indent = " ".repeat(lead.width());
    wrap_words(&one_line(text), width.saturating_sub(lead.width()).max(1))
        .into_iter()
        .enumerate()
        .map(|(i, part)| {
            let head = if i == 0 { lead.clone() } else { indent.clone() };
            Line::styled(format!("{head}{part}"), role(r, p))
        })
        .collect()
}

/// What blocks `w` and what only warns.
fn notes(s: &SettingsScreen, width: u16, p: Palette) -> Vec<Line<'static>> {
    let w = usize::from(width).max(1);
    let mut out = Vec::new();
    for problem in s.problems() {
        out.extend(marked(Glyph::Failed, &problem, w, Role::Failed, p));
    }
    for warning in s.warnings() {
        out.extend(marked(Glyph::Warning, &warning, w, Role::Attention, p));
    }
    out
}

/// The last save's outcome, or `saving…` while it is awaited. On the `models` section
/// a save says so of the models (MR §5.1).
fn outcome(app: &App, s: &SettingsScreen, width: u16, p: Palette) -> Vec<Line<'static>> {
    let w = usize::from(width).max(1);
    let mut out = Vec::new();
    if app.settings_saving() {
        let text = format!("saving{}", ellipsis(p));
        out.push(Line::styled(text, role(Role::Working, p)));
        return out;
    }
    match &s.outcome {
        Some(SaveOutcome::Saved) => {
            let saved = match s.section {
                SettingsSection::Models => MODELS_SAVED,
                SettingsSection::Limits => SAVED,
            };
            let text = saved.replace('·', dot(p));
            out.push(Line::styled(
                cut(&text, w, ellipsis(p)),
                role(Role::Done, p),
            ));
        }
        Some(SaveOutcome::Refused(problems)) => {
            out.push(Line::styled("not saved", role(Role::Failed, p)));
            for problem in problems {
                out.extend(marked(Glyph::Failed, problem, w, Role::Failed, p));
            }
        }
        Some(SaveOutcome::LinkLost) => {
            out.push(Line::styled(LINK_LOST, role(Role::Failed, p)));
        }
        Some(SaveOutcome::NotSent) => {
            out.push(Line::styled(NOT_SENT, role(Role::Failed, p)));
        }
        None => {}
    }
    out
}

/// What blocks `w`, what only warns, and the last save's outcome.
fn footer(app: &App, s: &SettingsScreen, width: u16, p: Palette) -> Vec<Line<'static>> {
    let mut out = notes(s, width, p);
    out.extend(outcome(app, s, width, p));
    out
}

/// Whether the `models` section draws (it draws whole, `models_table::lines`).
fn models_shown(s: &SettingsScreen) -> bool {
    s.loaded && s.section == SettingsSection::Models
}

/// `lines` cut to at most `rows`, what is cut marked on the last row.
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

/// Everything the screen shows, unscrolled: the section row, its lines, the footer.
#[cfg(test)]
pub(crate) fn body_lines(app: &App, s: &SettingsScreen, width: u16) -> Vec<Line<'static>> {
    let p = app.palette();
    let mut out = vec![section_row(s, p)];
    let keys = app.key_region() == KeyRegion::Screen;
    if models_shown(s) {
        let (notes, outcome) = (notes(s, width, p), outcome(app, s, width, p));
        out.extend(models_table::lines(
            app,
            s,
            (width, 60),
            keys,
            notes,
            outcome,
        ));
        return out;
    }
    out.extend(section_lines(s, keys, p).0);
    out.extend(footer(app, s, width, p));
    out
}

/// Draws the screen over `area` (the body), and its page over that.
pub fn render(frame: &mut Frame, app: &App, s: &SettingsScreen, area: Rect) {
    let p = app.palette();
    let title = if s.path.is_empty() {
        "settings".to_string()
    } else {
        format!("settings {} {}", dot(p), one_line(&s.path))
    };
    // Decision 5: the one accented border is the dialog's while one is open.
    let bars = app.key_region() == KeyRegion::Screen;
    let picker = s.models.picker.as_ref().filter(|_| models_shown(s));
    let keys_here = s.page.is_none() && picker.is_none() && bars;
    let block = kit::screen_frame(&title, keys_here, p);
    let inner = block.inner(area);
    frame.render_widget(Clear, area);
    frame.render_widget(block, area);
    if inner.width == 0 || inner.height == 0 {
        return;
    }
    if models_shown(s) {
        let (notes, outcome) = (notes(s, inner.width, p), outcome(app, s, inner.width, p));
        let size = (inner.width, inner.height);
        let lines = models_table::lines(app, s, size, keys_here, notes, outcome);
        frame.render_widget(Paragraph::new(lines), inner);
        if let Some(picker) = picker {
            let age = app.catalogs.updated_age(app.ticked_at);
            model_picker::render(frame, picker, age, area, p);
        }
    } else {
        draw_limits(frame, app, s, inner, bars);
    }
    if let Some(page) = &s.page {
        render_page(frame, page, area, p);
    }
}

/// The `limits` section (or `loading…`) in the frame's interior.
fn draw_limits(frame: &mut Frame, app: &App, s: &SettingsScreen, inner: Rect, bars: bool) {
    let p = app.palette();
    let height = usize::from(inner.height);
    // The footer may take up to half the screen; the section keeps the rest.
    let foot = capped(footer(app, s, inner.width, p), height / 2, p);
    let (lines, at) = section_lines(s, bars, p);
    let rows = height.saturating_sub(2 + foot.len());
    let mut all = vec![section_row(s, p), Line::default()];
    all.extend(kit::window(lines, at, rows, p));
    if !foot.is_empty() {
        all.extend(std::iter::repeat_n(
            Line::default(),
            height.saturating_sub(all.len() + foot.len()),
        ));
    }
    all.extend(foot);
    frame.render_widget(Paragraph::new(all), inner);
}

/// A page's title, whether it is destructive, its body and its hint line.
fn page_parts(
    page: &SettingsPage,
    width: u16,
    p: Palette,
) -> (String, bool, Vec<Line<'static>>, Line<'static>) {
    let hints = |list: &[(&str, &str)]| {
        let list: Vec<Hint> = list.iter().map(|(k, v)| hint(k, v, 5)).collect();
        kit::hints_joined(width, &list, dot_sep(p), p)
    };
    match page {
        SettingsPage::Discard => {
            let w = usize::from(width.min(kit::WRAP)).max(1);
            let body = wrap_words(DISCARD_ASK, w)
                .into_iter()
                .map(Line::raw)
                .collect();
            // Decision 5: a destructive dialog's action word is in `Failed`.
            let h = kit::destructive(
                hints(&[("y", "discard"), ("esc", "back")]),
                "y",
                "discard",
                p,
            );
            ("discard changes".into(), true, body, h)
        }
    }
}

/// A page's lines (its body, a blank, its hints), for `width` interior columns.
#[cfg(test)]
pub(crate) fn page_lines(app: &App, page: &SettingsPage, width: u16) -> Vec<Line<'static>> {
    let (_, _, mut body, hints) = page_parts(page, width, app.palette());
    body.push(Line::default());
    body.push(hints);
    body
}

fn render_page(frame: &mut Frame, page: &SettingsPage, area: Rect, p: Palette) {
    let width = area.width.min(kit::DIALOG_MAX).saturating_sub(4);
    let (title, destructive, body, hints) = page_parts(page, width, p);
    let room = usize::from(area.height.saturating_sub(4));
    let mut lines: Vec<Line<'static>> = body.into_iter().take(room).collect();
    lines.push(Line::default());
    lines.push(hints);
    let rect = kit::dialog_area(area, lines.len() as u16);
    if rect.width == 0 || rect.height == 0 {
        return;
    }
    frame.render_widget(Clear, rect);
    frame.render_widget(
        Paragraph::new(lines).block(kit::dialog_frame(&title, destructive, p)),
        rect,
    );
}

#[cfg(test)]
#[path = "settings_tests.rs"]
pub(crate) mod tests;

#[cfg(test)]
#[path = "settings_tuning_tests.rs"]
mod tuning_tests;
