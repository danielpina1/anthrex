//! Milestone 9.0.5 decisions 11–16: the plan review's pure state and keys. It reviews
//! a run at its gate, or one approval hold, from the snapshot alone (decision 11); its
//! `a`, `x`, `e` and `d` are the run view's own handlers, given the review's selection
//! (decision 16). `Keymap::review_mode` is kept in step with `App.plan_review` here,
//! in one place (Risks 1). Rendering is `ui/plan_review.rs`; this file does no I/O.

use super::{App, Effect};
use crate::inspector::run_format::{effort_text, strength_text, test_mode_text};
use crate::safe_text::{multi_line, one_line};
use crate::tree::{NodeKey, awaiting_holds, task_held};
use crate::ui::tree_view::truncate;
use crossterm::event::{KeyCode, KeyEvent};
use proto::{HoldState, Route, RunInfo, RunState, TaskInfo, TaskState};
use ratatui::layout::Rect;
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

/// Decision 11: what the review is of.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReviewTarget {
    /// The run's plan gate: every task not cancelled.
    Gate,
    /// One approval hold, by id: its tasks only, in plan order.
    Hold(String),
}

/// Decision 11: the open review. `selected` is a task id; `scroll` the right pane's
/// first row.
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

/// Decision 12's screen areas inside `body`: the title row, the task list on the left
/// (`clamp(width × 2 / 5, 30, 56)` columns) and the selected task's detail on the right,
/// one column of separator between them. The renderer and the reducer both use it, so
/// a page is the height the user sees.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ReviewPanes {
    pub title: Rect,
    pub left: Rect,
    pub right: Rect,
}

pub(crate) fn panes(body: Rect) -> ReviewPanes {
    let title = Rect {
        height: body.height.min(1),
        ..body
    };
    let below = Rect {
        y: body.y + title.height,
        height: body.height - title.height,
        ..body
    };
    let left_width = (below.width * 2 / 5).clamp(30, 56).min(below.width);
    let left = Rect {
        width: left_width,
        ..below
    };
    let gap = u16::from(below.width > left_width);
    let right = Rect {
        x: below.x + left_width + gap,
        width: below.width - left_width - gap,
        ..below
    };
    ReviewPanes { title, left, right }
}

/// What a right-pane row is, for the renderer's styles.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum LineKind {
    /// `<id>  <title>`.
    Title,
    /// A section label (`brief`, `owns`, …).
    Label,
    /// A section's text, indented under its label.
    Text,
    /// The empty row between sections.
    Blank,
}

/// One right-pane row, already sanitised and wrapped to the pane's width.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ReviewLine {
    pub kind: LineKind,
    pub text: String,
}

/// Columns a section's text is indented under its label.
const INDENT: usize = 2;

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
fn sections(run: &RunInfo, task: &TaskInfo) -> Vec<(&'static str, Vec<String>)> {
    let mut out = vec![
        ("brief", vec![task.brief.clone()]),
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
                    .map(|c| format!("☐ {}", one_line(c)))
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
    if !task.notes.is_empty() {
        out.push(("notes", task.notes.iter().map(|n| one_line(n)).collect()));
    }
    let mut deps = Vec::new();
    let after = all_deps(task);
    if !after.is_empty() {
        deps.push(format!("after {}", after.join(", ")));
    }
    let unblocks: Vec<&str> = run
        .tasks
        .iter()
        .filter(|other| all_deps(other).contains(&task.id.as_str()))
        .map(|other| other.id.as_str())
        .collect();
    if !unblocks.is_empty() {
        deps.push(format!("unblocks {}", unblocks.join(", ")));
    }
    if !deps.is_empty() {
        out.push(("deps", deps));
    }
    out.push(("route", vec![route_line(&task.route)]));
    let review = task
        .review_route
        .as_ref()
        .map_or_else(|| "none".to_owned(), route_line);
    out.push(("review", vec![review]));
    out
}

/// Greedy word wrap to `width` display columns; a word wider than the line is broken.
/// A blank line stays one empty row.
pub(crate) fn wrap(text: &str, width: usize) -> Vec<String> {
    let width = width.max(1);
    let mut lines = Vec::new();
    let mut current = String::new();
    for word in text.split_whitespace() {
        let joined = current.width() + 1 + word.width();
        if !current.is_empty() && joined > width {
            lines.push(std::mem::take(&mut current));
        } else if !current.is_empty() {
            current.push(' ');
        }
        current.push_str(word);
        while current.width() > width {
            let (mut used, mut end) = (0, 0);
            for (at, c) in current.char_indices() {
                let w = UnicodeWidthChar::width(c).unwrap_or(0);
                if used + w > width && end > 0 {
                    break;
                }
                used += w;
                end = at + c.len_utf8();
            }
            let rest = current.split_off(end);
            lines.push(std::mem::replace(&mut current, rest));
        }
    }
    if !current.is_empty() || lines.is_empty() {
        lines.push(current);
    }
    lines
}

/// Decision 12's right pane for `task`, wrapped to `width` columns: the title row, then
/// each section's label and its text indented under it, a blank row between sections.
/// Every agent-written string goes through `multi_line`, then `one_line` per line
/// (decision 27).
pub(crate) fn detail_lines(run: &RunInfo, task: &TaskInfo, width: u16) -> Vec<ReviewLine> {
    let width = usize::from(width);
    let line = |kind, text: String| ReviewLine { kind, text };
    // One row, spacing kept, cut with `…`: the list on the left carries the same title.
    let title = one_line(&format!("{}  {}", task.id, task.title));
    let mut out = vec![line(LineKind::Title, truncate(&title, width))];
    for (label, texts) in sections(run, task) {
        out.push(line(LineKind::Blank, String::new()));
        out.push(line(LineKind::Label, label.to_owned()));
        let pad = " ".repeat(INDENT.min(width.saturating_sub(1)));
        for text in texts {
            for raw in multi_line(&text).split('\n') {
                for row in wrap(&one_line(raw), width.saturating_sub(pad.len())) {
                    out.push(line(LineKind::Text, format!("{pad}{row}")));
                }
            }
        }
    }
    out
}

impl App {
    /// The renderer's body area (decision 12), reported after every draw.
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

    /// The page keys: by the right pane's height minus one, clamped to its content, by
    /// the same row count the renderer draws (`detail_lines`).
    ///
    /// The scroll is clamped first (review finding 3): content that shrank, or a body
    /// that grew, since the last step leaves it past the end, and a page up must step
    /// from the end the user sees.
    fn scroll_review(&mut self, down: bool) {
        let right = panes(self.body_area).right;
        let (Some(max), Some(review)) = (self.review_max_scroll(), self.plan_review.as_mut())
        else {
            return;
        };
        let page = right.height.saturating_sub(1).max(1);
        let from = review.scroll.min(max);
        review.scroll = if down {
            from.saturating_add(page).min(max)
        } else {
            from.saturating_sub(page)
        };
    }

    /// The last first row of the selected task's detail at `body_area`: its rows, by
    /// the renderer's own count, less the pane's height.
    fn review_max_scroll(&self) -> Option<u16> {
        let right = panes(self.body_area).right;
        let (run, tasks) = self.reviewed()?;
        let selected = self.plan_review.as_ref()?.selected.as_ref()?;
        let task = tasks.into_iter().find(|task| task.id == *selected)?;
        let rows = detail_lines(run, task, right.width).len();
        Some(
            u16::try_from(rows)
                .unwrap_or(u16::MAX)
                .saturating_sub(right.height),
        )
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
