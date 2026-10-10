//! The Profile screen's pages, drawn as kit dialogs over it (9.0.6 decision 5), as
//! milestone 9.10 decision 30 words them: the Detect toggles in plain words, the
//! Discard and Unset pages, the raw-text page (the file shown exactly, sanitised,
//! scrolled), a row's page, and the editors. Split from `ui/profile.rs` by responsibility (`AGENTS.md` hard rule 8).

use super::{hint, listed, pad, plain, tail_lines};
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

/// The editor's list area width in a terminal `width` wide, which its Up and Down
/// move by; a one-line value is 7 columns narrower (milestone 9.0.7 decision 35).
pub(crate) fn list_width(width: u16) -> u16 {
    width.min(kit::DIALOG_MAX).saturating_sub(4)
}

/// A page's title, whether it is destructive, its body and its hint line.
fn page_parts(
    s: &ProfileScreen,
    page: &ProfilePage,
    width: u16,
    keys: bool,
    p: Palette,
) -> (String, bool, Vec<Line<'static>>, Line<'static>) {
    let w = usize::from(width.min(kit::WRAP)).max(1);
    let wrap = |text: &str| -> Vec<Line<'static>> {
        wrap_words(&plain(text, p), w)
            .into_iter()
            .map(Line::raw)
            .collect()
    };
    let hints = |list: &[(&str, &str)]| {
        let list: Vec<Hint> = list.iter().map(|(k, v)| hint(k, v, 5)).collect();
        kit::hints_joined(width, &list, crate::theme::dot_sep(p), p)
    };
    match page {
        ProfilePage::Detect {
            trust_project,
            unconfined_checks,
            focus,
        } => {
            // Decision 30: each toggle in plain words, what it does under it.
            let toggle = |i: usize, label: &str, what: &str, on: bool| {
                let mark = if *focus == i {
                    glyph(Glyph::Selection, p.ascii)
                } else {
                    " "
                };
                let mut lines = vec![Line::from(vec![
                    Span::styled(format!("{mark} "), role(kit::bar_role(keys), p)),
                    Span::raw(pad(label, 34)),
                    Span::raw(kit::choice_in(if on { "on" } else { "off" }, p)),
                ])];
                for part in wrap_words(what, w.saturating_sub(2).max(1)) {
                    lines.push(Line::styled(format!("  {part}"), role(Role::Muted, p)));
                }
                lines
            };
            let mut body = toggle(
                0,
                "Trust this repo's agent settings",
                "let agents use the settings files this repo tracks, like .claude/ and .mcp.json",
                *trust_project,
            );
            body.extend(toggle(
                1,
                "Run checks outside the sandbox",
                "only needed on systems where anthrex can't confine commands",
                *unconfined_checks,
            ));
            body.push(Line::default());
            body.extend(
                wrap("a real agent will read the repository")
                    .into_iter()
                    .map(|l| l.style(role(Role::Attention, p))),
            );
            let h = hints(&[
                ("y", "start"),
                ("space", "toggle"),
                ("tab", "next"),
                ("esc", "cancel"),
            ]);
            (s.detect_title().into(), false, body, h)
        }
        ProfilePage::Discard => {
            // Decision 37 (principle 9): a destructive page's key and word in `Failed`,
            // as `ui/action_menu.rs::verb_hints` and the Settings discard page draw them.
            let h = kit::destructive(
                hints(&[("y", "discard"), ("esc", "back")]),
                "y",
                "discard",
                p,
            );
            ("discard proposal".into(), true, wrap(&s.discard_text()), h)
        }
        ProfilePage::Unset { key, .. } => (
            format!("unset {key}"),
            false,
            wrap(&format!(
                "the stored profile without {} becomes a proposal",
                one_line(key)
            )),
            hints(&[("⏎", "unset"), ("esc", "back")]),
        ),
        ProfilePage::RawText { text, .. } => {
            let body = multi_line(text)
                .split('\n')
                .flat_map(|line| hard_wrap(&one_line(line), w))
                .map(Line::raw)
                .collect();
            let h = hints(&[("j/k", "scroll"), ("esc", "back")]);
            ("profile file".into(), false, body, h)
        }
        ProfilePage::Row { key, .. } => {
            // Decision 30: the key dimmed, the value (a list one item per line), the
            // hint, the check line and, after a ✗, its output.
            let row = s.rows().into_iter().find(|r| r.key == *key);
            let shown = listed(s);
            let mut body = vec![Line::styled(plain(key, p), role(Role::Muted, p))];
            let value = match shown.and_then(|sh| sh.profile.as_ref()) {
                Some(profile) if key == "delivery.mode" => {
                    crate::profile_words::delivery_text(profile.delivery.as_ref())
                }
                Some(profile) => crate::profile_view::edit_text(profile, key),
                None => String::new(),
            };
            if value.is_empty() {
                body.push(Line::styled(plain("—", p), role(Role::Muted, p)));
            }
            for line in multi_line(&value).split('\n').filter(|l| !l.is_empty()) {
                body.extend(wrap(line));
            }
            body.extend(
                wrap(crate::profile_words::hint(key))
                    .into_iter()
                    .map(|l| l.style(role(Role::Muted, p))),
            );
            let check = row.and_then(|r| r.check);
            let tail = match (s.edit_of(key), &check) {
                (Some(proto::RowEditState::Failed { reason, tail, .. }), _) => {
                    // Final review C-I1: the value that failed, the one `s` stores.
                    let tried = s.row_edit().and_then(|e| e.value.as_deref());
                    let tried = crate::profile_view::tried_value(key, tried);
                    body.extend(
                        wrap(&format!("your edit: {tried}"))
                            .into_iter()
                            .map(|l| l.style(role(Role::Failed, p))),
                    );
                    body.extend(
                        wrap(&format!("couldn't verify: {reason}"))
                            .into_iter()
                            .map(|l| l.style(role(Role::Failed, p))),
                    );
                    tail.clone()
                }
                (Some(proto::RowEditState::Verifying), _) => {
                    body.push(Line::styled(plain("checking…", p), role(Role::Working, p)));
                    String::new()
                }
                (None, Some(c)) => {
                    body.push(Line::raw(crate::profile_view::check_cell(c, p.ascii)));
                    c.tail.clone()
                }
                _ => String::new(),
            };
            if !tail.is_empty() {
                body.extend(tail_lines(&tail, 2, w, p));
            }
            let h = hints(&[("j/k", "scroll"), ("esc", "back")]);
            (plain(&crate::profile_words::label(key), p), false, body, h)
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
            if let Some(error) = &editor.error {
                body.push(Line::default());
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
    let keys = app.key_region() == crate::app::region::KeyRegion::Screen;
    let (_, _, mut body, hints) = page_parts(s, page, width, keys, app.palette());
    body.push(Line::default());
    body.push(hints);
    body
}

pub(super) fn render_page(
    frame: &mut Frame,
    s: &ProfileScreen,
    page: &ProfilePage,
    area: Rect,
    keys: bool,
    p: Palette,
) {
    let width = area.width.min(kit::DIALOG_MAX).saturating_sub(4);
    let (title, destructive, body, hints) = page_parts(s, page, width, keys, p);
    // The page fits the area: a long TOML scrolls inside it.
    let room = usize::from(area.height.saturating_sub(4));
    let body = match page {
        ProfilePage::RawText { scroll, .. } | ProfilePage::Row { scroll, .. } => {
            scrolled(body, *scroll, room, p)
        }
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
