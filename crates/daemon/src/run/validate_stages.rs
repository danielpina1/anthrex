//! Milestone 9.1's plan rules for stages and atomic tasks (decisions 43, 44 and 54)
//! and its reserved ids (decisions 39 and 45), for every plan source: the per-task
//! fields for `validate::check_fields`, and the rules over the whole list for
//! `validate_graph::validate_tasks_with`. Pure — no `std::fs`, `std::process`,
//! `std::thread`, `tokio` or `std::time::SystemTime` (design decision 2).

use std::collections::{BTreeMap, BTreeSet};

use proto::{PlanTask, RunState, STAGES_MAX, TaskState};

use super::model::{Run, StageLayout, Task};
use super::plan::PlanError;
use super::validate::is_valid_id;

/// Decision 39's refusal of a new `fix<n>` task.
pub(super) const FIX_ID_MESSAGE: &str = "fix<n> ids are reserved for fix tasks the engine adds";

/// `<prefix><digits>`, at least one digit.
fn numbered(id: &str, prefix: &str) -> bool {
    id.strip_prefix(prefix)
        .is_some_and(|n| !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()))
}

/// Decisions 39 and 45: a **new** task of any source may not take a `fix<n>` or a
/// `stage-<n>` id. An existing one (the engine's own fix task, or one an older
/// `run.json` holds) stays amendable (controller ruling C-14 (e)), so this is checked
/// where tasks are added (`build_run`, `add_task`, `split_task`), not in
/// `check_fields`, which an amend re-runs.
pub(crate) fn reserved_new_id(id: &str) -> Option<PlanError> {
    let e = |message: String| PlanError::new(Some(id), "id", "id", message);
    if numbered(id, "fix") {
        Some(e(FIX_ID_MESSAGE.to_string()))
    } else if is_valid_id(id) && numbered(id, "stage-") {
        Some(e(format!("{id} is reserved for stage branches")))
    } else {
        None
    }
}

/// Controller ruling C-14 (a): a split cannot change stages, so a child takes its
/// parent's stage. `stage` defaults to 1 in serde, so a child naming 1 cannot be told
/// from one naming none, and takes the parent's too; any other stage is refused.
pub(super) fn split_child_stage(parent: u16, child: &mut PlanTask) -> Option<PlanError> {
    let named = child.stage;
    child.stage = parent;
    (named != parent && named != 1).then(|| {
        let message = "children stay in their parent's stage";
        PlanError::new(Some(&child.id), "split_task", "4.1", message)
    })
}

/// One task's own fields: the stage is 1 to `STAGES_MAX` (decision 44), and an atomic
/// task gives its reason (decision 54).
pub(super) fn check_stage_fields(spec: &PlanTask, errors: &mut Vec<PlanError>) {
    let id = spec.id.as_str();
    let e =
        |field: &str, rule: &str, message: String| PlanError::new(Some(id), field, rule, message);
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

/// Decision 46: once a `Single` run is approved, its tasks stay in stage 1.
pub(super) fn single_layout_rule(run: &Run) -> Vec<PlanError> {
    let approved = !matches!(run.state, RunState::Planning | RunState::AwaitingApproval);
    if run.stage_layout != StageLayout::Single || !approved {
        return Vec::new();
    }
    run.tasks
        .iter()
        .filter(|t| is_active(t) && t.spec.stage > 1)
        .map(|t| {
            let message = "this run was approved with one stage; its tasks stay in stage 1";
            PlanError::new(Some(t.id()), "stage", "4.1", message.to_string())
        })
        .collect()
}

fn is_active(task: &Task) -> bool {
    !task.state.is_finished()
}
