//! Milestone 9.0.6 decision 36: the Settings screen, drawn over the body. A frame titled
//! `settings · <config path>`, the section row, the section's lines (scrolled to keep the
//! selected row in view, cuts marked), then what blocks `w` (`Failed`), what only warns
//! (`Attention`) and the last save's outcome. A dialog (the custom model, the discard
//! question, or any modal over the screen) mutes the screen's border (decision 5). Every
//! model name, note, path and daemon problem passes `safe_text` here or in the kit.
//! Pure: `&App` in.

use super::profile::{frame_block, window};
use crate::app::App;
use crate::app::settings_screen::{
    DISCARD_ASK, SAVED, SaveOutcome, SettingsPage, SettingsScreen, SettingsSection,
    hard_stop_calls, hard_stop_minutes, strength_name,
};
use crate::safe_text::one_line;
use crate::theme::{Glyph, Palette, Role, glyph, role};
use crate::ui::kit::{self, Hint, cut, wrap_words};
use proto::Runtime;
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

fn pad(text: &str, width: usize) -> String {
    format!("{text}{}", " ".repeat(width.saturating_sub(text.width())))
}

/// The status bar's hints while the screen has the keys (decision 6).
pub(crate) fn hints(s: &SettingsScreen, ascii: bool) -> Vec<Hint> {
    if s.page.is_some() || !s.loaded {
        return vec![hint("esc", "back", 9)];
    }
    let arrows = if ascii { "left/right" } else { "←/→" };
    let mut out = match s.section.runtime() {
        Some(runtime) => {
            let rows = s.rows(runtime);
            match rows.get(s.selected) {
                None => vec![hint("⏎", "add a model", 7)],
                Some(row) if row.custom => {
                    vec![hint("space", "toggle", 7), hint(arrows, "strength", 6)]
                }
                Some(_) => vec![hint("space", "toggle", 7)],
            }
        }
        None if s.section == SettingsSection::Orchestrator => vec![hint(arrows, "change", 7)],
        None => vec![hint("0-9", "edit", 7)],
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

/// The selection bar and its space, or two spaces.
fn sel(on: bool, p: Palette) -> Span<'static> {
    let text = if on {
        format!("{} ", glyph(Glyph::Selection, p.ascii))
    } else {
        "  ".to_string()
    };
    Span::styled(text, role(Role::Accent, p))
}

fn default_span(on: bool, p: Palette) -> Option<Span<'static>> {
    on.then(|| Span::styled(format!("  {DEFAULT_MARK}"), role(Role::Muted, p)))
}

/// A model table: its head, one row per model, then `custom…`; and the selected line.
fn model_lines(
    s: &SettingsScreen,
    runtime: Runtime,
    width: usize,
    p: Palette,
) -> (Vec<Line<'static>>, usize) {
    let bold = Style::default().add_modifier(Modifier::BOLD);
    let mut head = vec![Span::styled("enabled models", bold)];
    head.extend(default_span(s.is_default(key::MODELS), p));
    let mut out = vec![Line::from(head)];
    let rows = s.rows(runtime);
    let label = |m: &str, l: &str| {
        if m.is_empty() && runtime == Runtime::Codex {
            "Codex default".to_string()
        } else {
            one_line(l)
        }
    };
    let name_w = rows
        .iter()
        .map(|r| label(&r.entry.model, &r.label).width())
        .max()
        .unwrap_or(0)
        .max(14)
        .min(width.saturating_sub(24).max(8));
    for (i, row) in rows.iter().enumerate() {
        let name = cut(&label(&row.entry.model, &row.label), name_w, ellipsis(p));
        let strength = strength_name(row.entry.strength);
        let mut spans = vec![
            sel(i == s.selected, p),
            Span::raw(if row.enabled { "[x] " } else { "[ ] " }),
            Span::raw(pad(&name, name_w + 2)),
        ];
        if row.custom {
            spans.push(Span::raw(kit::choice_in(strength, p)));
            spans.push(Span::styled("  custom", role(Role::Muted, p)));
        } else {
            spans.push(Span::raw(strength.to_string()));
        }
        if !row.entry.note.is_empty() {
            let used: usize = spans.iter().map(|s| s.content.width()).sum();
            let room = width.saturating_sub(used + 2);
            let note = cut(&one_line(&row.entry.note), room, ellipsis(p));
            spans.push(Span::styled(format!("  {note}"), role(Role::Muted, p)));
        }
        out.push(Line::from(spans));
    }
    out.push(Line::from(vec![
        sel(s.selected == rows.len(), p),
        Span::raw(format!("custom{}", ellipsis(p))),
    ]));
    (out, s.selected + 1)
}

/// `runtime ‹ configured ›` and `model ‹ default ›`.
fn orchestrator_lines(s: &SettingsScreen, p: Palette) -> (Vec<Line<'static>>, usize) {
    let runtime = s.runtime.map_or("configured", |r| r.label());
    let model = if s.model.is_empty() {
        "default".to_string()
    } else {
        one_line(&s.model)
    };
    let row = |i: usize, label: &str, value: &str, default: bool| {
        let mut spans = vec![
            sel(s.selected == i, p),
            Span::styled(pad(label, 9), role(Role::Muted, p)),
            Span::raw(kit::choice_in(value, p)),
        ];
        spans.extend(default_span(default, p));
        Line::from(spans)
    };
    let lines = vec![
        row(0, "runtime", runtime, s.is_default(key::AGENT_RUNTIME)),
        row(1, "model", &model, s.is_default(key::AGENT_MODEL)),
    ];
    (lines, s.selected)
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
fn limit_lines(s: &SettingsScreen, p: Palette) -> (Vec<Line<'static>>, usize) {
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
    let mut out = Vec::new();
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
            sel(i == s.selected, p),
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
    }
    (out, s.selected)
}

/// The section's lines and the selected line's index; `loading…` before the first doc.
fn section_lines(s: &SettingsScreen, width: u16, p: Palette) -> (Vec<Line<'static>>, usize) {
    if !s.loaded {
        let text = format!("loading{}", ellipsis(p));
        return (vec![Line::styled(text, role(Role::Muted, p))], 0);
    }
    match s.section.runtime() {
        Some(runtime) => model_lines(s, runtime, usize::from(width), p),
        None if s.section == SettingsSection::Orchestrator => orchestrator_lines(s, p),
        None => limit_lines(s, p),
    }
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

/// What blocks `w`, what only warns, and the last save's outcome.
fn footer(app: &App, s: &SettingsScreen, width: u16, p: Palette) -> Vec<Line<'static>> {
    let w = usize::from(width).max(1);
    let mut out = Vec::new();
    for problem in s.problems() {
        out.extend(marked(Glyph::Failed, &problem, w, Role::Failed, p));
    }
    for warning in s.warnings() {
        out.extend(marked(Glyph::Warning, &warning, w, Role::Attention, p));
    }
    if app.settings_saving() {
        let text = format!("saving{}", ellipsis(p));
        out.push(Line::styled(text, role(Role::Working, p)));
    } else {
        match &s.outcome {
            Some(SaveOutcome::Saved) => {
                let text = SAVED.replace('·', dot(p));
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
            None => {}
        }
    }
    out
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
    out.extend(section_lines(s, width, p).0);
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
    let keys_here = s.page.is_none() && app.modal.is_none();
    let block = frame_block(title, keys_here, p);
    let inner = block.inner(area);
    frame.render_widget(Clear, area);
    frame.render_widget(block, area);
    if inner.width == 0 || inner.height == 0 {
        return;
    }
    let height = usize::from(inner.height);
    // The footer may take up to half the screen; the section keeps the rest.
    let foot = capped(footer(app, s, inner.width, p), height / 2, p);
    let (lines, at) = section_lines(s, inner.width, p);
    let rows = height.saturating_sub(2 + foot.len());
    let mut all = vec![section_row(s, p), Line::default()];
    all.extend(window(lines, at, rows, p));
    if !foot.is_empty() {
        all.extend(std::iter::repeat_n(
            Line::default(),
            height.saturating_sub(all.len() + foot.len()),
        ));
    }
    all.extend(foot);
    frame.render_widget(Paragraph::new(all), inner);
    if let Some(page) = &s.page {
        render_page(frame, page, area, p);
    }
}

/// A page's title, whether it is destructive, its body and its hint line.
fn page_parts(
    page: &SettingsPage,
    width: u16,
    p: Palette,
) -> (String, bool, Vec<Line<'static>>, Line<'static>) {
    let hints = |list: &[(&str, &str)]| {
        let list: Vec<Hint> = list.iter().map(|(k, v)| hint(k, v, 5)).collect();
        kit::hints_joined(width, &list, &format!(" {} ", dot(p)), p)
    };
    match page {
        SettingsPage::Discard => {
            let w = usize::from(width.min(kit::WRAP)).max(1);
            let body = wrap_words(DISCARD_ASK, w)
                .into_iter()
                .map(Line::raw)
                .collect();
            let h = hints(&[("y", "discard"), ("esc", "back")]);
            ("discard changes".into(), true, body, h)
        }
        SettingsPage::Custom(c) => {
            let label = |text: &str, on: bool| {
                vec![
                    sel(on, p),
                    Span::styled(pad(text, 10), role(Role::Muted, p)),
                ]
            };
            let inner = width.saturating_sub(12);
            let mut line = kit::text_area_focus(&c.model, 1, inner, !c.on_strength, p).remove(0);
            let mut spans = label("model", !c.on_strength);
            spans.append(&mut line.spans);
            let mut strength = label("strength", c.on_strength);
            strength.push(Span::raw(kit::choice_in(strength_name(c.strength), p)));
            let mut body = vec![Line::from(spans), Line::from(strength)];
            if let Some(error) = &c.error {
                body.push(Line::styled(one_line(error), role(Role::Failed, p)));
            }
            let h = hints(&[("⏎", "add"), ("tab", "next"), ("esc", "cancel")]);
            (
                format!("custom {} model", c.runtime.label()),
                false,
                body,
                h,
            )
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
