//! The run view's node text (milestone 8c, Interfaces "Node content"; decisions 17
//! and 18): one plain-text content row per box, measured by `graph::content_text`
//! before it is painted.

use super::MAX_NODE_WIDTH;
use crate::theme::{Glyph, glyph};
use crate::tree::{DisplayRound, round_label, run_progress};
use crate::ui::tree_view::truncate_in;
use proto::{
    FullState, PlannerInfo, RunInfo, RunState, Size, StageInfo, TaskInfo, TaskOrigin, TaskState,
};
use unicode_width::UnicodeWidthStr;

/// The widest task text: the widest box less its borders and padding (4), then the
/// glyph and its space (2) — decision 18.
pub(crate) const TASK_TEXT_MAX: usize = MAX_NODE_WIDTH as usize - 4 - 2;

/// `orchestrator  {merged}/{total}` when the run's orchestrator window is listed,
/// else `run {last four of the id}  {merged}/{total}`; cancelled tasks are not counted.
/// A run being planned (milestone 9) says `planning` instead of its progress.
pub(crate) fn run_text(run: &RunInfo, orchestrator: bool) -> String {
    let (merged, total) = run_progress(run);
    let progress = if run.state == RunState::Planning {
        "planning".to_owned()
    } else {
        format!("{merged}/{total}")
    };
    if orchestrator {
        format!("orchestrator  {progress}")
    } else {
        format!("run {}  {progress}", run_short(&run.run_id))
    }
}

/// A run id's last four characters, as `Run::short()` names its windows.
fn run_short(run_id: &str) -> String {
    let skip = run_id.chars().count().saturating_sub(4);
    run_id.chars().skip(skip).collect()
}

/// `planner {epic} {title}  {merged}/{total}` over the tasks of its epic, or
/// `planner {epic}  {merged}/{total}` when the title is blank.
pub(crate) fn planner_text(run: &RunInfo, planner: &PlannerInfo) -> String {
    let (merged, total) = run
        .tasks
        .iter()
        .filter(|task| task.epic.as_deref() == Some(planner.epic.as_str()))
        .filter(|task| task.state != TaskState::Cancelled)
        .fold((0, 0), |(merged, total), task| {
            (
                merged + usize::from(task.state == TaskState::Merged),
                total + 1,
            )
        });
    let title = planner.title.trim();
    if title.is_empty() {
        format!("planner {}  {merged}/{total}", planner.epic)
    } else {
        format!("planner {} {title}  {merged}/{total}", planner.epic)
    }
}

/// A task's text, pre-fitted to [`TASK_TEXT_MAX`] so its size, hub mark and declared
/// deps survive the title's truncation (decision 18). A blank title is left out
/// rather than drawn as a double space.
#[cfg(test)]
pub(crate) fn task_text(task: &TaskInfo) -> String {
    task_text_in(task, false)
}

/// [`task_text`], its hub mark `◆` as `Glyph::Hub` in ASCII (milestone 9.0.7 decision
/// 6), cut with `...`. Its declared deps read `  after t0, t6` (decision 19).
pub(crate) fn task_text_in(task: &TaskInfo, ascii: bool) -> String {
    let mut tail = format!(" {}{}", size_letter(task.size), origin_tag(task.origin));
    if task.hub {
        tail.push(' ');
        tail.push_str(glyph(Glyph::Hub, ascii));
    }
    if !task.deps.is_empty() {
        tail.push_str("  after ");
        tail.push_str(&task.deps.join(", "));
    }
    let id_width = UnicodeWidthStr::width(task.id.as_str());
    let tail_width = UnicodeWidthStr::width(tail.as_str());
    let fixed = id_width + 1 + tail_width;
    let title = task.title.trim();
    if !title.is_empty() && fixed + 2 <= TASK_TEXT_MAX {
        let title = truncate_in(title, TASK_TEXT_MAX - fixed, ascii);
        format!("{} {title}{tail}", task.id)
    } else {
        truncate_in(&format!("{}{tail}", task.id), TASK_TEXT_MAX, ascii)
    }
}

fn size_letter(size: Size) -> &'static str {
    match size {
        Size::S => "S",
        Size::M => "M",
        Size::L => "L",
    }
}

/// Milestone 9.0.7 decision 18 (after 9.1 decision 55): `stage <n>/<N>  tier 3 ` then
/// `✓[ <secs>]` (green), `✗[ <secs>]` (red), `✗ bisecting`, `running`, or `◌` (not run,
/// or no head yet: as `theme::stage_look` reads it), `N` the run's stage count and the
/// time `run_stage::tier_duration`'s. The marks are their ASCII twins in ASCII.
pub(crate) fn stage_text_in(run: &RunInfo, stage: &StageInfo, ascii: bool) -> String {
    let g = |mark| glyph(mark, ascii);
    let secs = stage
        .full
        .secs
        .map(|secs| format!(" {}", crate::inspector::tier_duration(secs)))
        .unwrap_or_default();
    let tier = match stage.full.state {
        _ if stage.head.is_none() => g(Glyph::NotStarted).to_owned(),
        FullState::None => g(Glyph::NotStarted).to_owned(),
        FullState::Running => "running".to_owned(),
        FullState::Green => format!("{}{secs}", g(Glyph::Passed)),
        FullState::Red => format!("{}{secs}", g(Glyph::Failed)),
        FullState::Bisecting => format!("{} bisecting", g(Glyph::Failed)),
    };
    format!("stage {}/{}  tier 3 {tier}", stage.n, run.stages.len())
}

/// The tag after a task made by the engine rather than the plan: ` (bisect)`, ` (sync)`.
fn origin_tag(origin: TaskOrigin) -> &'static str {
    match origin {
        TaskOrigin::Plan => "",
        TaskOrigin::Bisect => " (bisect)",
        TaskOrigin::Sync => " (sync)",
        TaskOrigin::Ci => " (ci)",
        TaskOrigin::Review => " (review)",
    }
}

/// `{round_label} {runtime}`: the runtime as a word, never M6.5's badge (decision 17).
pub(crate) fn round_text(round: &DisplayRound<'_>) -> String {
    let info = round.info;
    format!(
        "{} {}",
        round_label(info.role, info.session, round.number),
        info.route.runtime.label()
    )
}

#[cfg(test)]
#[path = "run_text_tests.rs"]
mod tests;
