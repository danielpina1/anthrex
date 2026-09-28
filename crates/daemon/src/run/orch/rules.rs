//! Decision 23: the plan rules for the orchestrator's `edit_plan` and a sub-planner's
//! `submit_epic`, run after M8a's validation on the edited run
//! (`run::edits::apply_edits`). Plan files and the user's `run edit` keep M8a's rules
//! only: a user's own plan is theirs. Pure — no `std::fs`, `std::process`,
//! `std::thread`, `tokio` or `std::time::SystemTime` (M8a design decision 2).

use std::collections::{BTreeMap, BTreeSet};

use proto::{TaskKind, TaskState};

use super::EditSource;
use crate::run::model::{Run, Task};
use crate::run::plan::PlanError;
use crate::scout::report::ONBOARDING_ALIAS;

/// Decision 23.2's note on a code or docs task of a run with no scout report at all.
pub const UNBACKED_NOTE: &str = "size not backed by a scout report";

/// The errors decision 23 finds in `run` after a batch from `source` that added,
/// split in or amended the `touched` tasks. Empty for [`EditSource::User`]. Per touched
/// task in plan order: the reserved id (23.6), the epic (23.5), the budget (23.1) and
/// the scout evidence (23.2); then the caps (23.3) of every group a touched task is in.
pub fn check(run: &Run, touched: &BTreeSet<String>, source: &EditSource) -> Vec<PlanError> {
    if *source == EditSource::User {
        return Vec::new();
    }
    let mut errors = Vec::new();
    let mut groups = BTreeSet::new();
    for task in checked(run, touched) {
        let id = task.id();
        let e = |field: &str, rule: &str, message: String| {
            PlanError::new(Some(id), field, rule, message)
        };
        if task.orch.integration_of.is_none() && is_integration_id(id) {
            errors.push(e(
                "id",
                "5.3.reserved",
                "ids ending in -int<n> are reserved for integration reviews".into(),
            ));
        }
        if let Some(epic) = &task.spec.epic {
            match run.orch.epics.iter().find(|r| r.epic == *epic) {
                None => errors.push(e(
                    "epic",
                    "2.epic",
                    format!("{epic} is not an epic of this run; create it with spawn_subplanner"),
                )),
                Some(record) if *source == EditSource::Orchestrator && record.phase.is_live() => {
                    errors.push(e(
                        "epic",
                        "2.epic",
                        format!("epic {epic} is being planned by its sub-planner"),
                    ))
                }
                Some(_) => {}
            }
        }
        if task.spec.budget.is_some() {
            errors.push(e(
                "budget",
                "7.1.budget",
                "budgets come from the task's size; leave budget out (rule 7.1)".into(),
            ));
        }
        if changes_files(task) && has_reports(run) && task.spec.scout_refs.is_empty() {
            errors.push(e(
                "scout_refs",
                "7.1.evidence",
                "name the scout reports this task's size rests on (rule 7.1)".into(),
            ));
        }
        for reference in task.spec.scout_refs.iter().filter(|r| !is_report(run, r)) {
            errors.push(e(
                "scout_refs",
                "7.1.evidence",
                format!("{reference} is not a finished scout report of this run"),
            ));
        }
        groups.insert(task.spec.epic.clone());
    }
    errors.extend(caps(run, &groups));
    errors
}

/// Decision 23 on a batch's edited run: [`note_unbacked`], then [`check`]'s errors.
pub fn apply(run: &mut Run, touched: &BTreeSet<String>, source: &EditSource) -> Vec<PlanError> {
    note_unbacked(run, touched, source);
    check(run, touched, source)
}

/// Decision 23.2's note: with no finished scout report in the run, each touched code
/// or docs task from the orchestrator or a sub-planner says so, once.
pub fn note_unbacked(run: &mut Run, touched: &BTreeSet<String>, source: &EditSource) {
    if *source == EditSource::User || has_reports(run) {
        return;
    }
    for task in run.tasks.iter_mut() {
        let noted = task.notes.iter().any(|n| n == UNBACKED_NOTE);
        if touched.contains(task.id())
            && task.state != TaskState::Cancelled
            && changes_files(task)
            && !noted
        {
            task.notes.push(UNBACKED_NOTE.to_string());
        }
    }
}

/// The touched tasks decision 23 looks at: not cancelled (a split's parent is).
fn checked<'a>(run: &'a Run, touched: &'a BTreeSet<String>) -> impl Iterator<Item = &'a Task> {
    run.tasks
        .iter()
        .filter(|t| touched.contains(t.id()) && t.state != TaskState::Cancelled)
}

/// Decision 23.3: tasks with no epic count toward the orchestrator, tasks of epic `e`
/// toward `e`'s sub-planner; unfinished and finished alike, but not cancelled ones and
/// not decision 37's integration reviews. Only the `groups` a batch touched are checked.
fn caps(run: &Run, groups: &BTreeSet<Option<String>>) -> Vec<PlanError> {
    let cap = run.limits.orch.planner_task_cap;
    let mut counts: BTreeMap<&Option<String>, u32> = BTreeMap::new();
    for task in run
        .tasks
        .iter()
        .filter(|t| t.state != TaskState::Cancelled && t.orch.integration_of.is_none())
    {
        *counts.entry(&task.spec.epic).or_default() += 1;
    }
    let mut errors = Vec::new();
    for group in groups {
        let n = counts.get(group).copied().unwrap_or(0);
        if n <= cap {
            continue;
        }
        let message = match group {
            None => format!(
                "the orchestrator's plan has {n} tasks, more than planner_task_cap ({cap}); plan the rest through sub-planners (rule 5.1)"
            ),
            Some(e) => format!(
                "epic {e} has {n} tasks, more than planner_task_cap ({cap}); split the epic (rule 5.1)"
            ),
        };
        errors.push(PlanError::new(None, "tasks", "5.1.cap", message));
    }
    errors
}

/// Code and docs tasks change files; research and review tasks do not (decision 24).
fn changes_files(task: &Task) -> bool {
    matches!(task.spec.kind, TaskKind::Code | TaskKind::Docs)
}

/// Whether the run has any finished scout report: a run scout's, or the onboarding
/// report behind the alias `onboarding` (M8b decision 19).
fn has_reports(run: &Run) -> bool {
    !run.scout_reports.is_empty() || run.onboarding_report.is_some()
}

fn is_report(run: &Run, reference: &str) -> bool {
    run.scout_reports.iter().any(|s| s == reference)
        || (reference == ONBOARDING_ALIAS && run.onboarding_report.is_some())
}

/// Decision 23.6: `-int` then one or more digits, at the end.
fn is_integration_id(id: &str) -> bool {
    id.rsplit_once("-int")
        .is_some_and(|(_, n)| !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()))
}

#[cfg(test)]
#[path = "rules_tests.rs"]
mod tests;
