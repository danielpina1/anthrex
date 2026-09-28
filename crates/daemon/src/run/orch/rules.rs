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

/// The errors decision 23 finds in `run`, the result of a batch from `source` applied
/// to `before`. Empty for [`EditSource::User`]. The rules apply to what the batch
/// changed (the M9.4 review fixes): the reserved id (23.6) and the epic (23.5) to each
/// new task, the budget (23.1) and the scout evidence (23.2) to each new task and each
/// task whose size the batch changed, in plan order; then the cap (23.3) of every group
/// whose count the batch raised above it.
pub fn check(run: &Run, before: &Run, source: &EditSource) -> Vec<PlanError> {
    if *source == EditSource::User {
        return Vec::new();
    }
    let mut errors = Vec::new();
    for (task, new) in changed(run, before) {
        let id = task.id();
        let e = |field: &str, rule: &str, message: String| {
            PlanError::new(Some(id), field, rule, message)
        };
        if new && task.orch.integration_of.is_none() && is_integration_id(id) {
            errors.push(e(
                "id",
                "5.3.reserved",
                "ids ending in -int<n> are reserved for integration reviews".into(),
            ));
        }
        if let (true, Some(epic)) = (new, &task.spec.epic) {
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
    }
    errors.extend(caps(run, before, source));
    errors
}

/// Decision 23 on a batch's edited run: [`note_unbacked`], then [`check`]'s errors.
pub fn apply(run: &mut Run, before: &Run, source: &EditSource) -> Vec<PlanError> {
    note_unbacked(run, before, source);
    check(run, before, source)
}

/// Decision 23.2's note: with no finished scout report in the run, each code or docs
/// task the batch from the orchestrator or a sub-planner added or resized says so, once.
pub fn note_unbacked(run: &mut Run, before: &Run, source: &EditSource) {
    if *source == EditSource::User || has_reports(run) {
        return;
    }
    let ids: BTreeSet<String> = changed(run, before)
        .filter(|(task, _)| changes_files(task))
        .map(|(task, _)| task.id().to_string())
        .collect();
    for task in run.tasks.iter_mut().filter(|t| ids.contains(t.id())) {
        if !task.notes.iter().any(|n| n == UNBACKED_NOTE) {
            task.notes.push(UNBACKED_NOTE.to_string());
        }
    }
}

/// The tasks of `run` the batch from `before` changed, in plan order, not cancelled (a
/// split's parent is): each new one (`true`: added or split in), and each whose `size`
/// an `amend_task` changed (`false`). An amend of any other field is not a change here.
fn changed<'a>(run: &'a Run, before: &'a Run) -> impl Iterator<Item = (&'a Task, bool)> {
    run.tasks
        .iter()
        .filter(|t| t.state != TaskState::Cancelled)
        .filter_map(|t| match before.tasks.iter().find(|b| b.id() == t.id()) {
            None => Some((t, true)),
            Some(old) if old.spec.size != t.spec.size => Some((t, false)),
            Some(_) => None,
        })
}

/// Decision 23.3: tasks with no epic count toward the orchestrator, tasks of epic `e`
/// toward `e`'s sub-planner; unfinished and finished alike, but not cancelled ones and
/// not decision 37's integration reviews. A group is refused only when the batch raised
/// its count above the cap: one already over it (a user's edits are unchecked) never
/// refuses a batch that leaves it the same or smaller. Decision 23.5: the orchestrator's
/// fix tasks for an epic whose integration review has reported do not meet its cap, however
/// many (deliberate: M9.4 second review, ruling 2).
fn caps(run: &Run, before: &Run, source: &EditSource) -> Vec<PlanError> {
    let cap = run.limits.orch.planner_task_cap;
    let (after, prior) = (counts(run), counts(before));
    let mut errors = Vec::new();
    for (group, &n) in &after {
        if n <= cap || n <= prior.get(group).copied().unwrap_or(0) {
            continue;
        }
        if let Some(e) = group
            && *source == EditSource::Orchestrator
            && integration_reviewed(before, e)
        {
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

/// Decision 23.3's count per group (`None` is the orchestrator's).
fn counts(run: &Run) -> BTreeMap<Option<String>, u32> {
    let mut counts = BTreeMap::new();
    for task in run
        .tasks
        .iter()
        .filter(|t| t.state != TaskState::Cancelled && t.orch.integration_of.is_none())
    {
        *counts.entry(task.spec.epic.clone()).or_default() += 1;
    }
    counts
}

/// Whether an integration review of epic `e` has reported (decision 37), in `before`,
/// the run before the batch: a cancelled review reviewed nothing, and a batch cannot
/// cancel a review to exempt its own additions (M9.4 second review, ruling 1).
fn integration_reviewed(before: &Run, e: &str) -> bool {
    before
        .tasks
        .iter()
        .any(|t| t.orch.integration_of.as_deref() == Some(e) && t.state == TaskState::Reported)
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

#[cfg(test)]
#[path = "rules_tests_scope.rs"]
mod tests_scope;
