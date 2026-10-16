//! Milestone 9.15 decisions 35 and 37 (task M9.15.12): the Profile screen's Lessons
//! page, drawn over the screen in its own frame (`lessons · <project>`): the daemon's
//! last refusal or text, one row per lesson (`<glyph> <title>`, with `<layer> ·
//! <green>/<outcomes>` right-aligned), the selected lesson's id, body and files under
//! the list (`kit::labelled_rows`), and the bottom line: the page's keys, or the
//! question `reject <id>? y/n` / `forget <id>? y/n` in the kit's destructive style.
//! Every stored string passes `safe_text::one_line`, and is folded in ASCII. Pure.

use super::{hint, plain};
use crate::app::profile_screen::{LessonsPage, ProfileScreen};
use crate::safe_text::one_line;
use crate::theme::{Glyph, Palette, Role, dot, ellipsis, glyph, role};
use crate::ui::kit::{self, Hint, cut};
use proto::{LessonInfo, LessonStatus};
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Clear, Paragraph};
use unicode_width::UnicodeWidthStr;

/// Decision 37: a status's glyph (Unicode, ASCII) and its role (`None`: the normal
/// text colour).
fn look(status: LessonStatus, ascii: bool) -> (&'static str, Option<Role>) {
    let (uni, asc, r) = match status {
        LessonStatus::Known => ("✖", "x", Some(Role::Failed)),
        LessonStatus::OnTrial => ("◌", ".", Some(Role::Muted)),
        LessonStatus::Proposed => ("○", "o", Some(Role::Attention)),
        LessonStatus::Approved | LessonStatus::Kept => ("●", "*", None),
        LessonStatus::Rejected | LessonStatus::Retired => ("–", "-", Some(Role::Muted)),
    };
    (if ascii { asc } else { uni }, r)
}

/// `<layer> · <green>/<outcomes>`, `–` when unmeasured.
fn right_text(lesson: &LessonInfo, p: Palette) -> String {
    let usefulness = if lesson.outcomes == 0 {
        if p.ascii { "-" } else { "–" }.to_string()
    } else {
        format!("{}/{}", lesson.green, lesson.outcomes)
    };
    format!("{} {} {usefulness}", lesson.layer.name(), dot(p))
}

fn row(lesson: &LessonInfo, selected: bool, keys: bool, width: usize, p: Palette) -> Line<'static> {
    let bar = if selected {
        glyph(Glyph::Selection, p.ascii)
    } else {
        " "
    };
    let (g, r) = look(lesson.status, p.ascii);
    let right = right_text(lesson, p);
    let room = width.saturating_sub(2 + 1 + 2 + right.width());
    let title = cut(&plain(&lesson.title, p), room, ellipsis(p));
    let gap = width
        .saturating_sub(3 + title.width() + right.width())
        .max(1);
    Line::from(vec![
        Span::styled(bar.to_string(), role(kit::bar_role(keys), p)),
        Span::styled(g.to_string(), r.map_or(Style::default(), |r| role(r, p))),
        Span::raw(format!(" {title}")),
        Span::raw(" ".repeat(gap)),
        Span::styled(right, role(Role::Muted, p)),
    ])
}

/// The page's bottom line: its question, or its keys.
fn bottom(page: &LessonsPage, width: u16, p: Palette) -> Line<'static> {
    if let Some(q) = &page.question {
        let line = Line::from(vec![
            Span::raw(q.verb.word()),
            Span::raw(format!(" {}? ", plain(&q.id, p))),
            Span::styled("y/n", role(Role::Accent, p)),
        ]);
        return kit::destructive(line, "y/n", q.verb.word(), p);
    }
    let all = if page.all { "active only" } else { "all" };
    let list: [(&str, &str); 7] = [
        ("j/k", "move"),
        ("a", "approve"),
        ("r", "reject"),
        ("d", "forget"),
        ("t", all),
        ("g", "graph"),
        ("esc", "back"),
    ];
    let hints: Vec<Hint> = (list.iter().enumerate())
        .map(|(i, (k, w))| hint(k, w, if *k == "esc" { 9 } else { 8 - i as u8 }))
        .collect();
    kit::hints_joined(width, &hints, crate::theme::dot_sep(p), p)
}

/// The page's title, its body (the list and the selected lesson), the index of the
/// selected row in the body, and its bottom line, for `width` interior columns.
pub(super) fn parts(
    s: &ProfileScreen,
    page: &LessonsPage,
    width: u16,
    keys: bool,
    p: Palette,
) -> (String, Vec<Line<'static>>, usize, Line<'static>) {
    let w = usize::from(width);
    let title = format!("lessons {} {}", dot(p), super::project_name(s));
    let mut body = Vec::new();
    if let Some(error) = &page.error {
        body.push(Line::styled(
            cut(&plain(error, p), w, ellipsis(p)),
            role(Role::Failed, p),
        ));
    } else if let Some(message) = &page.message {
        body.push(Line::styled(
            cut(&plain(message, p), w, ellipsis(p)),
            role(Role::Muted, p),
        ));
    }
    let mut at = 0;
    match &page.lessons {
        None => body.push(Line::styled(
            format!("loading{}", ellipsis(p)),
            role(Role::Muted, p),
        )),
        Some(list) if list.is_empty() => {
            let text = format!("no lessons for {} yet", super::project_name(s));
            body.push(Line::styled(
                cut(&plain(&text, p), w, ellipsis(p)),
                role(Role::Muted, p),
            ));
        }
        Some(list) => {
            for (i, lesson) in list.iter().enumerate() {
                if i == page.selected {
                    at = body.len();
                }
                body.push(row(lesson, i == page.selected, keys, w, p));
            }
        }
    }
    if let Some(lesson) = page.selected_lesson() {
        let mut rows = vec![
            ("id".to_string(), one_line(&lesson.id)),
            ("body".to_string(), one_line(&lesson.body)),
        ];
        if !lesson.files.is_empty() {
            let files: Vec<String> = lesson.files.iter().map(|f| one_line(f)).collect();
            rows.push(("files".to_string(), files.join(", ")));
        }
        body.push(Line::default());
        body.extend(kit::labelled_rows(&rows, width, p));
    }
    (title, body, at, bottom(page, width, p))
}

/// Draws the page over `area` (the screen's): its frame, its body kept in view around
/// the selected row, and its bottom line on the frame's last row.
pub(super) fn render(
    frame: &mut Frame,
    s: &ProfileScreen,
    page: &LessonsPage,
    area: Rect,
    keys: bool,
    p: Palette,
) {
    let probe = kit::screen_frame("", keys, p);
    let inner = probe.inner(area);
    if inner.width == 0 || inner.height == 0 {
        return;
    }
    let (title, body, at, last) = parts(s, page, inner.width, keys, p);
    let rows = usize::from(inner.height).saturating_sub(1);
    let mut lines = kit::window(body, at, rows, p);
    lines.resize(rows, Line::default());
    lines.push(last);
    frame.render_widget(Clear, area);
    frame.render_widget(
        Paragraph::new(lines).block(kit::screen_frame(&title, keys, p)),
        area,
    );
}
