//! Milestone 9.0.5 decision 12: the plan review screen. It takes the whole body (the
//! sidebar column included): a title row, the task list on the left with the waves
//! summary on its last row, and the selected task's detail on the right, scrolled by
//! `PlanReview.scroll`. The geometry and the right pane's rows are the reducer's own
//! (`app::plan_review::{panes, detail_lines}`), so a page is what the user sees. Every
//! agent-written string is sanitised here or in `detail_lines` (decision 27), and every
//! row is cut to its pane. Pure: rendering takes `&App`.

use crate::app::App;
use crate::app::plan_review::{
    LineKind, PlanReview, ReviewTarget, all_deps, detail_lines, panes, review_tasks,
};
use crate::inspector::run_format::{effort_text, size_letter, test_mode_text};
use crate::safe_text::one_line;
use crate::theme::{self, Palette, Role, role};
use crate::ui::tree_view::truncate_in;
use proto::{RunInfo, TaskInfo};
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use unicode_width::UnicodeWidthStr;

/// `Plan review · <run> · <goal>` or `Hold <id> review · <run> · <goal>`.
fn title_text(review: &PlanReview, run: Option<&RunInfo>) -> String {
    let what = match &review.target {
        ReviewTarget::Gate => "Plan review".to_owned(),
        ReviewTarget::Hold(id) => format!("Hold {id} review"),
    };
    let goal = run
        .map(|run| format!(" · {}", run.goal))
        .unwrap_or_default();
    one_line(&format!("{what} · {}{goal}", review.run_id))
}

/// `<n> tasks · awaiting approval`.
fn count_text(n: usize) -> String {
    match n {
        1 => "1 task · awaiting approval".to_owned(),
        n => format!("{n} tasks · awaiting approval"),
    }
}

/// The task row's second line: `<runtime> <model> <effort> · <S|M|L> · <test mode>`,
/// then ` · after <deps>` when it has any.
fn route_row(task: &TaskInfo) -> String {
    let route = &task.route;
    let who: Vec<&str> = [
        route.runtime.label(),
        &route.model,
        effort_text(route.effort),
    ]
    .into_iter()
    .filter(|part| !part.trim().is_empty())
    .collect();
    let mut text = format!(
        "{} · {} · {}",
        who.join(" "),
        size_letter(task.size),
        test_mode_text(task.test_mode)
    );
    let deps = all_deps(task);
    if !deps.is_empty() {
        text.push_str(&format!(" · after {}", deps.join(", ")));
    }
    one_line(&text)
}

/// `waves  1 <ids> · 2 <ids> …`, by `wave + 1`, ids in plan order.
fn waves_text(tasks: &[&TaskInfo]) -> String {
    let mut waves: Vec<u32> = tasks.iter().map(|task| task.wave).collect();
    waves.sort_unstable();
    waves.dedup();
    let parts: Vec<String> = waves
        .iter()
        .map(|wave| {
            let ids: Vec<&str> = tasks
                .iter()
                .filter(|task| task.wave == *wave)
                .map(|task| task.id.as_str())
                .collect();
            format!("{} {}", wave + 1, ids.join(" "))
        })
        .collect();
    one_line(&format!("waves  {}", parts.join(" · ")))
}

/// One row of the screen: where it goes and what it shows. The render is built as a
/// list of these, so a test can read every span before the buffer does.
#[derive(Debug, Clone)]
pub(crate) struct Placed {
    pub area: Rect,
    pub line: Line<'static>,
}

/// One row at `y` in `area`, the width of `area`.
fn row(out: &mut Vec<Placed>, area: Rect, y: u16, line: Line<'static>) {
    if y >= area.y + area.height || area.width == 0 {
        return;
    }
    let area = Rect {
        y,
        height: 1,
        ..area
    };
    out.push(Placed { area, line });
}

/// `text` cut to `width`, its punctuation folded in ASCII (milestone 9.0.7 decision 5).
fn text(text: &str, width: u16, style: Style, ascii: bool) -> Line<'static> {
    let text = theme::fold(text, ascii);
    Line::from(Span::styled(
        truncate_in(&text, usize::from(width), ascii),
        style,
    ))
}

fn render_title(out: &mut Vec<Placed>, area: Rect, (left, right): (&str, &str), p: Palette) {
    if area.height == 0 {
        return;
    }
    let (left, right) = (&theme::fold(left, p.ascii), &theme::fold(right, p.ascii));
    let truncate = |text: &str, width| truncate_in(text, width, p.ascii);
    let width = usize::from(area.width);
    let right_width = right.width();
    let bold = Style::default().add_modifier(Modifier::BOLD);
    let mut spans = Vec::new();
    if right_width + 2 <= width {
        let left = truncate(left, width - right_width - 1);
        let gap = width - left.width() - right_width;
        // Milestone 9.0.7 decision 2: titles use weight, not the accent.
        spans.push(Span::styled(left, bold));
        spans.push(Span::raw(" ".repeat(gap)));
        spans.push(Span::styled(right.to_owned(), role(Role::Muted, p)));
    } else {
        spans.push(Span::styled(truncate(left, width), bold));
    }
    row(out, area, area.y, Line::from(spans));
}

/// The left pane: two rows a task, scrolled to keep the selection in view, and the
/// waves summary on the last row.
fn render_tasks(
    out: &mut Vec<Placed>,
    area: Rect,
    tasks: &[&TaskInfo],
    selected: Option<&str>,
    p: Palette,
) {
    if area.height == 0 || area.width == 0 {
        return;
    }
    let muted = role(Role::Muted, p);
    let list_rows = area.height - 1;
    let visible = usize::from(list_rows / 2);
    let at = selected
        .and_then(|id| tasks.iter().position(|task| task.id == id))
        .unwrap_or(0);
    let first = if visible == 0 {
        0
    } else {
        at.saturating_sub(visible - 1)
    };
    for (n, task) in tasks.iter().skip(first).take(visible).enumerate() {
        let y = area.y + 2 * n as u16;
        let chosen = selected == Some(task.id.as_str());
        let (mark, style) = if chosen {
            (
                if p.ascii { "> " } else { "▸ " },
                Style::default().add_modifier(Modifier::BOLD),
            )
        } else {
            ("  ", Style::default())
        };
        let head = one_line(&format!("{mark}{}  {}", task.id, task.title));
        row(out, area, y, text(&head, area.width, style, p.ascii));
        let route = format!("  {}", route_row(task));
        row(out, area, y + 1, text(&route, area.width, muted, p.ascii));
    }
    let waves = waves_text(tasks);
    let y = area.y + area.height - 1;
    row(out, area, y, text(&waves, area.width, muted, p.ascii));
}

/// The right pane: `detail_lines` from `scroll`, clamped to the content.
fn render_detail(out: &mut Vec<Placed>, app: &App, area: Rect, run: &RunInfo, task: &TaskInfo) {
    if area.height == 0 || area.width == 0 {
        return;
    }
    let lines = detail_lines(run, task, area.width);
    let max = lines.len().saturating_sub(usize::from(area.height));
    let scroll = usize::from(app.plan_review.as_ref().map_or(0, |r| r.scroll)).min(max);
    for (n, line) in lines
        .iter()
        .skip(scroll)
        .take(usize::from(area.height))
        .enumerate()
    {
        let style = match line.kind {
            LineKind::Title => Style::default().add_modifier(Modifier::BOLD),
            LineKind::Label => Style::default().add_modifier(Modifier::BOLD),
            LineKind::Text | LineKind::Blank => Style::default(),
        };
        let y = area.y + n as u16;
        row(
            out,
            area,
            y,
            text(&line.text, area.width, style, app.palette().ascii),
        );
    }
}

/// The one-column separator between the panes, when there is room for it.
fn render_separator(out: &mut Vec<Placed>, left: Rect, right: Rect, p: Palette) {
    let x = left.x + left.width;
    if right.x <= x || left.height == 0 {
        return;
    }
    let area = Rect {
        x,
        width: 1,
        ..left
    };
    for y in area.y..area.y + area.height {
        let bar = if p.ascii { "|" } else { "│" };
        row(
            out,
            area,
            y,
            Line::from(Span::styled(bar, role(Role::Muted, p))),
        );
    }
}

/// Decision 12's screen in `body`, row by row.
pub(crate) fn placed(app: &App, body: Rect) -> Vec<Placed> {
    let mut out = Vec::new();
    let Some(review) = app.plan_review.as_ref() else {
        return out;
    };
    let run = app.runs.runs.iter().find(|run| run.run_id == review.run_id);
    let tasks = run
        .map(|run| review_tasks(run, &review.target))
        .unwrap_or_default();
    let areas = panes(body);
    let title = title_text(review, run);
    let p = app.palette();
    render_title(&mut out, areas.title, (&title, &count_text(tasks.len())), p);
    let Some(run) = run else {
        return out;
    };
    render_tasks(&mut out, areas.left, &tasks, review.selected.as_deref(), p);
    render_separator(&mut out, areas.left, areas.right, p);
    let task = review
        .selected
        .as_ref()
        .and_then(|id| tasks.iter().find(|task| task.id == *id));
    if let Some(task) = task {
        render_detail(&mut out, app, areas.right, run, task);
    }
    out
}

pub fn render(frame: &mut Frame, app: &App, body: Rect) {
    for Placed { area, line } in placed(app, body) {
        frame.render_widget(Paragraph::new(line), area);
    }
}

#[cfg(test)]
#[path = "plan_review_tests.rs"]
mod tests;
