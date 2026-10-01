//! The Profile screen's pages, drawn as kit dialogs over it (decision 5): the Detect
//! toggles, the Reject, Unset and Confirm pages (the TOML shown exactly, scrolled), and
//! the editors. Split from `ui/profile.rs` by responsibility (`AGENTS.md` hard rule 8).

use super::{dot, hint, pad, store_line};
use crate::app::profile_screen::{EditorField, ProfilePage, ProfileScreen};
use crate::safe_text::{multi_line, one_line};
use crate::theme::{Glyph, Palette, Role, glyph, role};
use crate::ui::kit::{self, Hint, wrap_words};
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::Modifier;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Clear, Paragraph};
use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthStr;

/// The rows a list editor shows.
const LIST_ROWS: u16 = 6;

/// `lines` from line `top` (clamped) in `rows`, what is cut marked.
fn scrolled(lines: Vec<Line<'static>>, top: usize, rows: usize, p: Palette) -> Vec<Line<'static>> {
    if lines.len() <= rows || rows < 3 {
        return lines.into_iter().take(rows).collect();
    }
    let room = rows - 2;
    let top = top.min(lines.len() - room);
    let (up, down) = kit::scroll_marks(top, lines.len() - top - room, p.ascii);
    let muted = role(Role::Muted, p);
    let mut out = vec![Line::styled(up.unwrap_or_default(), muted)];
    out.extend(lines.into_iter().skip(top).take(room));
    out.push(Line::styled(down.unwrap_or_default(), muted));
    out
}

/// A line cut into `width`-column pieces, every space kept (the TOML is shown exactly).
fn hard_wrap(line: &str, width: usize) -> Vec<String> {
    let mut out = vec![String::new()];
    let mut used = 0;
    for g in line.graphemes(true) {
        if used + g.width() > width.max(1) && used > 0 {
            out.push(String::new());
            used = 0;
        }
        if let Some(l) = out.last_mut() {
            l.push_str(g);
        }
        used += g.width();
    }
    out
}

/// A page's title, whether it is destructive, its body and its hint line.
fn page_parts(
    s: &ProfileScreen,
    page: &ProfilePage,
    width: u16,
    p: Palette,
) -> (String, bool, Vec<Line<'static>>, Line<'static>) {
    let w = usize::from(width.min(kit::WRAP)).max(1);
    let wrap = |text: &str| -> Vec<Line<'static>> {
        wrap_words(&one_line(text), w)
            .into_iter()
            .map(Line::raw)
            .collect()
    };
    let hints = |list: &[(&str, &str)]| {
        let list: Vec<Hint> = list.iter().map(|(k, v)| hint(k, v, 5)).collect();
        kit::hints_joined(width, &list, &format!(" {} ", dot(p)), p)
    };
    let dir = s.dir.display().to_string();
    match page {
        ProfilePage::Detect {
            trust_project,
            unconfined_checks,
            focus,
        } => {
            let toggle = |i: usize, label: &str, on: bool| {
                let mark = if *focus == i {
                    glyph(Glyph::Selection, p.ascii)
                } else {
                    " "
                };
                Line::from(vec![
                    Span::styled(format!("{mark} "), role(Role::Accent, p)),
                    Span::styled(pad(label, 19), role(Role::Muted, p)),
                    Span::raw(kit::choice_in(if on { "on" } else { "off" }, p)),
                ])
            };
            let mut body = vec![
                toggle(0, "trust project", *trust_project),
                toggle(1, "unconfined checks", *unconfined_checks),
                Line::default(),
            ];
            body.extend(
                wrap("a real agent will read the repository")
                    .into_iter()
                    .map(|l| l.style(role(Role::Attention, p))),
            );
            let h = hints(&[
                ("y", "detect"),
                ("space", "toggle"),
                ("tab", "next"),
                ("esc", "cancel"),
            ]);
            ("detect".into(), false, body, h)
        }
        ProfilePage::Reject => (
            "reject proposal".into(),
            true,
            wrap(&format!(
                "the proposal for {dir} is deleted; a running scout or verification stops"
            )),
            hints(&[("y", "reject"), ("esc", "back")]),
        ),
        ProfilePage::Unset { key } => (
            format!("unset {key}"),
            false,
            wrap(&format!(
                "the stored profile without {} becomes a proposal",
                one_line(key)
            )),
            hints(&[("⏎", "unset"), ("esc", "back")]),
        ),
        ProfilePage::Confirm { toml, .. } => {
            let body = multi_line(toml)
                .split('\n')
                .flat_map(|line| hard_wrap(&one_line(line), w))
                .map(Line::raw)
                .collect();
            let h = hints(&[("y", "store"), ("j/k", "scroll"), ("esc", "back")]);
            ("confirm profile".into(), false, body, h)
        }
        ProfilePage::Edit(editor) => {
            let label = |text: &str| Span::styled(pad(text, 7), role(Role::Muted, p));
            let mut body: Vec<Line<'static>> = Vec::new();
            let mut list = false;
            let inner = width.saturating_sub(7);
            match &editor.field {
                EditorField::Line(area) => {
                    let mut line = kit::text_area(area, 1, inner, p).remove(0);
                    line.spans.insert(0, label("value"));
                    body.push(line);
                }
                EditorField::List(area) => {
                    list = true;
                    body.push(Line::styled("one per line", role(Role::Muted, p)));
                    body.extend(kit::text_area(area, LIST_ROWS, width, p));
                }
                EditorField::Digits(digits) => body.push(Line::from(vec![
                    label("value"),
                    Span::raw(digits.clone()),
                    Span::styled(
                        " ",
                        ratatui::style::Style::default().add_modifier(Modifier::REVERSED),
                    ),
                ])),
                EditorField::Choice { options, at } => body.push(Line::from(vec![
                    label("value"),
                    Span::raw(kit::choice_in(options.get(*at).copied().unwrap_or(""), p)),
                ])),
                EditorField::Env {
                    name,
                    value,
                    on_value,
                } => {
                    for (text, area, focused) in
                        [("name", name, !on_value), ("value", value, *on_value)]
                    {
                        let mut line = kit::text_area_focus(area, 1, inner, focused, p).remove(0);
                        line.spans.insert(0, label(text));
                        body.push(line);
                    }
                }
            }
            body.push(Line::default());
            body.push(store_line(s, p));
            if let Some(error) = &editor.error {
                body.extend(
                    wrap(error)
                        .into_iter()
                        .map(|l| l.style(role(Role::Failed, p))),
                );
            }
            let mut keys = vec![("⏎", "save")];
            if list {
                keys.push(("^J", "newline"));
            }
            if matches!(editor.field, EditorField::Env { .. }) {
                keys.push(("tab", "next"));
            }
            keys.push(("esc", "cancel"));
            (format!("edit {}", editor.key), false, body, hints(&keys))
        }
    }
}

/// A page's lines (its body, a blank, its hints), for `width` interior columns.
#[cfg(test)]
pub(crate) fn page_lines(
    app: &crate::app::App,
    s: &ProfileScreen,
    page: &ProfilePage,
    width: u16,
) -> Vec<Line<'static>> {
    let (_, _, mut body, hints) = page_parts(s, page, width, app.palette());
    body.push(Line::default());
    body.push(hints);
    body
}

pub(super) fn render_page(
    frame: &mut Frame,
    s: &ProfileScreen,
    page: &ProfilePage,
    area: Rect,
    p: Palette,
) {
    let width = area.width.min(kit::DIALOG_MAX).saturating_sub(4);
    let (title, destructive, body, hints) = page_parts(s, page, width, p);
    // The page fits the area: a long TOML scrolls inside it.
    let room = usize::from(area.height.saturating_sub(4));
    let body = match page {
        ProfilePage::Confirm { scroll, .. } => scrolled(body, *scroll, room, p),
        _ => body.into_iter().take(room).collect(),
    };
    let mut lines = body;
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
