//! Milestone 9.15 decision 48 (task M9.15.15): the graph page's inspect panel. For a
//! lesson: its title, layer, status, the full body (`multi_line`, wrapped), files,
//! origin runs, usefulness `green/outcomes`, reconfirmed count, last seen
//! (`YYYY-MM-DD`, UTC), and a known failure's signature headlines. For a module (or the
//! root hub): its lessons and known failures as a list, the panel's row selected while
//! the panel has the keys (`Tab`). Every stored string is sanitised and folded in
//! ASCII. Pure.

use super::kit::{self, cut};
use crate::app::screens::lessons_graph::LessonsGraphScreen;
use crate::safe_text::{multi_line, one_line};
use crate::theme::{Glyph, Palette, Role, ellipsis, fold, glyph, role};
use proto::{GraphNodeKind, LessonInfo};
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Clear, Paragraph};

/// `secs` since the Unix epoch as a UTC `YYYY-MM-DD` (Howard Hinnant's
/// `civil_from_days`).
pub fn date(secs: u64) -> String {
    let days = (secs / 86_400) as i64 + 719_468;
    let era = days.div_euclid(146_097);
    let doe = days.rem_euclid(146_097);
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!("{year:04}-{month:02}-{day:02}")
}

/// A lesson's panel lines at `width` columns.
pub fn lesson_lines(lesson: &LessonInfo, width: u16, p: Palette) -> Vec<Line<'static>> {
    let usefulness = if lesson.outcomes == 0 {
        "unmeasured".to_string()
    } else {
        format!("{}/{}", lesson.green, lesson.outcomes)
    };
    let mut lines = vec![Line::styled(
        cut(
            &fold(&one_line(&lesson.title), p.ascii),
            usize::from(width),
            ellipsis(p),
        ),
        Style::default().add_modifier(Modifier::BOLD),
    )];
    lines.extend(kit::labelled_rows(
        &[
            ("id".into(), lesson.id.clone()),
            ("layer".into(), lesson.layer.name().into()),
            ("status".into(), lesson.status.name().into()),
        ],
        width,
        p,
    ));
    lines.push(Line::default());
    for paragraph in multi_line(&lesson.body).lines() {
        let paragraph = fold(&one_line(paragraph), p.ascii);
        let parts = kit::wrap_words(&paragraph, usize::from(width).max(1));
        if parts.is_empty() {
            lines.push(Line::default());
        }
        lines.extend(parts.into_iter().map(Line::raw));
    }
    lines.push(Line::default());
    let mut rows: Vec<(String, String)> = vec![];
    if !lesson.files.is_empty() {
        rows.push(("files".into(), lesson.files.join(", ")));
    }
    if !lesson.runs.is_empty() {
        rows.push(("runs".into(), lesson.runs.join(", ")));
    }
    rows.extend([
        ("useful".into(), usefulness),
        ("reconfirmed".into(), lesson.reconfirmed.to_string()),
        ("last seen".into(), date(lesson.last_seen)),
    ]);
    for (i, headline) in lesson.headlines.iter().enumerate() {
        let label = if i == 0 { "signature" } else { "" };
        rows.push((label.into(), headline.clone()));
    }
    lines.extend(kit::labelled_rows(&rows, width, p));
    lines
}

/// A module's panel lines: its name, then one row per lesson or known failure, the
/// panel's `row` marked; and the index of that row's line.
pub fn module_lines(
    s: &LessonsGraphScreen,
    id: &str,
    width: u16,
    keys: bool,
    p: Palette,
) -> (Vec<Line<'static>>, usize) {
    let name = s
        .index_of(id)
        .and_then(|i| s.graph.as_ref()?.nodes.get(i))
        .map_or(id.to_string(), |n| n.label.clone());
    let mut lines = vec![Line::styled(
        cut(
            &fold(&one_line(&name), p.ascii),
            usize::from(width),
            ellipsis(p),
        ),
        Style::default().add_modifier(Modifier::BOLD),
    )];
    let rows = s.module_rows(id);
    if rows.is_empty() {
        lines.push(Line::styled("no lessons", role(Role::Muted, p)));
        return (lines, 0);
    }
    let focus = s.panel.is_some_and(|panel| panel.focus) && keys;
    let at = s.panel.map_or(0, |panel| panel.row).min(rows.len() - 1);
    for (i, lesson) in rows.iter().enumerate() {
        let node = s
            .index_of(lesson)
            .and_then(|j| s.graph.as_ref()?.nodes.get(j));
        let (g, r) = node.map_or(("•", None), |n| super::lessons_graph::look(n, p.ascii));
        let title = s
            .lesson(lesson)
            .map_or_else(|| lesson.clone(), |l| l.title.clone());
        let room = usize::from(width).saturating_sub(4);
        let bar = if i == at {
            glyph(Glyph::Selection, p.ascii)
        } else {
            " "
        };
        let text_style = if i == at && focus {
            Style::default().add_modifier(Modifier::REVERSED)
        } else {
            Style::default()
        };
        lines.push(Line::from(vec![
            Span::styled(bar.to_string(), role(kit::bar_role(focus), p)),
            Span::styled(g.to_string(), r.map_or(Style::default(), |r| role(r, p))),
            Span::styled(
                format!(
                    " {}",
                    cut(&fold(&one_line(&title), p.ascii), room, ellipsis(p))
                ),
                text_style,
            ),
        ]));
    }
    (lines, at + 1)
}

/// Draws the panel for the selected node in `area`, framed; its border in the accent
/// while it has the keys.
pub fn render(frame: &mut Frame, s: &LessonsGraphScreen, area: Rect, keys: bool, p: Palette) {
    let Some(node) = s.selected_node() else {
        return;
    };
    let focus = keys && s.panel.is_some_and(|panel| panel.focus);
    let block = kit::pane_frame(Line::raw("inspect"), focus, p);
    let inner = block.inner(area);
    frame.render_widget(Clear, area);
    frame.render_widget(block, area);
    if inner.width == 0 || inner.height == 0 {
        return;
    }
    let (lines, at) = match node.kind {
        GraphNodeKind::Lesson => match s.lesson(&node.id) {
            Some(lesson) => {
                let lines = lesson_lines(lesson, inner.width, p);
                let at = s.panel.map_or(0, |panel| panel.row);
                let at = at.min(lines.len().saturating_sub(1));
                (lines, at)
            }
            None => (vec![Line::raw(fold(&one_line(&node.label), p.ascii))], 0),
        },
        _ => module_lines(s, &node.id.clone(), inner.width, keys, p),
    };
    let shown = kit::window(lines, at, usize::from(inner.height), p);
    frame.render_widget(Paragraph::new(shown), inner);
}
