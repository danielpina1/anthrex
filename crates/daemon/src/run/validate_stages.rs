//! Milestone 9.1's plan rules for stages and atomic tasks (decisions 43, 44 and 54)
//! and its reserved ids (decisions 39 and 45), for every plan source: the per-task
//! fields for `validate::check_fields`, and the rules over the whole list for
//! `validate_graph::validate_tasks_with`. Pure — no `std::fs`, `std::process`,
//! `std::thread`, `tokio` or `std::time::SystemTime` (design decision 2).

use std::collections::{BTreeMap, BTreeSet};

use proto::{PlanTask, STAGES_MAX, TaskState};

use super::model::Task;
use super::plan::PlanError;
use super::validate::is_valid_id;

/// Decision 39's refusal of a new `fix<n>` task.
pub(super) const FIX_ID_MESSAGE: &str = "fix<n> ids are reserved for fix tasks the engine adds";

/// `<prefix><digits>`, at least one digit.
fn numbered(id: &str, prefix: &str) -> bool {
    id.strip_prefix(prefix)
        .is_some_and(|n| !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()))
}

/// Decision 39: a **new** task of any source may not take a `fix<n>` id. An existing
/// one (the engine's own fix task, or one an older `run.json` holds) stays amendable,
/// so this is checked where tasks are added (`build_run`, `add_task`, `split_task`),
/// not in `check_fields`, which an amend re-runs.
pub(crate) fn reserved_new_id(id: &str) -> Option<PlanError> {
    numbered(id, "fix").then(|| PlanError::new(Some(id), "id", "id", FIX_ID_MESSAGE))
}

/// One task's own fields: a `stage-<n>` id names a stage branch (decision 45), the
/// stage is 1 to `STAGES_MAX` (decision 44), and an atomic task gives its reason
/// (decision 54).
pub(super) fn check_stage_fields(spec: &PlanTask, errors: &mut Vec<PlanError>) {
    let id = spec.id.as_str();
    let e =
        |field: &str, rule: &str, message: String| PlanError::new(Some(id), field, rule, message);
    if is_valid_id(id) && numbered(id, "stage-") {
        errors.push(e(
            "id",
            "id",
            format!("{id} is reserved for stage branches"),
        ));
    }
    if !(1..=STAGES_MAX).contains(&spec.stage) {
        errors.push(e(
            "stage",
            "4.1",
            format!("must be between 1 and {STAGES_MAX}"),
        ));
    }
    let no_reason = spec
        .atomic_reason
        .as_deref()
        .is_none_or(|r| r.trim().is_empty());
    if spec.atomic && no_reason {
        errors.push(e(
            "atomic_reason",
            "4.3",
            "required when atomic is true".to_string(),
        ));
    }
}

/// Milestone 9.1 decision 44, over the whole list after every batch, since a move of
/// one task can break the rule for another. Every unfinished task depends only on its
/// own or an earlier stage; the stages in use run from 1 without a gap, counting tasks
/// in every state (a stage whose tasks were all cancelled still exists); and a stage
/// has at most one atomic task that is not cancelled. A stage outside 1 to
/// `STAGES_MAX` is `check_fields`' error alone, so it opens no gap here.
pub(super) fn stage_rules(tasks: &[Task], by_id: &BTreeMap<&str, &Task>) -> Vec<PlanError> {
    let mut errors = Vec::new();
    for task in tasks.iter().filter(|t| is_active(t)) {
        let stage = task.spec.stage;
        for dep in &task.spec.deps {
            if let Some(d) = by_id.get(dep.as_str())
                && d.spec.stage > stage
            {
                errors.push(PlanError::new(
                    Some(task.id()),
                    "stage",
                    "4.1",
                    format!(
                        "stage {stage} cannot depend on {dep} in stage {}",
                        d.spec.stage
                    ),
                ));
            }
        }
    }
    let used: BTreeSet<u16> = tasks
        .iter()
        .map(|t| t.spec.stage)
        .filter(|s| (1..=STAGES_MAX).contains(s))
        .collect();
    let highest = used.last().copied().unwrap_or(0);
    for k in (1..highest).filter(|k| !used.contains(k)) {
        errors.push(PlanError::new(
            None,
            "stage",
            "4.1",
            format!("stages must be numbered from 1 without gaps: stage {k} has no task"),
        ));
    }
    let mut atomic: BTreeMap<u16, &str> = BTreeMap::new();
    for task in tasks
        .iter()
        .filter(|t| t.spec.atomic && t.state != TaskState::Cancelled)
    {
        let stage = task.spec.stage;
        match atomic.get(&stage) {
            Some(other) => errors.push(PlanError::new(
                Some(task.id()),
                "atomic",
                "4.3",
                format!("stage {stage} already has the atomic task {other}"),
            )),
            None => {
                atomic.insert(stage, task.id());
            }
        }
    }
    errors
}

fn is_active(task: &Task) -> bool {
    !task.state.is_finished()
}
