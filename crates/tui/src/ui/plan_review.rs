//! The plan review screen (milestone 9.0.5 decision 12, reshaped by milestone 9.0.7
//! decisions 23–26). It takes the whole body (the sidebar column included) in one
//! `kit::pane_frame`, titled ` plan · <run name> ` or ` hold <h> · <run name> ` with
//! ` awaiting approval ` on the right, and stacks, top to bottom: the summary header
//! (counts, sizes, budget, critical path, `⚠` overlaps), a `├─…─┤` rule, the task list
//! in aligned columns (scrolled with `kit::window`), a rule, and the selected task's
//! labelled detail, scrolled by `PlanReview.scroll`. The geometry and the detail's rows
//! are the reducer's own (`app::plan_review::{stacked, detail_lines}`), so a page is
//! what the user sees. Every agent-written string is sanitised here, in
//! `app::plan_summary` or in `detail_lines` (decision 27), and every row is cut to its
//! area by display width. Pure: rendering takes `&App`.

use crate::app::App;
use crate::app::plan_review::review_tasks;
use crate::app::plan_review::{BAR, PlanReview, ReviewLayout, ReviewTarget, detail_lines};
use crate::app::plan_summary::{Cells, columns, header_line, overlap_lines, overlaps};
use crate::app::region::KeyRegion;
use crate::theme::{self, Glyph, Palette, Role, role};
use crate::ui::kit;
use crate::ui::tree_view::truncate_in;
use proto::{RunInfo, TaskInfo};
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Paragraph};
use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthStr;

/// The frame's right-hand title.
const RIGHT: &str = "awaiting approval";
/// Below this many columns for the run name, the right-hand title goes.
const MIN_NAME: usize = 16;
/// The title columns a task keeps before the route tag goes (decision 24).
const MIN_TITLE: usize = 8;
/// The columns between two columns.
const GAP: usize = 2;

/// One row of the screen: where it goes and what it shows. The render is built as a
/// list of these, so a test can read every span before the buffer does.
#[derive(Debug, Clone)]
pub(crate) struct Placed {
    pub area: Rect,
    pub line: Line<'static>,
}

/// One row at `y` in `area`, the width of `area`, its spans cut to that width.
fn row(out: &mut Vec<Placed>, area: Rect, y: u16, line: Line<'static>) {
    if y < area.y || y >= area.bottom() || area.width == 0 {
        return;
    }
    let area = Rect {
        y,
        height: 1,
        ..area
    };
    out.push(Placed {
        area,
        line: clip(line, usize::from(area.width)),
    });
}

/// `line` cut to `width` display columns, grapheme by grapheme (a wide grapheme that
/// would cross the edge is left out).
fn clip(line: Line<'static>, width: usize) -> Line<'static> {
    let mut left = width;
    let mut spans = Vec::with_capacity(line.spans.len());
    for span in line.spans {
        if span.content.width() <= left {
            left -= span.content.width();
            spans.push(span);
            continue;
        }
        let mut kept = String::new();
        for g in span.content.graphemes(true) {
            if g.width() > left {
                break;
            }
            left -= g.width();
            kept.push_str(g);
        }
        spans.push(Span::styled(kept, span.style));
        break;
    }
    Line::from(spans).style(line.style)
}

fn width_of(text: &str) -> usize {
    text.width()
}

/// Decision 23's frame: ` plan · <run name> ` or ` hold <h> · <run name> `, the name
/// cut to what ` awaiting approval ` leaves, which goes first when it leaves under
/// `MIN_NAME` columns. The border is in the accent while the review has the keys.
fn frame_block(
    app: &App,
    review: &PlanReview,
    run: Option<&RunInfo>,
    body: Rect,
) -> Block<'static> {
    let p = app.palette();
    let what = match &review.target {
        ReviewTarget::Gate => "plan · ".to_owned(),
        ReviewTarget::Hold(id) => format!("hold {id} · "),
    };
    let what = theme::fold(&crate::safe_text::one_line(&what), p.ascii);
    // The corners, each title's two spaces, and one `─` between the titles.
    let room = usize::from(body.width).saturating_sub(4 + width_of(&what));
    let with_right = room.saturating_sub(RIGHT.len() + 3);
    let (room, right) = if with_right >= MIN_NAME {
        (with_right, true)
    } else {
        (room, false)
    };
    let room = u16::try_from(room).unwrap_or(u16::MAX);
    let name = match run {
        Some(run) => kit::run_name_in(&run.goal, &run.run_id, room, p),
        None => crate::safe_text::one_line(&review.run_id),
    };
    let keys_here = app.key_region() == KeyRegion::Review;
    let block = kit::pane_frame(Line::from(format!("{what}{name}")), keys_here, p);
    if !right {
        return block;
    }
    let muted = role(Role::Muted, p);
    block.title_top(Line::from(Span::styled(format!(" {RIGHT} "), muted)).right_aligned())
}

/// The header (decision 23): the summary row, then the warnings in `Attention`, each
/// indented by the bar's column and cut to the width.
fn render_header(
    out: &mut Vec<Placed>,
    area: Rect,
    run: &RunInfo,
    tasks: &[&TaskInfo],
    p: Palette,
) {
    let area = indented(area);
    let width = usize::from(area.width);
    let mut lines = vec![Line::raw(header_line(run, tasks, area.width, p.ascii))];
    let warn = role(Role::Attention, p);
    lines.extend(
        overlap_lines(&overlaps(tasks), p.ascii)
            .into_iter()
            .map(|text| Line::styled(truncate_in(&text, width, p.ascii), warn)),
    );
    for (n, line) in lines.into_iter().enumerate() {
        let y = area.y.saturating_add(u16::try_from(n).unwrap_or(u16::MAX));
        row(out, area, y, line);
    }
}

/// `area` less the bar's column on the left.
fn indented(area: Rect) -> Rect {
    let bar = BAR.min(area.width);
    Rect {
        x: area.x + bar,
        width: area.width - bar,
        ..area
    }
}

/// Decision 23's rules, `├─…─┤` across the frame (`|-…-|` in ASCII, whose `+` would
/// read as a stacked frame's corner), in the frame's border style.
fn render_rules(
    out: &mut Vec<Placed>,
    body: Rect,
    layout: &ReviewLayout,
    border: Style,
    p: Palette,
) {
    if body.width < 2 {
        return;
    }
    let (left, line, right) = if p.ascii {
        ("|", "-", "|")
    } else {
        ("├", "─", "┤")
    };
    let text = format!("{left}{}{right}", line.repeat(usize::from(body.width) - 2));
    for y in layout.rules.into_iter().flatten() {
        row(out, body, y, Line::styled(text.clone(), border));
    }
}

/// Every column's width once the row is fitted to the list (decision 24): `None` for
/// a dropped column.
struct Fit {
    label: usize,
    route: Option<usize>,
    size: usize,
    stage: Option<usize>,
    deps: usize,
}

impl Fit {
    /// Each column as wide as its widest value; when the row does not fit, the title
    /// shrinks to `MIN_TITLE` columns first, then the route tag goes, then the stage
    /// column; the title takes back what those freed, and only then are the deps cut.
    fn of(cells: &[Cells], width: usize) -> Self {
        let max = |f: &dyn Fn(&Cells) -> usize| cells.iter().map(f).max().unwrap_or(0);
        let label = |c: &Cells, title: usize| {
            width_of(&c.id) + usize::from(!c.title.is_empty()) + title.min(width_of(&c.title))
        };
        let (want, least) = (
            max(&|c| label(c, usize::MAX)),
            max(&|c| label(c, MIN_TITLE)),
        );
        let deps = max(&|c| width_of(&c.deps));
        let stage = cells
            .iter()
            .filter_map(|c| c.stage.as_deref().map(width_of))
            .max();
        let mut fit = Fit {
            label: want,
            route: Some(max(&|c| width_of(&c.route))),
            size: max(&|c| width_of(&c.size)),
            stage,
            deps,
        };
        let rest = |f: &Fit, deps: usize| {
            f.route.map_or(0, |w| GAP + w)
                + GAP
                + f.size
                + f.stage.map_or(0, |w| GAP + w)
                + if deps > 0 { GAP + deps } else { 0 }
        };
        let room = width.saturating_sub(usize::from(BAR));
        if least + rest(&fit, deps) > room {
            fit.route = None;
        }
        if least + rest(&fit, deps) > room {
            fit.stage = None;
        }
        fit.label = room.saturating_sub(rest(&fit, deps)).clamp(least, want);
        let used = fit.label + rest(&fit, 0) + GAP;
        fit.deps = deps.min(room.saturating_sub(used));
        fit
    }
}

/// `text` cut with `…` to `to` columns and padded to them.
fn pad(text: &str, to: usize, ascii: bool) -> String {
    let text = truncate_in(text, to, ascii);
    let fill = to.saturating_sub(width_of(&text));
    format!("{text}{}", " ".repeat(fill))
}

/// One task's row: the bar or a space, `<id> <title>`, then the fitted columns; the
/// selection reversed across the row, its bar in the accent (decision 20's idiom).
fn task_line(cells: &Cells, fit: &Fit, width: usize, selected: bool, p: Palette) -> Line<'static> {
    let muted = role(Role::Muted, p);
    let label = if cells.title.is_empty() {
        cells.id.clone()
    } else {
        format!("{} {}", cells.id, cells.title)
    };
    let mut spans = vec![Span::raw(pad(&label, fit.label, p.ascii))];
    let mut column = |text: &str, to: usize| {
        spans.push(Span::raw(" ".repeat(GAP)));
        spans.push(Span::styled(pad(text, to, p.ascii), muted));
    };
    if let Some(to) = fit.route {
        column(&cells.route, to);
    }
    column(&cells.size, fit.size);
    if let Some(to) = fit.stage {
        column(cells.stage.as_deref().unwrap_or(""), to);
    }
    if !cells.deps.is_empty() && fit.deps > 0 {
        spans.push(Span::raw(" ".repeat(GAP)));
        spans.push(Span::styled(
            truncate_in(&cells.deps, fit.deps, p.ascii),
            muted,
        ));
    }
    if !selected {
        spans.insert(0, Span::raw(" ".repeat(usize::from(BAR))));
        return Line::from(spans);
    }
    let used: usize = spans.iter().map(|s| s.content.width()).sum();
    spans.push(Span::raw(
        " ".repeat(width.saturating_sub(used + usize::from(BAR))),
    ));
    let bar = Span::styled(
        theme::glyph(Glyph::Selection, p.ascii),
        role(Role::Accent, p),
    );
    let reversed = spans.into_iter().map(|span| {
        let style = span.style.add_modifier(Modifier::REVERSED);
        Span::styled(span.content, style)
    });
    Line::from(std::iter::once(bar).chain(reversed).collect::<Vec<_>>())
}

/// Decision 24's list: a row a task, scrolled by `kit::window` to keep the selection
/// in view, its `↑`/`↓ n more` marks indented to the ids' column.
fn render_list(
    out: &mut Vec<Placed>,
    area: Rect,
    run: &RunInfo,
    tasks: &[&TaskInfo],
    selected: Option<&str>,
    p: Palette,
) {
    let width = usize::from(area.width);
    let cells = columns(run, tasks, p.ascii);
    let fit = Fit::of(&cells, width);
    let at = selected
        .and_then(|id| tasks.iter().position(|task| task.id == id))
        .unwrap_or(0);
    let lines: Vec<Line<'static>> = cells
        .iter()
        .zip(tasks)
        .map(|(c, task)| task_line(c, &fit, width, selected == Some(task.id.as_str()), p))
        .collect();
    let shown = kit::window(lines, at, usize::from(area.height), p);
    for (n, mut line) in shown.into_iter().enumerate() {
        // A mark is one span; a task row is the bar or its space and more.
        if line.spans.len() == 1 {
            line.spans
                .insert(0, Span::raw(" ".repeat(usize::from(BAR))));
        }
        let y = area.y.saturating_add(u16::try_from(n).unwrap_or(u16::MAX));
        row(out, area, y, line);
    }
}

/// Decision 25's detail: `detail_lines` from `scroll`, clamped to the content.
fn render_detail(out: &mut Vec<Placed>, app: &App, area: Rect, run: &RunInfo, task: &TaskInfo) {
    let lines = detail_lines(run, task, area.width, app.palette());
    let height = usize::from(area.height);
    let max = lines.len().saturating_sub(height);
    let scroll = usize::from(app.plan_review.as_ref().map_or(0, |r| r.scroll)).min(max);
    let area = indented(area);
    for (n, line) in lines.into_iter().skip(scroll).take(height).enumerate() {
        let y = area.y.saturating_add(u16::try_from(n).unwrap_or(u16::MAX));
        row(out, area, y, line);
    }
}

/// The screen inside the frame at `body`, row by row (the rules on the frame's sides).
pub(crate) fn placed(app: &App, body: Rect) -> Vec<Placed> {
    let mut out = Vec::new();
    let Some(review) = app.plan_review.as_ref() else {
        return out;
    };
    let Some(run) = app.runs.runs.iter().find(|run| run.run_id == review.run_id) else {
        return out;
    };
    let p = app.palette();
    let tasks = review_tasks(run, &review.target);
    let layout = app.review_layout(body);
    let keys_here = app.key_region() == KeyRegion::Review;
    let border = role(if keys_here { Role::Accent } else { Role::Muted }, p);
    render_header(&mut out, layout.header, run, &tasks, p);
    render_rules(&mut out, body, &layout, border, p);
    let selected = review.selected.as_deref();
    render_list(&mut out, layout.list, run, &tasks, selected, p);
    if let Some(task) = selected.and_then(|id| tasks.iter().find(|task| task.id == id)) {
        render_detail(&mut out, app, layout.detail, run, task);
    }
    out
}

pub fn render(frame: &mut Frame, app: &App, body: Rect) {
    let Some(review) = app.plan_review.as_ref() else {
        return;
    };
    if body.width == 0 || body.height == 0 {
        return;
    }
    let run = app.runs.runs.iter().find(|run| run.run_id == review.run_id);
    frame.render_widget(frame_block(app, review, run, body), body);
    for Placed { area, line } in placed(app, body) {
        frame.render_widget(Paragraph::new(line), area);
    }
}

#[cfg(test)]
#[path = "plan_review_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "plan_review_frame_tests.rs"]
mod frame_tests;
