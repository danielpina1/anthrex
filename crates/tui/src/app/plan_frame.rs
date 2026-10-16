//! Milestone 9.17 decisions 19 and 20 (rulings C7, C20): the plan review's frame, what
//! the plan decided for the user and what it leaves out. The header's `frame` row, the
//! frame's detail rows, and `f`, which toggles the detail between the selected task and
//! the frame. The gate shows the run's frame and every epic's; a hold shows its epics'
//! frames, and the `promotion` hold the run's too. The snapshot carries frames only
//! while they await approval. Every model-written text reaches a row through
//! `labelled_rows` (`one_line`, then folded in ASCII) and is wrapped to the width by
//! display width. Pure: no I/O; rendering is `ui/plan_review.rs`.

use super::App;
use super::plan_review::{BAR, ReviewTarget, detail_lines, review_tasks};
use crate::safe_text::{multi_line, one_line};
use crate::theme;
use crate::ui::kit::labelled_rows;
use crate::ui::tree_view::truncate_in;
use proto::{HoldKind, PlanFrameInfo, RunInfo};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};

/// Decision 19's bold title of the frame's detail.
const TITLE: &str = "Decided for you · Out of scope";
/// What an empty list, or a frame with nothing in it, says.
const NONE_STATED: &str = "none stated";

/// `stacked`'s optional header rows: the round's, the delivery's and the frame's.
pub(crate) type HeaderRows = (Option<String>, Option<String>, Option<FrameRow>);

/// The header's `frame` row: its text, and whether it is in the `Attention` role (both
/// lists empty and no spec Non-goals).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct FrameRow {
    pub text: String,
    pub attention: bool,
}

/// The frames `target` shows: the run's (the gate, or the `promotion` hold), then each
/// shown epic's with its epic id, in the planners' order.
fn shown<'a>(
    run: &'a RunInfo,
    target: &ReviewTarget,
) -> (Option<&'a PlanFrameInfo>, Vec<(&'a str, &'a PlanFrameInfo)>) {
    let (own, epics): (bool, Vec<&str>) = match target {
        ReviewTarget::Gate => (true, run.planners.iter().map(|p| p.epic.as_str()).collect()),
        ReviewTarget::Hold(id) => {
            let Some(hold) = run.holds.iter().find(|h| h.id == *id) else {
                return (None, Vec::new());
            };
            let mut epics: Vec<&str> = (review_tasks(run, target).into_iter())
                .filter_map(|t| t.epic.as_deref())
                .collect();
            if let HoldKind::Epic { epic } = &hold.kind {
                epics.push(epic);
            }
            (matches!(hold.kind, HoldKind::Promotion), epics)
        }
    };
    let framed = (run.planners.iter())
        .filter(|p| epics.contains(&p.epic.as_str()))
        .filter_map(|p| Some((p.epic.as_str(), p.frame.as_ref()?)))
        .collect();
    (run.plan_frame.as_ref().filter(|_| own), framed)
}

/// The spec's Non-goals, a row a non-blank line.
fn non_goals(frame: &PlanFrameInfo) -> Vec<String> {
    let text = frame.spec_non_goals.as_deref().map(multi_line);
    (text.iter().flat_map(|t| t.split('\n')))
        .map(|l| one_line(l).trim_end().to_owned())
        .filter(|l| !l.trim().is_empty())
        .collect()
}

/// Decision 19's header row for `target`, or `None` when it shows no frame:
/// `frame  [spec non-goals · ]<d> decided for you · <o> out of scope · f shows them`,
/// or `frame  none stated` in `Attention`.
pub(crate) fn frame_row(run: &RunInfo, target: &ReviewTarget, ascii: bool) -> Option<FrameRow> {
    let (own, epics) = shown(run, target);
    if own.is_none() && epics.is_empty() {
        return None;
    }
    let all = || own.into_iter().chain(epics.iter().map(|(_, f)| *f));
    let decided: usize = all().map(|f| f.decided.len()).sum();
    let out: usize = all().map(|f| f.out_of_scope.len()).sum();
    let spec = own.is_some_and(|f| !non_goals(f).is_empty());
    if decided == 0 && out == 0 && !spec {
        let text = format!("frame  {NONE_STATED}");
        return Some(FrameRow {
            text,
            attention: true,
        });
    }
    let spec = if spec { "spec non-goals · " } else { "" };
    let text =
        format!("frame  {spec}{decided} decided for you · {out} out of scope · f shows them");
    Some(FrameRow {
        text: theme::fold(&text, ascii),
        attention: false,
    })
}

/// One labelled group: the label on its first row, `none stated` when empty.
fn group(rows: &mut Vec<(String, String)>, label: String, items: Vec<String>) {
    let items = if items.is_empty() {
        vec![NONE_STATED.to_owned()]
    } else {
        items
    };
    for (n, item) in items.into_iter().enumerate() {
        let label = if n == 0 { label.clone() } else { String::new() };
        rows.push((label, item));
    }
}

/// Decision 19's frame detail in a detail area `width` columns wide (the bar's column
/// left out, as `detail_lines`): the bold title, then `decided` (`<q> → <a>`, `->` in
/// ASCII), `out of scope` and `spec non-goals` for the run's frame, and `<epic> decided`
/// and `<epic> out of scope` for each shown epic's.
pub(crate) fn frame_lines(
    run: &RunInfo,
    target: &ReviewTarget,
    width: u16,
    p: theme::Palette,
) -> Vec<Line<'static>> {
    let width = width.saturating_sub(BAR);
    let decided = |f: &PlanFrameInfo| -> Vec<String> {
        (f.decided.iter())
            .map(|d| format!("{} → {}", one_line(&d.question), one_line(&d.answer)))
            .collect()
    };
    let out = |f: &PlanFrameInfo| f.out_of_scope.iter().map(|o| one_line(o)).collect();
    let (own, epics) = shown(run, target);
    let mut rows = Vec::new();
    if let Some(frame) = own {
        group(&mut rows, "decided".into(), decided(frame));
        group(&mut rows, "out of scope".into(), out(frame));
        let goals = non_goals(frame);
        if !goals.is_empty() {
            group(&mut rows, "spec non-goals".into(), goals);
        }
    }
    for (epic, frame) in epics {
        let epic = one_line(epic);
        group(&mut rows, format!("{epic} decided"), decided(frame));
        group(&mut rows, format!("{epic} out of scope"), out(frame));
    }
    let title = truncate_in(&theme::fold(TITLE, p.ascii), usize::from(width), p.ascii);
    let bold = Style::default().add_modifier(Modifier::BOLD);
    let mut lines = vec![Line::from(Span::styled(title, bold))];
    lines.extend(labelled_rows(&rows, width, p));
    lines
}

impl App {
    /// Whether the open review shows a frame row (and so `f` has something to show).
    pub(crate) fn review_has_frame(&self) -> bool {
        let Some(review) = self.plan_review.as_ref() else {
            return false;
        };
        (self.runs.runs.iter())
            .find(|run| run.run_id == review.run_id)
            .is_some_and(|run| frame_row(run, &review.target, false).is_some())
    }

    /// The open review's detail rows at `width`: the frame's while it shows, else the
    /// selected task's. The renderer draws them and the page keys count them.
    pub(crate) fn review_detail(&self, width: u16) -> Vec<Line<'static>> {
        let Some(review) = self.plan_review.as_ref() else {
            return Vec::new();
        };
        let Some(run) = self.runs.runs.iter().find(|r| r.run_id == review.run_id) else {
            return Vec::new();
        };
        if review.frame && frame_row(run, &review.target, false).is_some() {
            return frame_lines(run, &review.target, width, self.palette());
        }
        let selected = review.selected.as_deref();
        (review_tasks(run, &review.target).into_iter())
            .find(|task| Some(task.id.as_str()) == selected)
            .map_or_else(Vec::new, |task| {
                detail_lines(run, task, width, self.palette())
            })
    }

    /// Decision 19's `f`: the detail flips between the selected task and the frame, from
    /// its top. Nothing without a frame, or on the Plan doc tab (ruling C20).
    pub(super) fn toggle_review_frame(&mut self) {
        if !self.review_has_frame() || self.plan_doc().is_some() {
            return;
        }
        if let Some(review) = self.plan_review.as_mut() {
            review.frame = !review.frame;
            review.scroll = 0;
        }
    }

    /// Decision 19: `j`/`k` leave the frame for the task they select, at its top.
    pub(super) fn leave_review_frame(&mut self) {
        if let Some(review) = self.plan_review.as_mut().filter(|r| r.frame) {
            review.frame = false;
            review.scroll = 0;
        }
    }
}
