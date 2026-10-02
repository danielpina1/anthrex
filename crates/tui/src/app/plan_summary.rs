//! Milestone 9.0.7 decisions 23 and 24: the plan review's pure facts. The summary
//! header (counts, sizes, the budget in tool calls, the critical path), the `owns`
//! overlaps between tasks that can run at the same time, each task's column cells, and
//! the columns' widths at a list width (`Fit`). Every agent-written string is sanitised
//! here (`safe_text::one_line`) and folded in ASCII; the renderer only draws and cuts.
//! Pure: no I/O.

use super::plan_review::{BAR, all_deps};
use crate::inspector::run_format::{route_tag, size_letter, test_mode_text};
use crate::safe_text::one_line;
use crate::theme::{self, Glyph};
use crate::ui::tree_view::truncate_in;
use proto::{RunInfo, Size, TaskInfo};
use std::collections::HashMap;

/// Sizes read as a sequence (`S+M+S`) up to this many tasks, else as counts.
const SIZE_SEQUENCE_MAX: usize = 8;
/// Overlap rows the header shows before `⚠ <k> more overlaps`.
const OVERLAP_ROWS: usize = 3;
/// The title columns a task keeps before the route tag goes (decision 24).
const MIN_TITLE: usize = 8;
/// The columns between two list columns.
pub(crate) const GAP: usize = 2;

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

/// Two tasks that can run at the same time and both own `path` (decision 23), as the
/// snapshot has them (unsanitised).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Overlap<'a> {
    pub a: &'a str,
    pub b: &'a str,
    pub path: &'a str,
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
    // `starts_with` first, and the boundary byte read, never sliced: `short.len()` can
    // fall inside a multi-byte character of `long` (`src/a` against `src/é`).
    let boundary = short.ends_with('/') || long.as_bytes().get(short.len()) == Some(&b'/');
    (long.starts_with(short) && boundary).then_some(long)
}

/// Final fix wave I3: the work `overlaps` does, counted in tests (a task visited or a
/// dep followed while ordering tasks, an `owns` pair compared), so a test bounds the
/// operations rather than the wall clock (`docs/timing-budgets.md`).
#[cfg(test)]
pub(crate) mod work {
    use std::cell::Cell;
    thread_local! {
        static STEPS: Cell<usize> = const { Cell::new(0) };
    }
    pub(crate) fn add(n: usize) {
        STEPS.with(|s| s.set(s.get().saturating_add(n)));
    }
    /// The steps since the last `take`, on this thread.
    pub(crate) fn take() -> usize {
        STEPS.with(|s| s.replace(0))
    }
}

fn step() {
    #[cfg(test)]
    work::add(1);
}

/// Which tasks each task reaches through deps and implicit deps, transitively, among
/// every task of the run (a hold's tasks can be ordered through tasks outside it):
/// decision 23's "neither reaches the other". Built once per call to `overlaps` (final
/// fix wave I3): one walk a task over an id map, into a bitset a task, so a pair is
/// two bit reads instead of two searches of the run.
struct Reach<'a> {
    index: HashMap<&'a str, usize>,
    /// `bits[i]`'s bit `j`: task `i` reaches task `j`.
    bits: Vec<Vec<u64>>,
}

impl<'a> Reach<'a> {
    fn of(tasks: &'a [TaskInfo]) -> Self {
        let index: HashMap<&str, usize> = tasks
            .iter()
            .enumerate()
            .map(|(i, t)| (t.id.as_str(), i))
            .collect();
        let deps: Vec<Vec<usize>> = tasks
            .iter()
            .map(|t| {
                all_deps(t)
                    .iter()
                    .filter_map(|d| index.get(d).copied())
                    .collect()
            })
            .collect();
        let words = tasks.len().div_ceil(64);
        let bits = (0..tasks.len())
            .map(|from| {
                let mut seen = vec![0u64; words];
                let mut stack = vec![from];
                while let Some(at) = stack.pop() {
                    step();
                    for &dep in &deps[at] {
                        step();
                        let (word, bit) = (dep / 64, 1u64 << (dep % 64));
                        if seen[word] & bit == 0 {
                            seen[word] |= bit;
                            stack.push(dep);
                        }
                    }
                }
                seen
            })
            .collect();
        Reach { index, bits }
    }

    fn reaches(&self, from: &str, to: &str) -> bool {
        match (self.index.get(from), self.index.get(to)) {
            (Some(&from), Some(&to)) => self.bits[from][to / 64] & (1u64 << (to % 64)) != 0,
            _ => false,
        }
    }
}

/// Decision 23's overlaps, one a pair of the reviewed `tasks` in plan order (the first
/// path they share): an `owns` entry equal to, or a directory of, one of the other's,
/// between tasks neither of which reaches the other through `run`'s tasks, so they can
/// run at the same time. Borrowed from `run`, unsanitised: `overlap_lines` sanitises
/// the few it draws (a plan can have thousands).
pub(crate) fn overlaps<'a>(run: &'a RunInfo, tasks: &[&'a TaskInfo]) -> Vec<Overlap<'a>> {
    let reach = Reach::of(&run.tasks);
    let mut out = Vec::new();
    for (i, a) in tasks.iter().enumerate() {
        for b in &tasks[i + 1..] {
            if reach.reaches(&a.id, &b.id) || reach.reaches(&b.id, &a.id) {
                continue;
            }
            let path = a.owns.iter().find_map(|x| {
                b.owns.iter().find_map(|y| {
                    step();
                    shared(x, y)
                })
            });
            if let Some(path) = path {
                out.push(Overlap {
                    a: &a.id,
                    b: &b.id,
                    path,
                });
            }
        }
    }
    out
}

/// The header's warning rows: `⚠ <a> and <b> both own <path>`, at most three, then
/// `⚠ <k> more overlaps`. Sanitised and folded in ASCII; the renderer cuts them to the
/// width.
pub(crate) fn overlap_lines(overlaps: &[Overlap<'_>], ascii: bool) -> Vec<String> {
    let warn = theme::glyph(Glyph::Warning, ascii);
    let mut out: Vec<String> = overlaps
        .iter()
        .take(OVERLAP_ROWS)
        .map(|o| {
            let (a, b, path) = (one_line(o.a), one_line(o.b), one_line(o.path));
            format!("{warn} {a} and {b} both own {path}")
        })
        .collect();
    let more = overlaps.len().saturating_sub(OVERLAP_ROWS);
    if more > 0 {
        let s = if more == 1 { "" } else { "s" };
        out.push(format!("{warn} {more} more overlap{s}"));
    }
    out.into_iter().map(|l| theme::fold(&l, ascii)).collect()
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
    /// `<runtime tag> <model>`, `default` for an empty model: `cl opus` (the task panel
    /// footer's form, `run_format::route_tag`).
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
        .map(|task| Cells {
            id: text(&task.id),
            title: text(one_line(&task.title).trim()),
            route: text(&route_tag(&task.route)),
            size: format!(
                "{} {}",
                size_letter(task.size),
                test_mode_text(task.test_mode)
            ),
            stage: multi.then(|| format!("stage {}", task.stage)),
            deps: text(&after_text(task)),
        })
        .collect()
}

fn width_of(text: &str) -> usize {
    unicode_width::UnicodeWidthStr::width(text)
}

/// Every column's width once the row is fitted to the list (decision 24): `None` for
/// a dropped column.
pub(crate) struct Fit {
    pub label: usize,
    pub route: Option<usize>,
    pub size: usize,
    pub stage: Option<usize>,
    pub deps: usize,
}

impl Fit {
    /// Each column as wide as its widest value; when the row does not fit, the title
    /// shrinks to `MIN_TITLE` columns first, then the route tag goes, then the stage
    /// column; the title takes back what those freed, and only then are the deps cut.
    pub(crate) fn of(cells: &[Cells], width: usize) -> Self {
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
