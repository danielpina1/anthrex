//! Milestone 9.0.6 §1.2–1.3: the action menu, its confirmation page and the moved-base
//! page, on the kit's dialog grammar (decisions 5, 12, 14, 18). Every string an agent,
//! git or the daemon wrote passes `safe_text` (the kit's widgets, or `one_line` here).
//! Pure: rendering reads `&App`.

use crate::actions_request::{ActionTarget, short_id};
use crate::app::App;
use crate::app::actions::{ActionFlow, ActionStep, ConfirmPage, MovedBasePage};
use crate::safe_text::one_line;
use crate::theme::{Glyph, Palette, Role, glyph, role};
use crate::ui::kit::{self, Hint};
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::Modifier;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Clear, Paragraph};
use unicode_width::UnicodeWidthStr;

fn hint(key: &str, word: &str, priority: u8) -> Hint {
    Hint {
        key: key.to_string(),
        word: word.to_string(),
        priority,
    }
}

fn dot(p: Palette) -> &'static str {
    if p.ascii { " - " } else { " · " }
}

fn ellipsis(p: Palette) -> &'static str {
    if p.ascii { "..." } else { "…" }
}

/// The interior's text width for a dialog in `area`.
fn inner_width(area: Rect) -> u16 {
    area.width.min(kit::DIALOG_MAX).saturating_sub(4)
}

/// `text`, sanitised, wrapped at `min(width, WRAP)` columns, each line in `style`.
fn wrapped(text: &str, width: u16, style: ratatui::style::Style) -> Vec<Line<'static>> {
    let width = usize::from(width.min(kit::WRAP)).max(1);
    kit::wrap_words(&one_line(text), width)
        .into_iter()
        .map(|l| Line::styled(l, style))
        .collect()
}

/// Decision 12's title: `<run name>`, `<run name> › stage <n>`, `<run name> › <t> <title>`.
fn title(app: &App, flow: &ActionFlow, width: u16, p: Palette) -> String {
    let sep = glyph(Glyph::Separator, p.ascii);
    let suffix = match &flow.target {
        ActionTarget::Run => String::new(),
        ActionTarget::Stage(n) => format!(" {sep} stage {n}"),
        ActionTarget::Task(id) => {
            let title = app
                .runs
                .runs
                .iter()
                .find(|r| r.run_id == flow.run_id)
                .and_then(|r| r.tasks.iter().find(|t| t.id == *id))
                .map(|t| t.title.as_str())
                .unwrap_or("");
            one_line(&format!(" {sep} {id} {title}"))
                .trim_end()
                .to_string()
        }
    };
    let room = width.saturating_sub(suffix.width() as u16);
    let name = kit::run_name_in(&flow.goal, &flow.run_id, room.max(8), p);
    kit::cut(&format!("{name}{suffix}"), usize::from(width), ellipsis(p))
}

/// The menu's rows: the entries (scrolled to keep the selection in `rows`), or `not
/// connected`.
fn menu_body(
    app: &App,
    flow: &ActionFlow,
    width: u16,
    rows: u16,
    p: Palette,
) -> Vec<Line<'static>> {
    let muted = role(Role::Muted, p);
    let hints_line = |hints: &[Hint]| kit::hints_joined(width, hints, dot(p), p);
    if !app.connected() {
        return vec![
            Line::styled("not connected", muted),
            Line::raw(""),
            hints_line(&[hint("esc", "close", 1)]),
        ];
    }
    let mut body = Vec::new();
    if flow.items.is_empty() {
        body.push(Line::styled("no actions", muted));
    }
    let shown = usize::from(rows.saturating_sub(2)).max(1);
    let top = (flow.selected + 1).saturating_sub(shown);
    for (i, item) in flow.items.iter().enumerate().skip(top).take(shown) {
        let selected = i == flow.selected;
        let mark = if selected {
            glyph(Glyph::Selection, p.ascii)
        } else {
            " "
        };
        let label = one_line(&item.label);
        let line = match &item.refused_why {
            Some(why) => {
                let text = format!("{mark} {label}  {}", one_line(why));
                Line::styled(kit::cut(&text, usize::from(width), ellipsis(p)), muted)
            }
            None => {
                let mut style = if item.destructive {
                    role(Role::Failed, p)
                } else {
                    ratatui::style::Style::default()
                };
                if selected {
                    style = style.add_modifier(Modifier::BOLD);
                }
                let text = kit::cut(&label, usize::from(width.saturating_sub(2)), ellipsis(p));
                Line::from(vec![
                    Span::styled(format!("{mark} "), role(Role::Accent, p)),
                    Span::styled(text, style),
                ])
            }
        };
        body.push(line);
    }
    body.push(Line::raw(""));
    body.push(hints_line(&[
        hint("⏎", "choose", 9),
        hint("j/k", "move", 5),
        hint("esc", "close", 1),
    ]));
    body
}

/// The confirm or moved-base page's verb hint: `y` for a destructive-grade action,
/// both drawn in `Failed` when it is destructive.
fn verb_hints(verb: &str, key: &str, failed: bool, width: u16, p: Palette) -> Line<'static> {
    let mut line = kit::hints_joined(
        width,
        &[hint(key, verb, 9), hint("esc", "back", 1)],
        dot(p),
        p,
    );
    if failed {
        let style = role(Role::Failed, p);
        let verb = one_line(verb);
        for span in line.spans.iter_mut() {
            if span.content == key || span.content == verb.as_str() {
                span.style = style;
            }
        }
    }
    line
}

/// Decision 14: the effect, the details, a refusal the snapshot brought, the hints.
fn confirm_body(page: &ConfirmPage, width: u16, p: Palette) -> Vec<Line<'static>> {
    let mut body = wrapped(&page.info.effect, width, ratatui::style::Style::default());
    if !page.details.is_empty() {
        body.push(Line::raw(""));
        body.extend(kit::labelled_rows(&page.details, width.min(kit::WRAP), p));
    }
    if let Some(why) = &page.info.refused_why {
        body.push(Line::raw(""));
        body.extend(wrapped(why, width, role(Role::Failed, p)));
    }
    body.push(Line::raw(""));
    let key = if page.y_only() { "y" } else { "⏎" };
    let destructive = page.info.destructive || page.info.kind.destructive();
    body.push(verb_hints(&page.info.label, key, destructive, width, p));
    body
}

/// Decision 18: the moved base, its commits as many as fit `rows`, the short-id box.
fn moved_base_body(
    flow: &ActionFlow,
    page: &MovedBasePage,
    base: &str,
    width: u16,
    rows: u16,
    p: Palette,
) -> Vec<Line<'static>> {
    let seven = |sha: &str| sha.chars().take(7).collect::<String>();
    let arrow = if p.ascii { "->" } else { "→" };
    let total = page.moved.total as usize;
    let unit = if total == 1 { "commit" } else { "commits" };
    let to7 = one_line(&seven(&page.moved.to));
    let head = format!(
        "{} moved {} {arrow} {to7} ({total} {unit})",
        one_line(base),
        seven(&page.moved.from)
    );
    let mut body = wrapped(&head, width, ratatui::style::Style::default());
    let short = one_line(short_id(&flow.run_id));
    let mut tail = vec![
        Line::raw(""),
        Line::raw(format!("type {short} to merge onto {to7}")),
        Line::from(vec![
            Span::styled(
                format!("{} ", glyph(Glyph::Separator, p.ascii)),
                role(Role::Accent, p),
            ),
            Span::raw(one_line(&page.typed)),
            Span::styled(if p.ascii { "_" } else { "█" }, role(Role::Accent, p)),
        ]),
    ];
    // A snapshot refused the accept meanwhile (decision 13): Enter only toasts it.
    if let Some(why) = &page.info.refused_why {
        tail.extend(wrapped(why, width, role(Role::Failed, p)));
    }
    if page.wrong {
        tail.push(Line::styled(
            format!("type {short} exactly"),
            role(Role::Failed, p),
        ));
    }
    tail.push(Line::raw(""));
    tail.push(verb_hints("accept", "⏎", false, width, p));
    // The commits take what is left, keeping a row for `… and <k> more`.
    let room = usize::from(rows).saturating_sub(body.len() + tail.len());
    let commits = &page.moved.commits;
    let fits = if commits.len() <= room {
        commits.len()
    } else {
        room.saturating_sub(1)
    };
    let muted = role(Role::Muted, p);
    for commit in commits.iter().take(fits) {
        let text = kit::cut(
            &one_line(commit),
            usize::from(width.saturating_sub(2)),
            ellipsis(p),
        );
        body.push(Line::styled(format!("  {text}"), muted));
    }
    let more = total.saturating_sub(fits);
    if more > 0 && room > fits {
        body.push(Line::styled(
            format!("  {} and {more} more", ellipsis(p)),
            muted,
        ));
    }
    body.extend(tail);
    body
}

/// Draws the open `flow` over `area`.
pub fn render(frame: &mut Frame, app: &App, flow: &ActionFlow, area: Rect) {
    let p = app.palette();
    let width = inner_width(area);
    let rows = area.height.saturating_sub(2);
    let (title, destructive, body) = match &flow.step {
        ActionStep::Menu => (
            title(app, flow, width.saturating_sub(2), p),
            false,
            menu_body(app, flow, width, rows, p),
        ),
        ActionStep::Form(form) => (
            crate::ui::action_forms::title(form),
            false,
            crate::ui::action_forms::body(form, width, p),
        ),
        ActionStep::Confirm(page) => (
            page.info.label.clone(),
            page.info.destructive || page.info.kind.destructive(),
            confirm_body(page, width, p),
        ),
        ActionStep::MovedBase(page) => {
            let base = app
                .runs
                .runs
                .iter()
                .find(|r| r.run_id == flow.run_id)
                .map(|r| r.base_branch.clone())
                .unwrap_or_default();
            let body = moved_base_body(flow, page, &base, width, rows, p);
            (page.info.label.clone(), false, body)
        }
    };
    let rect = kit::dialog_area(area, body.len() as u16);
    if rect.width == 0 || rect.height == 0 {
        return;
    }
    frame.render_widget(Clear, rect);
    frame.render_widget(
        Paragraph::new(body).block(kit::dialog_frame(&title, destructive, p)),
        rect,
    );
}

#[cfg(test)]
#[path = "action_menu_tests.rs"]
mod tests;
