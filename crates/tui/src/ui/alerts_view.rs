//! Milestone 9.0.7 decision 11: the Alerts view, drawn over the main pane while
//! `app.alerts_focus` is set (`C-b a`). A frame titled ` ⚑ Alerts ` with ` <i>/<n> `;
//! the list (`P<p> <glyph> <who>[ › <t>]`, the age flush right, the selection reversed
//! with the `▌` bar) left of a `│` rule from an interior of 60 columns, else on top of
//! a `─` rule; the selected alert's whole detail beside or below it, scrolled by
//! PgUp/PgDn. The alerts are `app::alerts`'s; every daemon string passes `safe_text`
//! here. Pure: rendering takes `&App`.

use super::alerts::{age_text, fitted, who_text};
use super::kit::{self, Hint};
use crate::app::alerts_view::{alert_actions, alert_node, can_message, enter_label};
use crate::app::region::KeyRegion;
use crate::app::{Alert, AlertKey, AlertWho, App, alerts};
use crate::inspector::run_format::reason_text;
use crate::safe_text::{multi_line, one_line};
use crate::theme::{self, Glyph, Palette, Role, fold, glyph};
use crate::tree::{self, format_elapsed};
use crate::ui::tree_view::truncate_in;
use proto::{AgentRole, RunInfo, TaskInfo, TaskState};
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
        format!(" {} {}", glyph(Glyph::Separator, p.ascii), one_line(t))
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

/// `1 task` or `<n> tasks`.
fn tasks_text(n: usize) -> String {
    match n {
        1 => "1 task".to_owned(),
        n => format!("{n} tasks"),
    }
}

/// `<n tasks>[ · <s> stages]`: stages only when the plan has more than one.
fn plan_text(run: &RunInfo, n: usize) -> String {
    let planned = run.tasks.iter().map(|t| t.stage).max().unwrap_or(1);
    let stages = usize::from(planned).max(run.stages.len());
    if stages > 1 {
        format!("{} · {stages} stages", tasks_text(n))
    } else {
        tasks_text(n)
    }
}

/// `<base>@<base7>`, the branch alone when the sha is unknown; none without a branch.
fn base_text(run: &RunInfo) -> Option<String> {
    let branch = one_line(&run.base_branch);
    if branch.trim().is_empty() {
        return None;
    }
    let sha: String = one_line(&run.base_sha).chars().take(7).collect();
    Some(if sha.trim().is_empty() {
        branch
    } else {
        format!("{branch}@{sha}")
    })
}

/// `<tag> <model> · <k> calls · <elapsed>` of the task's latest worker round (M9.0.5's
/// `worker_line` facts: the elapsed time frozen at the round's end).
fn worker_text(app: &App, task: &TaskInfo) -> Option<String> {
    let round = task
        .rounds
        .iter()
        .filter(|round| round.role == AgentRole::Worker)
        .max_by_key(|round| (round.started_at, round.session, round.round))?;
    let elapsed = match round.ended_at {
        Some(ended) => ended.saturating_sub(round.started_at),
        None => app.run_age(round.started_at),
    };
    let calls = match round.tool_calls {
        1 => "1 call".to_owned(),
        n => format!("{n} calls"),
    };
    Some(format!(
        "{} {} · {calls} · {}",
        theme::runtime_tag(round.route.runtime),
        model_text(&round.route.model),
        format_elapsed(elapsed)
    ))
}

fn model_text(model: &str) -> String {
    let model = one_line(model);
    if model.trim().is_empty() {
        "default".to_owned()
    } else {
        model
    }
}

/// `label` on the first non-blank line of `text`, the rest under it.
fn text_rows(rows: &mut Vec<(String, String)>, label: &str, text: &str) {
    let text = multi_line(text);
    let lines: Vec<&str> = text.lines().collect();
    let first = lines.iter().position(|line| !line.trim().is_empty());
    let last = lines.iter().rposition(|line| !line.trim().is_empty());
    let (Some(first), Some(last)) = (first, last) else {
        return;
    };
    for (i, line) in lines[first..=last].iter().enumerate() {
        let label = if i == 0 { label } else { "" };
        rows.push((label.to_owned(), line.trim_end().to_owned()));
    }
}

fn run_of<'a>(app: &'a App, run_id: &str) -> Option<&'a RunInfo> {
    app.runs.runs.iter().find(|run| run.run_id == run_id)
}

fn run_name(run: &RunInfo, p: Palette) -> String {
    kit::run_name_in(tree::run_title(run), &run.run_id, u16::MAX, p)
}

/// Decision 11's labelled rows for `alert`, in order, each only with a value: `phase`,
/// `asked` or `reason`, `plan`, `merged`, `base`, `run`, `worker` or `orchestrator`,
/// `project`, `age`.
fn facts(app: &App, alert: &Alert) -> Vec<(String, String)> {
    let p = app.palette();
    let mut rows: Vec<(String, String)> = Vec::new();
    let push = |rows: &mut Vec<(String, String)>, label: &str, value: String| {
        rows.push((label.to_owned(), value));
    };
    let run = match &alert.key {
        AlertKey::Proposal(_) => None,
        AlertKey::Orchestrator(id)
        | AlertKey::Gate(id)
        | AlertKey::Halted(id)
        | AlertKey::Accept(id)
        | AlertKey::Hold { run: id, .. }
        | AlertKey::Blocked { run: id, .. } => run_of(app, id),
    };
    let counted = |run: &RunInfo| {
        run.tasks
            .iter()
            .filter(|task| task.state != TaskState::Cancelled)
            .count()
    };
    match (&alert.key, run) {
        (AlertKey::Blocked { task, .. }, Some(run)) => {
            let task = run.tasks.iter().find(|t| t.id == *task);
            let block = task.and_then(|t| t.block.as_ref());
            let phase = match block {
                Some(block) => format!("blocked · {}", reason_text(block.reason)),
                None => "blocked".to_owned(),
            };
            push(&mut rows, "phase", phase);
            if let Some(block) = block {
                let label = match block.reason {
                    proto::BlockReason::Question => "asked",
                    _ => "reason",
                };
                text_rows(&mut rows, label, &block.text);
            }
            push(&mut rows, "run", run_name(run, p));
            if let Some(worker) = task.and_then(|t| worker_text(app, t)) {
                push(&mut rows, "worker", worker);
            }
        }
        (AlertKey::Gate(_), Some(run)) => {
            push(&mut rows, "phase", "awaiting approval".to_owned());
            push(&mut rows, "plan", plan_text(run, counted(run)));
            if let Some(base) = base_text(run) {
                push(&mut rows, "base", base);
            }
        }
        (AlertKey::Hold { hold, .. }, Some(run)) => {
            push(&mut rows, "phase", format!("hold {hold} awaiting approval"));
            let n = run
                .holds
                .iter()
                .find(|h| h.id == *hold)
                .map(|h| h.tasks.len());
            if let Some(n) = n {
                push(&mut rows, "plan", plan_text(run, n));
            }
        }
        (AlertKey::Halted(_), Some(run)) => {
            push(&mut rows, "phase", "halted".to_owned());
            if let Some(reason) = &run.halted_reason {
                text_rows(&mut rows, "reason", reason);
            }
        }
        (AlertKey::Accept(_), Some(run)) => {
            push(&mut rows, "phase", "complete · ready to accept".to_owned());
            let merged = run
                .tasks
                .iter()
                .filter(|task| task.state == TaskState::Merged)
                .count();
            push(&mut rows, "merged", format!("{merged}/{}", counted(run)));
            if let Some(base) = base_text(run) {
                push(&mut rows, "base", base);
            }
        }
        (AlertKey::Orchestrator(_), Some(run)) => {
            let orch = run.orchestrator.as_ref();
            let window = orch
                .and_then(|orch| orch.window_id)
                .and_then(|id| app.windows.iter().find(|w| w.id == id));
            if let (Some(orch), Some(window)) = (orch, window) {
                let status = window.status.label();
                push(&mut rows, "phase", status.to_owned());
                let route = &orch.route;
                let tag = theme::runtime_tag(route.runtime);
                let model = model_text(&route.model);
                push(
                    &mut rows,
                    "orchestrator",
                    format!("{tag} {model} · {status}"),
                );
            }
        }
        (AlertKey::Proposal(project), _) => {
            push(&mut rows, "phase", "proposal ready".to_owned());
            push(&mut rows, "project", project.to_string_lossy().into_owned());
        }
        (_, None) => {}
    }
    if let Some(age) = alert.age {
        push(&mut rows, "age", age_text(age));
    }
    rows
}

/// The header: `<t>  <title>` for a task alert, else the run's name (the project's
/// for a proposal), bold.
fn header(app: &App, alert: &Alert, width: usize) -> Line<'static> {
    let p = app.palette();
    let text = match (&alert.key, &alert.who) {
        (AlertKey::Blocked { run, task }, _) => {
            let title = run_of(app, run)
                .and_then(|r| r.tasks.iter().find(|t| t.id == *task))
                .map(|t| one_line(&t.title))
                .unwrap_or_default();
            format!("{}  {}", one_line(task), title.trim())
        }
        (_, AlertWho::Project(name)) => one_line(name),
        (_, AlertWho::Run { goal, id }) => kit::run_name_in(goal, id, u16::MAX, p),
    };
    let text = truncate_in(&fold(text.trim_end(), p.ascii), width, p.ascii);
    Line::from(Span::styled(
        text,
        Style::default().add_modifier(Modifier::BOLD),
    ))
}

/// `actions  <label> · <label> …` on one row, a refused entry muted (the menu gives
/// its reason); none when the alert has no node.
fn actions_line(app: &App, alert: &Alert, width: usize) -> Option<Line<'static>> {
    let items = alert_actions(app, &alert.key);
    if items.is_empty() {
        return None;
    }
    let p = app.palette();
    let muted = theme::role(Role::Muted, p);
    let separator = fold(" · ", p.ascii);
    let mut parts = vec![("actions  ".to_owned(), muted)];
    for (i, item) in items.iter().enumerate() {
        if i > 0 {
            parts.push((separator.clone(), muted));
        }
        let style = if item.refused_why.is_some() {
            muted
        } else {
            Style::default()
        };
        parts.push((fold(&one_line(&item.label), p.ascii), style));
    }
    Some(fitted(parts, width, p.ascii))
}

/// The detail's own hint row (Interfaces "Hint priorities"): `⏎ <label>`, `. all
/// actions`, `m message` where offered, `o open <task|run>` (not on a proposal, whose
/// Enter opens the same screen).
fn detail_hints(app: &App, alert: &Alert) -> Vec<Hint> {
    let hint = |key: &str, word: &str, priority| Hint {
        key: key.to_owned(),
        word: word.to_owned(),
        priority,
    };
    let mut hints = vec![hint("⏎", &enter_label(app, &alert.key), 9)];
    if alert_node(&alert.key).is_some() {
        hints.push(hint(".", "all actions", 8));
    }
    if can_message(app, &alert.key) {
        hints.push(hint("m", "message", 7));
    }
    match alert.key {
        AlertKey::Blocked { .. } => hints.push(hint("o", "open task", 6)),
        AlertKey::Proposal(_) => {}
        _ => hints.push(hint("o", "open run", 6)),
    }
    hints
}

/// Every row of `alert`'s detail at `width`: the header, the labelled rows, a blank
/// row, the actions and the hints. The reducer pages over the same rows.
pub(crate) fn detail_lines(app: &App, alert: &Alert, width: u16) -> Vec<Line<'static>> {
    let w = usize::from(width);
    if w == 0 {
        return Vec::new();
    }
    let p = app.palette();
    let mut out = vec![header(app, alert, w)];
    out.extend(kit::labelled_rows(&facts(app, alert), width, p));
    out.push(Line::default());
    out.extend(actions_line(app, alert, w));
    out.push(kit::hints(width, &detail_hints(app, alert), p));
    out
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
        let at = format!(" {}/{} ", i + 1, all.len());
        block = block.title_top(Line::from(at).right_aligned());
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

#[cfg(test)]
#[path = "alerts_view_tests.rs"]
mod tests;
