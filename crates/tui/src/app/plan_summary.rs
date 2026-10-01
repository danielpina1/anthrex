//! Milestone 9.0.7 decisions 23 and 24: the plan review's pure facts. The summary
//! header (counts, sizes, the budget in tool calls, the critical path), the `owns`
//! overlaps between tasks that can run at the same time, and each task's column cells.
//! Every agent-written string is sanitised here (`safe_text::one_line`) and folded in
//! ASCII; the renderer only aligns and cuts. Pure: no I/O.

use super::plan_review::all_deps;
use crate::inspector::run_format::{size_letter, test_mode_text};
use crate::safe_text::one_line;
use crate::theme::{self, Glyph};
use crate::tree::short_model;
use crate::ui::tree_view::truncate_in;
use proto::{RunInfo, Size, TaskInfo};

/// Sizes read as a sequence (`S+M+S`) up to this many tasks, else as counts.
const SIZE_SEQUENCE_MAX: usize = 8;
/// Overlap rows the header shows before `⚠ <k> more overlaps`.
const OVERLAP_ROWS: usize = 3;

/// `1 task`, `3 tasks`.
fn count(n: usize, word: &str) -> String {
    if n == 1 {
        format!("1 {word}")
    } else {
        format!("{n} {word}s")
    }
}

/// The plan's stage count, as the detail's `stage` row reads it (ruling C-28 (5)): the
/// highest planned stage, or the run's stages when it has more.
pub(crate) fn plan_stages(run: &RunInfo) -> u16 {
    let planned = run.tasks.iter().map(|t| t.stage).max().unwrap_or(1);
    planned.max(u16::try_from(run.stages.len()).unwrap_or(u16::MAX))
}

/// `S+M+S` in plan order for at most eight tasks, else `<k>S <m>M <l>L` with the zero
/// counts left out.
fn sizes(tasks: &[&TaskInfo]) -> String {
    if tasks.len() <= SIZE_SEQUENCE_MAX {
        return tasks
            .iter()
            .map(|t| size_letter(t.size))
            .collect::<Vec<_>>()
            .join("+");
    }
    [Size::S, Size::M, Size::L]
        .into_iter()
        .filter_map(|size| {
            let n = tasks.iter().filter(|t| t.size == size).count();
            (n > 0).then(|| format!("{n}{}", size_letter(size)))
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// The header's first row (decision 23): `<n tasks>[ · <e> epics][ · <s> stages] ·
/// <sizes> · ~<calls> calls[ · critical <t1 › t2 › t3>]`, cut with `…` to `width`, so
/// the critical path, last, is what a narrow screen cuts. The stages are those of the
/// reviewed tasks (milestone 9.0.7 task 6's ruling for a hold); the calls the sum of
/// the budgets the tasks were given.
pub(crate) fn header_line(run: &RunInfo, tasks: &[&TaskInfo], width: u16, ascii: bool) -> String {
    let mut parts = vec![count(tasks.len(), "task")];
    let mut epics: Vec<&str> = tasks.iter().filter_map(|t| t.epic.as_deref()).collect();
    epics.sort_unstable();
    epics.dedup();
    if !epics.is_empty() {
        parts.push(count(epics.len(), "epic"));
    }
    let mut stages: Vec<u16> = tasks.iter().map(|t| t.stage).collect();
    stages.sort_unstable();
    stages.dedup();
    if stages.len() > 1 {
        parts.push(count(stages.len(), "stage"));
    }
    if !tasks.is_empty() {
        parts.push(sizes(tasks));
    }
    let calls: u64 = tasks.iter().map(|t| u64::from(t.budget.tool_calls)).sum();
    parts.push(format!("~{calls} calls"));
    if !run.critical_path.is_empty() {
        let path: Vec<String> = run.critical_path.iter().map(|id| one_line(id)).collect();
        parts.push(format!("critical {}", path.join(" › ")));
    }
    let text = theme::fold(&one_line(&parts.join(" · ")), ascii);
    truncate_in(&text, usize::from(width), ascii)
}

/// Two tasks that can run at the same time and both own `path` (decision 23).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Overlap {
    pub a: String,
    pub b: String,
    pub path: String,
}

/// The path two `owns` entries share: the same entry, or the longer one when the
/// shorter is its directory at a `/` boundary.
fn shared<'a>(a: &'a str, b: &'a str) -> Option<&'a str> {
    let (a, b) = (a.trim(), b.trim());
    if a.is_empty() || b.is_empty() {
        return None;
    }
    if a == b {
        return Some(a);
    }
    let (short, long) = if a.len() <= b.len() { (a, b) } else { (b, a) };
    let boundary = short.ends_with('/') || long[short.len()..].starts_with('/');
    (long.starts_with(short) && boundary).then_some(long)
}

/// Whether `from` reaches `to` through deps and implicit deps, transitively, among
/// `tasks`.
fn reaches(tasks: &[&TaskInfo], from: &str, to: &str) -> bool {
    let mut seen: Vec<&str> = Vec::new();
    let mut stack = vec![from];
    while let Some(id) = stack.pop() {
        let Some(task) = tasks.iter().find(|t| t.id == id) else {
            continue;
        };
        for dep in all_deps(task) {
            if dep == to {
                return true;
            }
            if !seen.contains(&dep) {
                seen.push(dep);
                stack.push(dep);
            }
        }
    }
    false
}

/// Decision 23's overlaps, one a pair of tasks in plan order (the first path they
/// share): an `owns` entry equal to, or a directory of, one of the other's, between
/// tasks neither of which reaches the other, so they can run at the same time.
pub(crate) fn overlaps(tasks: &[&TaskInfo]) -> Vec<Overlap> {
    let mut out = Vec::new();
    for (i, a) in tasks.iter().enumerate() {
        for b in &tasks[i + 1..] {
            let path = a
                .owns
                .iter()
                .find_map(|x| b.owns.iter().find_map(|y| shared(x, y)));
            let Some(path) = path else {
                continue;
            };
            if reaches(tasks, &a.id, &b.id) || reaches(tasks, &b.id, &a.id) {
                continue;
            }
            out.push(Overlap {
                a: one_line(&a.id),
                b: one_line(&b.id),
                path: one_line(path),
            });
        }
    }
    out
}

/// The header's warning rows: `⚠ <a> and <b> both own <path>`, at most three, then
/// `⚠ <k> more overlaps`. Folded in ASCII; the renderer cuts them to the width.
pub(crate) fn overlap_lines(overlaps: &[Overlap], ascii: bool) -> Vec<String> {
    let warn = theme::glyph(Glyph::Warning, ascii);
    let mut out: Vec<String> = overlaps
        .iter()
        .take(OVERLAP_ROWS)
        .map(|o| format!("{warn} {} and {} both own {}", o.a, o.b, o.path))
        .collect();
    let more = overlaps.len().saturating_sub(OVERLAP_ROWS);
    if more > 0 {
        let s = if more == 1 { "" } else { "s" };
        out.push(format!("{warn} {more} more overlap{s}"));
    }
    out.into_iter().map(|l| theme::fold(&l, ascii)).collect()
}

/// The header's rows: its first row and the warning rows.
pub(crate) fn header_rows(tasks: &[&TaskInfo]) -> u16 {
    let n = overlaps(tasks).len();
    let warnings = n.min(OVERLAP_ROWS) + usize::from(n > OVERLAP_ROWS);
    1 + u16::try_from(warnings).unwrap_or(u16::MAX)
}

/// `after t1, t3 (implied)`: the explicit deps, then each implicit one marked; empty
/// for a task with none (decisions 19 and 24).
pub(crate) fn after_text(task: &TaskInfo) -> String {
    let deps: Vec<String> = all_deps(task)
        .into_iter()
        .map(|dep| {
            let id = one_line(dep);
            if task.deps.iter().any(|d| d == dep) {
                id
            } else {
                format!("{id} (implied)")
            }
        })
        .collect();
    if deps.is_empty() {
        String::new()
    } else {
        format!("after {}", deps.join(", "))
    }
}

/// One task's list cells (decision 24), sanitised and folded, before the renderer
/// aligns and cuts them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Cells {
    pub id: String,
    pub title: String,
    /// `<runtime tag> <short model>`, `default` for an empty model: `cl opus`.
    pub route: String,
    /// `<S|M|L> <mode>`.
    pub size: String,
    /// `stage <n>`, in a multi-stage plan only.
    pub stage: Option<String>,
    /// `after …`, or empty.
    pub deps: String,
}

/// Every reviewed task's cells, in plan order.
pub(crate) fn columns(run: &RunInfo, tasks: &[&TaskInfo], ascii: bool) -> Vec<Cells> {
    let multi = plan_stages(run) > 1;
    let text = |s: &str| theme::fold(&one_line(s), ascii);
    tasks
        .iter()
        .map(|task| {
            let route = &task.route;
            let model = one_line(&route.model);
            let model = if model.trim().is_empty() {
                "default".to_owned()
            } else {
                short_model(route.runtime, model.trim())
            };
            Cells {
                id: text(&task.id),
                title: text(one_line(&task.title).trim()),
                route: text(&format!("{} {model}", theme::runtime_tag(route.runtime))),
                size: format!(
                    "{} {}",
                    size_letter(task.size),
                    test_mode_text(task.test_mode)
                ),
                stage: multi.then(|| format!("stage {}", task.stage)),
                deps: text(&after_text(task)),
            }
        })
        .collect()
}
