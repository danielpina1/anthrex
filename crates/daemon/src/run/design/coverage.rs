//! The plan's design checks (brief decisions 18 and 19, DF §5.1), in their order:
//! coverage, dangling `covers`, then brief shape. The first failing check is the
//! refusal; within a check every task it fails is named, joined by `; `. Only a design
//! run is checked. A cancelled (removed) task covers nothing and is not checked. Pure.
//!
//! The requirements are an argument: they are the approved spec's (`DesignState.
//! requirements`, Review focus 2), which the engine passes in.

use proto::{DesignMode, PlanTask, TaskState, safe_text};

use super::requirements::Requirement;
use super::template::lines;
use crate::run::model::Run;

/// Decision 19: the headings every brief of a design run has, each on its own line.
pub const BRIEF_HEADINGS: [&str; 5] =
    ["Files:", "Tests first:", "Steps:", "Acceptance:", "Verify:"];

/// `None` when the plan passes, else the exact refusal.
pub fn check(run: &Run, requirements: &[Requirement]) -> Option<String> {
    check_round(run, (requirements, requirements), None)
}

/// Decision 29 (task M9.6.15): a round's plan. Every requirement of `owed` (a round's:
/// the ones its amendment added or changed) is covered by a live task of `round` (every
/// live task when `None`); those tasks cover only requirements of `all`, and their
/// briefs have the headings. Earlier rounds' tasks were checked in their own rounds.
pub fn check_round(
    run: &Run,
    (owed, all): (&[Requirement], &[Requirement]),
    round: Option<u32>,
) -> Option<String> {
    if run.design_mode != DesignMode::Full {
        return None;
    }
    let tasks = live(run, round);
    uncovered(&tasks, owed)
        .or_else(|| dangling(&tasks, all))
        .or_else(|| briefs(&tasks))
}

/// Requirement → the live tasks that cover it, in requirement order then run order:
/// `plan.md`'s table and the gate's.
pub fn table(run: &Run, requirements: &[Requirement]) -> Vec<(String, Vec<String>)> {
    table_in(run, requirements, None)
}

/// [`table`] over round `round`'s tasks (every task's when `None`).
pub fn table_in(
    run: &Run,
    requirements: &[Requirement],
    round: Option<u32>,
) -> Vec<(String, Vec<String>)> {
    let tasks = live(run, round);
    requirements
        .iter()
        .map(|r| {
            let by = tasks
                .iter()
                .filter(|t| t.covers.contains(&r.id))
                .map(|t| t.id.clone())
                .collect();
            (r.id.clone(), by)
        })
        .collect()
}

/// The first of [`BRIEF_HEADINGS`] that `brief` lacks as a line of its own (review m8):
/// read on the cleaned text (`safe_text::multi_line`, as every agent-written text is
/// read), outside fenced code blocks.
pub fn missing_heading(brief: &str) -> Option<&'static str> {
    let cleaned = safe_text::multi_line(brief);
    let lines = lines(&cleaned);
    BRIEF_HEADINGS
        .into_iter()
        .find(|h| !lines.iter().any(|l| !l.code && l.text.trim() == *h))
}

fn live(run: &Run, round: Option<u32>) -> Vec<&PlanTask> {
    run.tasks
        .iter()
        .filter(|t| t.state != TaskState::Cancelled)
        .filter(|t| round.is_none_or(|k| t.round == k))
        .map(|t| &t.spec)
        .collect()
}

fn uncovered(tasks: &[&PlanTask], requirements: &[Requirement]) -> Option<String> {
    let ids: Vec<&str> = requirements
        .iter()
        .filter(|r| !tasks.iter().any(|t| t.covers.contains(&r.id)))
        .map(|r| r.id.as_str())
        .collect();
    (!ids.is_empty()).then(|| format!("{} are covered by no task", ids.join(", ")))
}

fn dangling(tasks: &[&PlanTask], requirements: &[Requirement]) -> Option<String> {
    let found: Vec<String> = tasks
        .iter()
        .flat_map(|t| t.covers.iter().map(move |c| (t, c)))
        .filter(|(_, c)| !requirements.iter().any(|r| &r.id == *c))
        .map(|(t, c)| format!("task {} covers {c}, which the spec does not have", t.id))
        .collect();
    joined(found)
}

fn briefs(tasks: &[&PlanTask]) -> Option<String> {
    let found: Vec<String> = tasks
        .iter()
        .filter_map(|t| {
            missing_heading(&t.brief)
                .map(|h| format!("task {}'s brief is missing the heading \"{h}\"", t.id))
        })
        .collect();
    joined(found)
}

fn joined(found: Vec<String>) -> Option<String> {
    (!found.is_empty()).then(|| found.join("; "))
}

#[cfg(test)]
#[path = "coverage_tests.rs"]
mod tests;
