//! Milestone 9.0.7 decision 11: the Alerts view, drawn over the main pane while
//! `app.alerts_focus` is set (`C-b a`). A frame titled ` ⚑ Alerts ` with ` <i>/<n> `;
//! the list (`P<p> <glyph> <who>[ › <t>]`, the age flush right, the selection reversed
//! with the `▌` bar) left of a `│` rule from an interior of 60 columns, else on top of
//! a `─` rule; the selected alert's whole detail beside or below it, scrolled by
//! PgUp/PgDn. The alerts are `app::alerts`'s; every daemon string passes `safe_text`
//! here. Pure: rendering takes `&App`.

use super::alerts::{age_text, fitted, who_text};
use super::kit;
use crate::app::region::KeyRegion;
use crate::app::{Alert, App, alerts};
use crate::safe_text::one_line;
use crate::theme::{self, Glyph, Palette, Role, fold, glyph};
use crate::ui::tree_view::truncate_in;
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use unicode_width::UnicodeWidthStr;

/// The list sits left of the detail from an interior this wide (decision 11).
const SIDE_BY_SIDE: u16 = 60;

/// The view's parts inside its frame: the list, the rule, the detail (one column in
/// from the rule).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Areas {
    pub list: Rect,
    pub rule: Rect,
    pub detail: Rect,
    pub stacked: bool,
}

fn inset(r: Rect) -> Rect {
    Rect {
        x: r.x.saturating_add(1),
        y: r.y.saturating_add(1),
        width: r.width.saturating_sub(2),
        height: r.height.saturating_sub(2),
    }
}

/// Decision 11's layout of `main` for `n` alerts: at an interior of 60 columns or more
/// the list takes `clamp(interior × 2/5, 26, 44)` columns, then the `│` rule; narrower,
/// the list takes its rows (at most half the interior), then the `─` rule.
pub(crate) fn areas(main: Rect, n: usize) -> Areas {
    let inner = inset(main);
    if inner.width >= SIDE_BY_SIDE {
        let list_w = (inner.width * 2 / 5).clamp(26, 44);
        let rule_x = inner.x + list_w;
        return Areas {
            list: Rect {
                width: list_w,
                ..inner
            },
            rule: Rect {
                x: rule_x,
                width: 1,
                ..inner
            },
            detail: Rect {
                x: rule_x + 2,
                width: inner.width.saturating_sub(list_w + 2),
                ..inner
            },
            stacked: false,
        };
    }
    let n = u16::try_from(n).unwrap_or(u16::MAX);
    let list_h = n.min(inner.height / 2).max(1).min(inner.height);
    let rule_h = inner.height.saturating_sub(list_h).min(1);
    Areas {
        list: Rect {
            height: list_h,
            ..inner
        },
        rule: Rect {
            y: inner.y + list_h,
            height: rule_h,
            ..inner
        },
        detail: Rect {
            x: inner.x.saturating_add(1),
            y: inner.y + list_h + rule_h,
            width: inner.width.saturating_sub(1),
            height: inner.height - list_h - rule_h,
        },
        stacked: true,
    }
}

/// The selected alert's index: its key's position, else the last seen position.
fn selected_index(app: &App, all: &[Alert]) -> Option<usize> {
    let focus = app.alerts_focus.as_ref()?;
    if all.is_empty() {
        return None;
    }
    let found = focus
        .selected
        .as_ref()
        .and_then(|key| all.iter().position(|alert| alert.key == *key));
    Some(found.unwrap_or(focus.at.min(all.len() - 1)))
}

/// One list row at `width`: the `▌` bar (the accent) or a space, then `P<p> <glyph>
/// <who>[ › <t>]` in the priority's style and the age muted, flush right one column in
/// from the edge; the who is cut first. The selected row is reversed after the bar.
fn list_line(app: &App, alert: &Alert, selected: bool, width: u16) -> Line<'static> {
    let p = app.palette();
    let w = usize::from(width);
    if w == 0 {
        return Line::default();
    }
    let room = w.saturating_sub(2);
    let style = theme::alert_style(alert.priority, p);
    let head = format!(
        "P{} {} ",
        alert.priority,
        theme::alert_glyph(alert.priority, p.ascii)
    );
    let task = alert.task.as_deref().map_or_else(String::new, |t| {
        let t = fold(&one_line(t), p.ascii);
        format!(" {} {t}", glyph(Glyph::Separator, p.ascii))
    });
    let age = alert.age.map(age_text);
    let age_room = age.as_ref().map_or(0, |age| age.width() + 1);
    let who_room = room.saturating_sub(head.width() + task.width() + age_room);
    let who = who_text(&alert.who, who_room, p);
    let mut parts = vec![(head, style), (who, style), (task, style)];
    if let Some(age) = age {
        let used: usize = parts.iter().map(|(text, _)| text.width()).sum();
        let gap = room.saturating_sub(used + age.width()).max(1);
        parts.push((" ".repeat(gap), Style::default()));
        parts.push((age, theme::role(Role::Muted, p)));
    }
    let mut spans = fitted(parts, room, p.ascii).spans;
    let used: usize = spans.iter().map(|span| span.content.width()).sum();
    spans.push(Span::raw(" ".repeat(w.saturating_sub(1 + used))));
    let bar = if selected {
        glyph(Glyph::Selection, p.ascii)
    } else {
        " "
    };
    let mut out = vec![Span::styled(bar, theme::role(Role::Accent, p))];
    out.extend(spans.into_iter().map(|span| {
        if selected {
            let style = span.style.add_modifier(Modifier::REVERSED);
            Span::styled(span.content, style)
        } else {
            span
        }
    }));
    Line::from(out)
}

/// The last first row and a page, for PgUp/PgDn at `main`: the detail's rows by
/// [`detail_lines`], less its height but the row the `↑` mark takes.
pub(crate) fn detail_scroll(app: &App, main: Rect) -> Option<(u16, u16)> {
    let all = alerts(app);
    let alert = &all[selected_index(app, &all)?];
    let detail = areas(main, all.len()).detail;
    let n = detail_lines(app, alert, detail.width).len();
    let rows = usize::from(detail.height);
    let max = if n <= rows || rows == 0 {
        0
    } else {
        n - (rows - 1)
    };
    let page = detail.height.saturating_sub(2).max(1);
    Some((u16::try_from(max).unwrap_or(u16::MAX), page))
}

/// `lines` from `scroll` in `rows`, the cut marked: `↑ <k> more` on the first row once
/// scrolled, `↓ <k> more` on the last while more follows.
fn detail_window(
    lines: Vec<Line<'static>>,
    scroll: u16,
    rows: usize,
    p: Palette,
) -> Vec<Line<'static>> {
    let n = lines.len();
    if n <= rows {
        return lines;
    }
    let scroll = usize::from(scroll);
    if rows < 3 {
        return lines
            .into_iter()
            .skip(scroll.min(n - rows))
            .take(rows)
            .collect();
    }
    let top = scroll.min(n - (rows - 1));
    let room = if top == 0 || top + rows - 1 < n {
        rows - 1 - usize::from(top > 0)
    } else {
        n - top
    };
    let below = n - top - room;
    let (up, down) = kit::scroll_marks(top, below, p.ascii);
    let muted = theme::role(Role::Muted, p);
    let mut out: Vec<Line<'static>> = up.map(|m| Line::styled(m, muted)).into_iter().collect();
    out.extend(lines.into_iter().skip(top).take(room));
    out.extend(down.map(|m| Line::styled(m, muted)));
    out
}

/// ` ⚑ Alerts ` (the glyph in `Attention`), or ` Alerts ` with none.
fn title(n: usize, p: Palette) -> Line<'static> {
    if n == 0 {
        return Line::from("Alerts");
    }
    Line::from(vec![
        Span::styled(
            glyph(Glyph::NeedsYou, p.ascii),
            theme::role(Role::Attention, p),
        ),
        Span::raw(" Alerts"),
    ])
}

pub fn render(frame: &mut Frame, app: &App, area: Rect) {
    if area.width == 0 || area.height == 0 {
        return;
    }
    let p = app.palette();
    let all = alerts(app);
    let index = selected_index(app, &all);
    let keys_here = app.key_region() == KeyRegion::Alerts;
    let mut block = kit::pane_frame(title(all.len(), p), keys_here, p);
    if let Some(i) = index {
        // Muted: unstyled, the right title would take the border's accent.
        let at = format!(" {}/{} ", i + 1, all.len());
        let muted = theme::role(Role::Muted, p);
        block = block.title_top(Line::from(Span::styled(at, muted)).right_aligned());
    }
    frame.render_widget(block, area);
    let inner = inset(area);
    let muted = theme::role(Role::Muted, p);
    let (Some(i), false) = (index, inner.width == 0 || inner.height == 0) else {
        let text = truncate_in("no alerts", usize::from(inner.width), p.ascii);
        frame.render_widget(Paragraph::new(Line::styled(text, muted)), inner);
        return;
    };
    let parts = areas(area, all.len());
    let rows: Vec<Line<'static>> = all
        .iter()
        .enumerate()
        .map(|(at, alert)| list_line(app, alert, at == i, parts.list.width))
        .collect();
    let rows = kit::window(rows, i, usize::from(parts.list.height), p);
    frame.render_widget(Paragraph::new(rows), parts.list);
    let border = theme::border_set(p.ascii);
    let rule: Vec<Line<'static>> = if parts.stacked {
        let line = border.horizontal_top.repeat(usize::from(parts.rule.width));
        vec![Line::styled(line, muted)]
    } else {
        (0..parts.rule.height)
            .map(|_| Line::styled(border.vertical_left, muted))
            .collect()
    };
    frame.render_widget(Paragraph::new(rule), parts.rule);
    let detail = detail_lines(app, &all[i], parts.detail.width);
    let scroll = app.alerts_focus.as_ref().map_or(0, |focus| focus.scroll);
    let detail = detail_window(detail, scroll, usize::from(parts.detail.height), p);
    frame.render_widget(Paragraph::new(detail), parts.detail);
}

/// The detail's rows: `ui/alerts_detail.rs`.
#[path = "alerts_detail.rs"]
mod detail;
pub(crate) use detail::detail_lines;

#[cfg(test)]
#[path = "alerts_view_tests.rs"]
mod tests;
