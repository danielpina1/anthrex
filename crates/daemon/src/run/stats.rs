//! M8b decision 35: `anthrex run stats`, the recorded aggregates of `history.jsonl`:
//! one row per class (`S`, `M` non-hub, `hub`), and the deciders' and the size
//! cross-check's totals. Recording only: nothing is proposed or refitted (M9.5).
//! Pure (M8b decision 1); `history_io::summarise` reads the file and finds reverts.

use std::collections::HashSet;
use std::path::Path;

use proto::{DeciderSource, HistoryLine, HistoryStats, Size, StatsRow, TaskOutcome, TaskRecord};

/// The rows, in order.
const CLASSES: [&str; 3] = ["S", "M", "hub"];

/// The row a task belongs to: `hub` whatever its size, else its final size. A non-hub
/// `L` task (blocked for a split, never merged) belongs to none.
fn class(task: &TaskRecord) -> Option<&'static str> {
    match (task.hub, task.final_size) {
        (true, _) => Some("hub"),
        (false, Size::S) => Some("S"),
        (false, Size::M) => Some("M"),
        (false, Size::L) => None,
    }
}

fn is_merged(task: &TaskRecord) -> bool {
    matches!(
        task.outcome,
        TaskOutcome::Merged | TaskOutcome::MergedWithoutApproval
    )
}

/// The lower middle value (decision 35); `None` for no values.
fn median<T: Ord + Copy>(mut values: Vec<T>) -> Option<T> {
    values.sort_unstable();
    values.get(values.len().checked_sub(1)? / 2).copied()
}

/// Every bounce, gate by gate.
fn bounces(task: &TaskRecord) -> u32 {
    let b = task.bounces;
    [b.done, b.proof, b.check, b.review, b.merge]
        .iter()
        .map(|&n| u32::from(n))
        .sum()
}

/// The task's billable tokens: its workers', reviewers' and deciders'.
fn billable(task: &TaskRecord) -> u64 {
    task.worker_usage
        .billable()
        .saturating_add(task.reviewer_usage.billable())
        .saturating_add(task.decider_usage.billable())
}

/// One class's row over its tasks. Medians are over the merged tasks (lines over
/// those with a measured diff).
fn row(class: &str, tasks: &[&TaskRecord], reverted: &dyn Fn(&TaskRecord) -> bool) -> StatsRow {
    let merged: Vec<&TaskRecord> = tasks.iter().copied().filter(|t| is_merged(t)).collect();
    StatsRow {
        class: class.to_string(),
        tasks: tasks.len() as u32,
        merged: merged.len() as u32,
        median_lines: median(
            merged
                .iter()
                .filter_map(|t| t.diff.map(|d| d.added.saturating_add(d.removed)))
                .collect(),
        ),
        median_tool_calls: median(merged.iter().map(|t| t.tool_calls).collect()),
        median_tokens: median(merged.iter().map(|t| billable(t)).collect()),
        median_work_secs: median(merged.iter().map(|t| t.phases.working).collect()),
        bounces: tasks.iter().map(|t| bounces(t)).sum(),
        reverted: merged.iter().filter(|t| reverted(t)).count() as u32,
    }
}

/// Decision 35's aggregates of `lines` (the last line of each `record_id`, as
/// `history_io::read_history` keeps them). A task record whose task never ran a session
/// (`sessions == 0`, a plan rejected at the gate) says nothing about its class and is
/// left out of the rows (controller ruling, M8b.16 review Q2); it is still one of the
/// task records. A merged task is reverted when a `revert` record names it, or names
/// its run's accept merge (`task_id: None`). `problems` is the caller's.
pub fn aggregate(lines: &[HistoryLine], path: &Path) -> HistoryStats {
    let mut tasks = Vec::new();
    let mut runs = Vec::new();
    let mut task_reverts: HashSet<(&str, &str)> = HashSet::new();
    let mut run_reverts: HashSet<&str> = HashSet::new();
    for line in lines {
        match line {
            HistoryLine::Task(t) => tasks.push(t),
            HistoryLine::Run(r) => runs.push(r),
            // Decision 43: role-routing records never count toward the aggregates.
            HistoryLine::RoleRoute(_) => {}
            HistoryLine::Revert(r) => match &r.task_id {
                Some(task) => {
                    task_reverts.insert((r.run_id.as_str(), task.as_str()));
                }
                None => {
                    run_reverts.insert(r.run_id.as_str());
                }
            },
        }
    }
    let reverted = |t: &TaskRecord| {
        run_reverts.contains(t.run_id.as_str())
            || task_reverts.contains(&(t.run_id.as_str(), t.task_id.as_str()))
    };
    let rows = CLASSES
        .iter()
        .map(|&c| {
            let of_class: Vec<&TaskRecord> = tasks
                .iter()
                .copied()
                .filter(|t| t.sessions > 0 && class(t) == Some(c))
                .collect();
            row(c, &of_class, &reverted)
        })
        .collect();
    let checked: Vec<_> = tasks
        .iter()
        .filter_map(|t| t.size_check.as_ref())
        .filter(|c| c.source == DeciderSource::Decider)
        .collect();
    let usage = || runs.iter().filter_map(|r| r.usage.as_ref());
    HistoryStats {
        path: path.to_path_buf(),
        task_records: tasks.len() as u32,
        run_records: runs.len() as u32,
        rows,
        decider_calls: usage().map(|u| u.decider_calls).sum(),
        decider_fallbacks: usage().map(|u| u.decider_fallbacks).sum(),
        size_checked: checked.len() as u32,
        size_raised: checked
            .iter()
            .filter(|c| c.decided.is_some_and(|d| d > c.engine))
            .count() as u32,
        problems: Vec::new(),
    }
}

/// `<n> <word>`, with an `s` unless `n` is 1.
fn count(n: u32, word: &str) -> String {
    if n == 1 {
        format!("1 {word}")
    } else {
        format!("{n} {word}s")
    }
}

/// Tokens as `<n>`, `<n>.<d>k` under 10k, `<n>k`, or `<n>.<d>M`, rounded down.
pub(crate) fn tokens(n: u64) -> String {
    match n {
        0..1_000 => n.to_string(),
        1_000..10_000 => format!("{}.{}k", n / 1_000, n % 1_000 / 100),
        10_000..1_000_000 => format!("{}k", n / 1_000),
        _ => format!("{}.{}M", n / 1_000_000, n % 1_000_000 / 100_000),
    }
}

fn or_dash<T>(value: Option<T>, show: impl Fn(T) -> String) -> String {
    value.map_or_else(|| "-".to_string(), show)
}

/// The brief's text layout (Interfaces, CLI): a header, one row per class in columns
/// as wide as their header or widest cell, two spaces apart, the totals line, and one
/// line for the lines skipped. Work minutes are rounded to the nearest minute.
pub fn render(stats: &HistoryStats) -> String {
    const HEADER: [&str; 9] = [
        "CLASS",
        "TASKS",
        "MERGED",
        "LINES",
        "TOOL CALLS",
        "TOKENS",
        "WORK MIN",
        "BOUNCES",
        "REVERTED",
    ];
    let mut table: Vec<Vec<String>> = vec![HEADER.map(str::to_string).to_vec()];
    for r in &stats.rows {
        table.push(vec![
            r.class.clone(),
            r.tasks.to_string(),
            r.merged.to_string(),
            or_dash(r.median_lines, |n| n.to_string()),
            or_dash(r.median_tool_calls, |n| n.to_string()),
            or_dash(r.median_tokens, tokens),
            or_dash(r.median_work_secs, |s| {
                (s.saturating_add(30) / 60).to_string()
            }),
            r.bounces.to_string(),
            r.reverted.to_string(),
        ]);
    }
    let widths: Vec<usize> = (0..HEADER.len())
        .map(|i| table.iter().map(|cells| cells[i].len()).max().unwrap_or(0))
        .collect();
    let mut out = format!(
        "history: {}  ({}, {})\n",
        stats.path.display(),
        count(stats.task_records, "task record"),
        count(stats.run_records, "run")
    );
    for cells in &table {
        let line: Vec<String> = cells
            .iter()
            .zip(&widths)
            .map(|(cell, &w)| format!("{cell:<w$}"))
            .collect();
        out.push_str(line.join("  ").trim_end());
        out.push('\n');
    }
    out.push_str(&format!(
        "deciders: {}, {} · size cross-check: {} checked, {} raised\n",
        count(stats.decider_calls, "call"),
        count(stats.decider_fallbacks, "fallback"),
        stats.size_checked,
        stats.size_raised
    ));
    // A file that could not be read is said as it is (M8b.17 review, m6).
    match stats.problems.first() {
        Some(first) if first.starts_with(super::history_io::UNREADABLE) => {
            out.push_str(&format!("history: {first}\n"));
        }
        Some(first) => out.push_str(&format!(
            "history: {} skipped: {first}\n",
            count(stats.problems.len() as u32, "line")
        )),
        None => {}
    }
    out
}

#[cfg(test)]
#[path = "stats_tests.rs"]
mod tests;
