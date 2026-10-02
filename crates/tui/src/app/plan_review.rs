//! Milestone 9.0.5 decisions 11–16: the plan review's pure state and keys. It reviews
//! a run at its gate, or one approval hold, from the snapshot alone (decision 11); its
//! `a`, `x`, `e` and `d` are the run view's own handlers, given the review's selection
//! (decision 16). `Keymap::review_mode` is kept in step with `App.plan_review` here,
//! in one place (Risks 1). Rendering is `ui/plan_review.rs`; this file does no I/O.

use super::plan_summary::{overlap_lines, overlaps, plan_stages};
use super::{App, Effect};
use crate::inspector::run_format::after_text;
use crate::inspector::run_format::{effort_text, strength_text, test_mode_text};
use crate::safe_text::{multi_line, one_line};
use crate::theme::{self, Glyph, Palette};
use crate::tree::{NodeKey, awaiting_holds, task_held};
use crate::ui::kit::labelled_rows;
use crate::ui::tree_view::truncate_in;
use crossterm::event::{KeyCode, KeyEvent};
use proto::{HoldState, Route, RunInfo, RunState, TaskInfo, TaskState};
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};

/// Decision 11: what the review is of.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReviewTarget {
    /// The run's plan gate: every task not cancelled.
    Gate,
    /// One approval hold, by id: its tasks only, in plan order.
    Hold(String),
}

/// Decision 11: the open review. `selected` is a task id; `scroll` the detail's first
/// row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlanReview {
    pub run_id: String,
    pub target: ReviewTarget,
    pub selected: Option<String>,
    pub scroll: u16,
}

/// The toast of `C-b a` while the review is open (decision 13).
pub(crate) const LEAVE_REVIEW_FIRST: &str = "leave the plan review first (esc)";

/// The tasks `target` reviews, in plan order: at the gate every task not cancelled (a
/// dropped task leaves the list), for a hold the hold's own tasks.
pub(crate) fn review_tasks<'a>(run: &'a RunInfo, target: &ReviewTarget) -> Vec<&'a TaskInfo> {
    match target {
        ReviewTarget::Gate => run
            .tasks
            .iter()
            .filter(|task| task.state != TaskState::Cancelled)
            .collect(),
        ReviewTarget::Hold(id) => {
            let Some(hold) = run.holds.iter().find(|hold| hold.id == *id) else {
                return Vec::new();
            };
            run.tasks
                .iter()
                .filter(|task| hold.tasks.contains(&task.id))
                .collect()
        }
    }
}

/// Decision 15: the review still has something to approve.
fn still_awaiting(run: &RunInfo, target: &ReviewTarget) -> bool {
    match target {
        ReviewTarget::Gate => run.state == RunState::AwaitingApproval,
        ReviewTarget::Hold(id) => run
            .holds
            .iter()
            .any(|hold| hold.id == *id && hold.state == HoldState::Awaiting),
    }
}

/// Milestone 9.0.7 decision 26's geometry inside the frame `body`, top to bottom: the
/// summary header, a rule, the task list (its rows, at most `max(3, interior / 2)`), a
/// rule, and the detail in the rest. The renderer and the reducer both use it, so a
/// page is the height the user sees.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub(crate) struct ReviewLayout {
    /// The frame's interior.
    pub inner: Rect,
    pub header: Rect,
    /// Ruling R-13: a `pr` run's gate's `delivered as …` row
    /// (`plan_summary::delivery_line`), under the summary row.
    pub delivery: Option<String>,
    /// The header's `⚠` rows (`plan_summary::overlap_lines`), computed once a frame:
    /// the header is the summary row, the delivery row and these.
    pub warnings: Vec<String>,
    /// The rows of the two `├─…─┤` rules, below the header and below the list, while
    /// they fit.
    pub rules: [Option<u16>; 2],
    pub list: Rect,
    pub detail: Rect,
}

/// The columns the header and the detail are indented by: the list's selection bar's,
/// so every text starts in one column.
pub(crate) const BAR: u16 = 1;

/// Decision 26's stacking of a header (the summary row and `warnings`) and a list of
/// `tasks` in `body`.
pub(crate) fn stacked(
    body: Rect,
    delivery: Option<String>,
    warnings: Vec<String>,
    tasks: usize,
) -> ReviewLayout {
    let header_rows = u16::try_from(warnings.len() + usize::from(delivery.is_some()))
        .unwrap_or(u16::MAX)
        .saturating_add(1);
    let inner = Rect {
        x: body.x.saturating_add(1),
        y: body.y.saturating_add(1),
        width: body.width.saturating_sub(2),
        height: body.height.saturating_sub(2),
    };
    let mut y = inner.y;
    let mut take = |rows: u16| {
        let height = rows.min(inner.bottom() - y);
        let area = Rect { y, height, ..inner };
        y += height;
        area
    };
    let header = take(header_rows);
    let first = take(1);
    let list_rows = u16::try_from(tasks)
        .unwrap_or(u16::MAX)
        .min((inner.height / 2).max(3));
    let list = take(list_rows);
    let second = take(1);
    let detail = take(u16::MAX);
    let rule = |area: Rect| (area.height == 1).then_some(area.y);
    ReviewLayout {
        inner,
        header,
        delivery,
        warnings,
        rules: [rule(first), rule(second)],
        list,
        detail,
    }
}

/// `<runtime> · <model> · <strength> · <effort> effort` (decision 12).
/// A blank model (the policy's default) is left out.
fn route_line(route: &Route) -> String {
    let effort = format!("{} effort", effort_text(route.effort));
    [
        route.runtime.label(),
        route.model.as_str(),
        strength_text(route.strength),
        effort.as_str(),
    ]
    .into_iter()
    .filter(|part| !part.trim().is_empty())
    .collect::<Vec<_>>()
    .join(" · ")
}

/// A task's deps, explicit then implicit, without repeats.
pub(crate) fn all_deps(task: &TaskInfo) -> Vec<&str> {
    let mut deps: Vec<&str> = Vec::new();
    for dep in task.deps.iter().chain(&task.implicit_deps) {
        if !deps.contains(&dep.as_str()) {
            deps.push(dep);
        }
    }
    deps
}

/// An empty list reads `none`, so its label never stands alone.
fn or_none(lines: Vec<String>) -> Vec<String> {
    if lines.is_empty() {
        vec!["none".to_owned()]
    } else {
        lines
    }
}

/// The sections of decision 12, unwrapped: each label with its lines of text.
fn sections(run: &RunInfo, task: &TaskInfo, ascii: bool) -> Vec<(&'static str, Vec<String>)> {
    let pending = theme::glyph(Glyph::NotStarted, ascii);
    let mut out = vec![
        // One row a line of the brief, each through `one_line`.
        (
            "brief",
            multi_line(&task.brief).split('\n').map(one_line).collect(),
        ),
        // One entry is one row: a line break inside an entry must not forge another.
        (
            "owns",
            or_none(task.owns.iter().map(|o| one_line(o)).collect()),
        ),
        (
            "done when",
            or_none(
                task.acceptance
                    .iter()
                    .map(|c| format!("{pending} {}", one_line(c)))
                    .collect(),
            ),
        ),
    ];
    let mode = test_mode_text(task.test_mode);
    let mode = match task.test_mode_reason.as_deref() {
        Some(reason) if !reason.trim().is_empty() => format!("{mode} — {reason}"),
        _ => mode.to_owned(),
    };
    out.push(("test mode", vec![mode]));
    out.extend(stage_sections(run, task));
    if !task.notes.is_empty() {
        out.push(("notes", task.notes.iter().map(|n| one_line(n)).collect()));
    }
    // Milestone 9.0.7 decision 19's form, then what the task unblocks, on one row.
    let mut deps = Vec::new();
    let after = after_text(task);
    if !after.is_empty() {
        deps.push(after);
    }
    let unblocks: Vec<String> = run
        .tasks
        .iter()
        .filter(|other| all_deps(other).contains(&task.id.as_str()))
        .map(|other| one_line(&other.id))
        .collect();
    if !unblocks.is_empty() {
        deps.push(format!("unblocks {}", unblocks.join(", ")));
    }
    if !deps.is_empty() {
        out.push(("deps", vec![deps.join(" · ")]));
    }
    out.push(("route", vec![route_line(&task.route)]));
    let review = task
        .review_route
        .as_ref()
        .map_or_else(|| "none".to_owned(), route_line);
    out.push(("review", vec![review]));
    out
}

/// Ruling C-28 (5): a Multi plan's stage facts for `task`: `stage  <n> of <m>`, then
/// `atomic` (the plan's reason, else `yes`) and `interface change` when set. A Single
/// plan has none, so its review is as it was.
fn stage_sections(run: &RunInfo, task: &TaskInfo) -> Vec<(&'static str, Vec<String>)> {
    let stages = plan_stages(run);
    if stages <= 1 {
        return Vec::new();
    }
    let mut out = vec![("stage", vec![format!("{} of {stages}", task.stage)])];
    if task.atomic {
        let reason = task
            .atomic_reason
            .as_deref()
            .filter(|r| !r.trim().is_empty())
            .unwrap_or("yes");
        out.push(("atomic", vec![reason.to_owned()]));
    }
    if task.interface_change {
        out.push(("interface change", vec!["yes".to_owned()]));
    }
    out
}

/// Milestone 9.0.7 decision 25's detail for `task` in a detail area `width` columns
/// wide (the bar's column left out): the bold `<id>  <title>` row, as the task panel
/// leads with its title, then `kit::labelled_rows` of decision 12's sections in their
/// order, an entry a row (a label on its first), each value wrapped under itself.
/// Every agent-written string goes through `multi_line` and `one_line` (decision 27);
/// `labelled_rows` folds in ASCII. The reducer pages by these rows' count.
pub(crate) fn detail_lines(
    run: &RunInfo,
    task: &TaskInfo,
    width: u16,
    p: Palette,
) -> Vec<Line<'static>> {
    let width = width.saturating_sub(BAR);
    let title = theme::fold(&one_line(&format!("{}  {}", task.id, task.title)), p.ascii);
    let bold = Style::default().add_modifier(Modifier::BOLD);
    let title = truncate_in(&title, usize::from(width), p.ascii);
    let mut rows = Vec::new();
    for (label, texts) in sections(run, task, p.ascii) {
        for (n, text) in texts.into_iter().enumerate() {
            let label = if n == 0 { label } else { "" };
            rows.push((label.to_owned(), text));
        }
    }
    let mut out = vec![Line::from(Span::styled(title, bold))];
    out.extend(labelled_rows(&rows, width, p));
    out
}

impl App {
    /// The renderer's body area (the review's frame), reported after every draw.
    pub fn set_body_area(&mut self, area: Rect) {
        self.body_area = area;
    }

    /// Decision 14: opens the review of `target` on `run_id`, its first task selected,
    /// over whatever screen shows; `Esc` gives that screen back. The alerts' Enter and
    /// the run view's `p` both come here.
    pub(crate) fn open_plan_review(&mut self, run_id: String, target: ReviewTarget) -> Vec<Effect> {
        let selected = self
            .runs
            .runs
            .iter()
            .find(|run| run.run_id == run_id)
            .and_then(|run| {
                review_tasks(run, &target)
                    .first()
                    .map(|task| task.id.clone())
            });
        self.plan_review = Some(PlanReview {
            run_id,
            target,
            selected,
            scroll: 0,
        });
        self.sync_review_mode();
        vec![]
    }

    /// Decision 13: `Esc`. Only the review and its keymap mode close, so the screen
    /// underneath, its tree mode and its selection are as they were.
    pub(crate) fn close_plan_review(&mut self) {
        self.plan_review = None;
        self.sync_review_mode();
    }

    /// Risks 1: the keymap's review mode follows `plan_review`, here only.
    fn sync_review_mode(&mut self) {
        self.keymap.set_review_mode(self.plan_review.is_some());
    }

    /// The reviewed run and the review's tasks, while both are listed.
    fn reviewed(&self) -> Option<(&RunInfo, Vec<&TaskInfo>)> {
        let review = self.plan_review.as_ref()?;
        let run = self
            .runs
            .runs
            .iter()
            .find(|run| run.run_id == review.run_id)?;
        Some((run, review_tasks(run, &review.target)))
    }

    /// Decision 13's keys: `j`/`k` select, the page keys scroll, `a` `x` `e` `d` are the
    /// run view's handlers given the review's task (decision 16), `Esc` closes. Every
    /// other key does nothing.
    pub(super) fn on_review_key(&mut self, key: KeyEvent) -> Vec<Effect> {
        let Some(review) = self.plan_review.as_ref() else {
            return vec![];
        };
        match key.code {
            KeyCode::Esc => self.close_plan_review(),
            KeyCode::Char('j') | KeyCode::Down => self.move_review_selection(1),
            KeyCode::Char('k') | KeyCode::Up => self.move_review_selection(-1),
            KeyCode::PageDown => self.scroll_review(true),
            KeyCode::PageUp => self.scroll_review(false),
            // Review finding 6: a hold review's `a` and `x` decide the hold under
            // review, whatever hold its selected task names.
            KeyCode::Char(c @ ('a' | 'x')) if matches!(review.target, ReviewTarget::Hold(_)) => {
                let ReviewTarget::Hold(hold) = review.target.clone() else {
                    return vec![];
                };
                let run_id = review.run_id.clone();
                return self.decide_hold(&run_id, c, &hold);
            }
            KeyCode::Char(c @ ('a' | 'x' | 'e' | 'd')) => {
                let run_id = review.run_id.clone();
                let selected = review.selected.clone().map(|id| NodeKey::Task {
                    run: run_id.clone(),
                    id,
                });
                return self.on_gate_key(run_id, c, selected);
            }
            _ => {}
        }
        vec![]
    }

    /// `j`/`k`: the next or previous task, stopping at the ends; a new selection starts
    /// at the top of its detail.
    fn move_review_selection(&mut self, delta: isize) {
        let Some((_, tasks)) = self.reviewed() else {
            return;
        };
        let ids: Vec<String> = tasks.iter().map(|task| task.id.clone()).collect();
        let Some(review) = self.plan_review.as_mut() else {
            return;
        };
        let at = review
            .selected
            .as_ref()
            .and_then(|id| ids.iter().position(|other| other == id));
        let next = match at {
            Some(at) => at
                .saturating_add_signed(delta)
                .min(ids.len().saturating_sub(1)),
            None => 0,
        };
        if let Some(id) = ids.get(next)
            && review.selected.as_ref() != Some(id)
        {
            review.selected = Some(id.clone());
            review.scroll = 0;
        }
    }

    /// The page keys: by the detail's height minus one, clamped to its content, by
    /// the same row count the renderer draws (`detail_lines`).
    ///
    /// The scroll is clamped first (review finding 3): content that shrank, or a body
    /// that grew, since the last step leaves it past the end, and a page up must step
    /// from the end the user sees.
    fn scroll_review(&mut self, down: bool) {
        let detail = self.review_layout(self.body_area).detail;
        let (Some(max), Some(review)) =
            (self.review_max_scroll_in(detail), self.plan_review.as_mut())
        else {
            return;
        };
        let page = detail.height.saturating_sub(1).max(1);
        let from = review.scroll.min(max);
        review.scroll = if down {
            from.saturating_add(page).min(max)
        } else {
            from.saturating_sub(page)
        };
    }

    /// The last first row of the selected task's detail at `body_area`: its rows, by
    /// the renderer's own count, less the detail's height.
    fn review_max_scroll(&self) -> Option<u16> {
        self.review_max_scroll_in(self.review_layout(self.body_area).detail)
    }

    /// [`Self::review_max_scroll`] for a `detail` area the caller already laid out, so a
    /// page key lays the review out once (final fix wave I3).
    fn review_max_scroll_in(&self, detail: Rect) -> Option<u16> {
        let (run, tasks) = self.reviewed()?;
        let selected = self.plan_review.as_ref()?.selected.as_ref()?;
        let task = tasks.into_iter().find(|task| task.id == *selected)?;
        let rows = detail_lines(run, task, detail.width, self.palette()).len();
        Some(
            u16::try_from(rows)
                .unwrap_or(u16::MAX)
                .saturating_sub(detail.height),
        )
    }

    /// Decision 26's geometry of the open review in the frame `body`: the header's
    /// warnings and the list's rows from the reviewed tasks (`stacked`).
    pub(crate) fn review_layout(&self, body: Rect) -> ReviewLayout {
        let ascii = self.palette().ascii;
        let gate = (self.plan_review.as_ref()).is_some_and(|r| r.target == ReviewTarget::Gate);
        let (delivery, warnings, tasks) =
            self.reviewed()
                .map_or((None, Vec::new(), 0), |(run, tasks)| {
                    (
                        super::plan_summary::delivery_line(run, gate, ascii),
                        overlap_lines(&overlaps(run, &tasks), ascii),
                        tasks.len(),
                    )
                });
        stacked(body, delivery, warnings, tasks)
    }

    /// Decision 14's `p` in the run view on `run_id`: the gate while the run awaits
    /// approval, else the selected held task's awaiting hold, else the oldest awaiting
    /// hold, else a toast.
    pub(super) fn review_from_run_view(&mut self, run_id: &str) -> Vec<Effect> {
        let Some(run) = self.runs.runs.iter().find(|run| run.run_id == run_id) else {
            return vec![];
        };
        let selected_hold = match &self.tree.selected {
            Some(NodeKey::Task { run: r, id }) if r == run_id => run
                .tasks
                .iter()
                .find(|task| task.id == *id && task_held(run, task))
                .and_then(|task| task.hold.as_ref())
                .filter(|hold| awaiting_holds(run).any(|h| h.id == **hold)),
            _ => None,
        };
        let oldest = || {
            awaiting_holds(run)
                .enumerate()
                .min_by_key(|(at, hold)| (hold.created_at, *at))
                .map(|(_, hold)| &hold.id)
        };
        let target = if run.state == RunState::AwaitingApproval {
            ReviewTarget::Gate
        } else if let Some(hold) = selected_hold.or_else(oldest) {
            ReviewTarget::Hold(hold.clone())
        } else {
            self.toast(format!("run {run_id} has nothing awaiting approval"));
            return vec![];
        };
        self.open_plan_review(run_id.to_owned(), target)
    }

    /// Before a snapshot replaces the runs: the selected task's position in the list.
    pub(super) fn review_position(&self) -> Option<usize> {
        let selected = self.plan_review.as_ref()?.selected.as_ref()?;
        let (_, tasks) = self.reviewed()?;
        tasks.iter().position(|task| task.id == *selected)
    }

    /// Decision 15, after every snapshot: a review with nothing left to approve, or of
    /// a run that is gone, closes as `Esc` would, without a toast. A selected task that
    /// left the list (dropped at the gate) gives way to the task now at its position.
    pub(super) fn follow_review(&mut self, position: Option<usize>) {
        let Some(review) = self.plan_review.as_ref() else {
            return;
        };
        let open = self
            .runs
            .runs
            .iter()
            .find(|run| run.run_id == review.run_id)
            .is_some_and(|run| still_awaiting(run, &review.target));
        if !open {
            self.close_plan_review();
            return;
        }
        let Some((_, tasks)) = self.reviewed() else {
            return;
        };
        let ids: Vec<String> = tasks.iter().map(|task| task.id.clone()).collect();
        let Some(review) = self.plan_review.as_mut() else {
            return;
        };
        if review.selected.as_ref().is_some_and(|id| ids.contains(id)) {
            // Review finding 3: the same task, perhaps shorter now.
            if let Some(max) = self.review_max_scroll()
                && let Some(review) = self.plan_review.as_mut()
            {
                review.scroll = review.scroll.min(max);
            }
            return;
        }
        let at = position.unwrap_or(0).min(ids.len().saturating_sub(1));
        review.selected = ids.get(at).cloned();
        review.scroll = 0;
    }
}
