//! The plan's design checks (brief decisions 18 and 19, DF §5.1), in their order:
//! coverage, dangling `covers`, then brief shape. The first failing check is the
//! refusal; within a check every task it fails is named, joined by `; `. Only a design
//! run is checked. A cancelled (removed) task covers nothing and is not checked. Pure.
//!
//! The requirements are an argument: they are the approved spec's (`DesignState.
//! requirements`, Review focus 2), which the engine passes in.

use proto::{DesignMode, PlanTask, TaskState};

use super::requirements::Requirement;
use crate::run::model::Run;

/// Decision 19: the headings every brief of a design run has, each on its own line.
pub const BRIEF_HEADINGS: [&str; 5] =
    ["Files:", "Tests first:", "Steps:", "Acceptance:", "Verify:"];

/// `None` when the plan passes, else the exact refusal.
pub fn check(run: &Run, requirements: &[Requirement]) -> Option<String> {
    if run.design_mode != DesignMode::Full {
        return None;
    }
    let tasks = live(run);
    uncovered(&tasks, requirements)
        .or_else(|| dangling(&tasks, requirements))
        .or_else(|| briefs(&tasks))
}

/// Requirement → the live tasks that cover it, in requirement order then run order:
/// `plan.md`'s table and the gate's.
pub fn table(run: &Run, requirements: &[Requirement]) -> Vec<(String, Vec<String>)> {
    let tasks = live(run);
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

/// The first of [`BRIEF_HEADINGS`] that `brief` lacks as a line of its own.
pub fn missing_heading(brief: &str) -> Option<&'static str> {
    BRIEF_HEADINGS
        .into_iter()
        .find(|h| !brief.lines().any(|l| l.trim() == *h))
}

fn live(run: &Run) -> Vec<&PlanTask> {
    run.tasks
        .iter()
        .filter(|t| t.state != TaskState::Cancelled)
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
