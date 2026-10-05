//! `plan.md` (brief decision 21, DF §5.1): the plan as the user reads it at the plan
//! gate and as the repository receives it. A title; per stage, then per task, the task's
//! heading, its `Covers:` line, its size, route and test mode, and its brief; then the
//! requirement → tasks coverage table. A cancelled (removed) task is left out. Every
//! agent-written text is cleaned (Review focus 4). Pure.

use std::collections::BTreeMap;
use std::fmt::Write as _;

use proto::{TaskState, safe_text};

use super::coverage;
use super::requirements::Requirement;
use crate::run::contract::{mode_label, size_label};
use crate::run::model::{Run, Task};
use crate::run::orch::json::route_text;

/// The title's goal head, as the documents commit's message cuts it (decision 24).
pub const GOAL_HEAD_CHARS: usize = 60;

/// The goal on one line, cut to [`GOAL_HEAD_CHARS`] characters.
pub fn goal_head(goal: &str) -> String {
    safe_text::one_line(goal)
        .trim()
        .chars()
        .take(GOAL_HEAD_CHARS)
        .collect()
}

pub fn render(run: &Run, requirements: &[Requirement]) -> String {
    render_round(run, requirements, None)
}

/// Task M9.6.15: round `round`'s `plan.md` (decision 24's `…-round<k>.md`): its title
/// names the round, and its tasks and coverage table are the round's; `None` is
/// [`render`]'s, every task.
pub fn render_round(run: &Run, requirements: &[Requirement], round: Option<u32>) -> String {
    let mut out = format!("# Plan: {}", goal_head(&run.goal));
    if let Some(k) = round {
        let _ = write!(out, " (round {k})");
    }
    out.push_str("\n\n");
    let mut stages: BTreeMap<u16, Vec<&Task>> = BTreeMap::new();
    let of_round = |t: &&Task| round.is_none_or(|k| t.round == k);
    for task in run
        .tasks
        .iter()
        .filter(|t| t.state != TaskState::Cancelled)
        .filter(of_round)
    {
        stages.entry(task.spec.stage).or_default().push(task);
    }
    for (stage, tasks) in stages {
        let _ = writeln!(out, "## Stage {stage}\n");
        for task in tasks {
            write_task(&mut out, task);
        }
    }
    out.push_str("## Coverage\n\n| Requirement | Tasks |\n|---|---|\n");
    for (id, tasks) in coverage::table_in(run, requirements, round) {
        let by = if tasks.is_empty() {
            "none".to_string()
        } else {
            tasks.join(", ")
        };
        let _ = writeln!(out, "| {id} | {by} |");
    }
    out
}

fn write_task(out: &mut String, task: &Task) {
    let spec = &task.spec;
    let covers = if spec.covers.is_empty() {
        "none".to_string()
    } else {
        spec.covers.join(", ")
    };
    let _ = writeln!(
        out,
        "### {} {}",
        safe_text::one_line(&spec.id),
        safe_text::one_line(&spec.title)
    );
    let _ = writeln!(out, "Covers: {}", safe_text::one_line(&covers));
    let _ = writeln!(
        out,
        "Size: {} · Route: {} · Tests: {}\n",
        size_label(task.size),
        safe_text::one_line(&route_text(&task.route)),
        mode_label(task.test_mode)
    );
    let _ = writeln!(out, "{}\n", safe_text::multi_line(&spec.brief).trim_end());
}

#[cfg(test)]
#[path = "plan_md_tests.rs"]
mod tests;
