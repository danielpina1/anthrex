//! M8b decision 35: `anthrex run stats`, the recorded aggregates of `history.jsonl`:
//! one row per class (`S`, `M` non-hub, `hub`), and the deciders' and the size
//! cross-check's totals. Recording only, and proposing flaky tests for quarantine
//! (milestone 9.1 decision 34): nothing is applied or refitted (M9.5).
//! Pure (M8b decision 1); `history_io::summarise` reads the file and finds reverts.

use std::collections::{BTreeMap, HashSet};
use std::path::Path;

use proto::{
    DeciderSource, FlakyProposal, HistoryLine, HistoryStats, Size, StatsRow, TaskOutcome,
    TaskRecord,
};

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
            // Decision 43: role-routing records never count toward the aggregates;
            // neither do milestone 9.1's tier, flaky and bisect lines (decision 57);
            // `flaky_proposals` reads the flaky ones.
            HistoryLine::RoleRoute(_)
            | HistoryLine::Tier(_)
            | HistoryLine::Flaky(_)
            | HistoryLine::Bisect(_) => {}
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
                // M9.9 review fixes, M5: a research or review task reports; it is
                // neither merged nor unmerged work.
                .filter(|t| t.outcome != TaskOutcome::Reported)
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
        flaky_proposals: Vec::new(),
        window_days: 0,
        quarantine_after: 0,
    }
}

/// Seconds in a day, for `flaky_window_days`.
const DAY_SECS: u64 = 86_400;
/// Ruling C-23 (as C-13 item 3 for the cache): a `flaky` line dated more than this
/// ahead of now is ignored.
const FUTURE_SECS: u64 = 300;

/// Milestone 9.1 decision 34: every test recorded flaky in at least `after` distinct
/// runs within the last `window_days` before `now` (a line exactly `window_days` old is
/// inside), with its run count and its last time; the most runs first, then by name.
/// A line more than [`FUTURE_SECS`] in the future is ignored (ruling C-23). Nothing is
/// applied: `run stats` only prints them.
pub fn flaky_proposals(
    lines: &[HistoryLine],
    now: u64,
    window_days: u32,
    after: u32,
) -> Vec<FlakyProposal> {
    let window = u64::from(window_days).saturating_mul(DAY_SECS);
    let mut tests: BTreeMap<&str, (HashSet<&str>, u64)> = BTreeMap::new();
    for line in lines {
        let HistoryLine::Flaky(f) = line else {
            continue;
        };
        if f.at > now.saturating_add(FUTURE_SECS) || now.saturating_sub(f.at) > window {
            continue;
        }
        let (runs, last) = tests.entry(f.test.as_str()).or_default();
        runs.insert(f.run_id.as_str());
        *last = (*last).max(f.at);
    }
    let mut proposals: Vec<FlakyProposal> = tests
        .into_iter()
        .map(|(test, (runs, last_at))| FlakyProposal {
            test: test.to_string(),
            runs: u32::try_from(runs.len()).unwrap_or(u32::MAX),
            last_at,
        })
        .filter(|p| p.runs >= after)
        .collect();
    // Stable: equal counts stay in the map's name order.
    proposals.sort_by_key(|p| std::cmp::Reverse(p.runs));
    proposals
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
    flaky_block(stats, &mut out);
    out
}

/// Decision 34's proposals (Interfaces, CLI), after M8b's lines; nothing without one.
fn flaky_block(stats: &HistoryStats, out: &mut String) {
    if stats.flaky_proposals.is_empty() {
        return;
    }
    let days = stats.window_days;
    out.push_str(&format!(
        "Flaky tests (at least {} runs in the last {days} days):\n",
        stats.quarantine_after
    ));
    for p in &stats.flaky_proposals {
        let (test, runs) = (&p.test, p.runs);
        // Ruling C-24: a name with a line break stays on its one line here.
        let shown = crate::run::messages::one_line(test);
        out.push_str(&format!(
            "proposal: add {shown} to slow_tests (flaky in {runs} runs in the last {days} days)\n"
        ));
        let goal = format!(
            "Make the test {test} deterministic; it failed and then passed on retry in {runs} runs"
        );
        let goal = if test.chars().all(plain) {
            format!("\"{goal}\"")
        } else {
            shell_quote(&goal)
        };
        out.push_str(&format!("  fix: anthrex run start --goal {goal}\n"));
    }
}

/// Ruling C-23: a test-name character the brief's `"…"` quoting leaves harmless.
fn plain(c: char) -> bool {
    c.is_ascii_alphanumeric() || "_:./-[]".contains(c)
}

/// `text` single-quoted for a POSIX shell, each `'` written `'\''` (ruling C-23).
fn shell_quote(text: &str) -> String {
    format!("'{}'", text.replace('\'', "'\\''"))
}

#[cfg(test)]
#[path = "stats_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "stats_tests_flaky.rs"]
mod flaky_tests;
