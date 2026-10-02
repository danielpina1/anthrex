//! Milestone 9.1 decisions 37, 39 and 51, engine side: the one function that adds an
//! engine-made fix task ([`add_fix`]), and fix-task ids (`fix<n>` from `Run.fix_seq`).
//! A fix task is an ordinary task: it is created `pending` in its stage, needs no
//! approval, and goes through M8a's states and gates like any other. 9.2 adds its `ci`,
//! `review` and base-`sync` tasks through [`add_fix`]. Pure (design decision 2).

use std::collections::BTreeSet;

use proto::{PlanTask, RouteSpec, Size, TaskKind, TaskOrigin, TestMode};

use super::Effect;
use super::requests::log;
use crate::run::model::{FixOf, Run, SyncState, TaskEvent, task_branch, task_path};
use crate::run::validate::{
    EditScope, combined_cycles, implicit_deps, protected_notes, resolve_task_lenient,
    validate_tasks_with,
};

/// Decision 37: a fix task's priority, above every planned task's usual one.
pub(crate) const FIX_PRIORITY: i32 = 100;

/// What [`add_fix`] makes (decisions 37 and 51).
#[derive(Debug, Clone)]
pub(crate) struct FixSpec {
    pub origin: TaskOrigin,
    pub fixes: FixOf,
    pub stage: u16,
    pub title: String,
    pub brief: String,
    pub acceptance: Vec<String>,
    pub owns: Vec<String>,
    pub size: Size,
    pub epic: Option<String>,
    pub route: RouteSpec,
    pub test_mode: TestMode,
    pub test_mode_reason: Option<String>,
    /// Decision 51's sync state, kept as `Task.sync`.
    pub sync: Option<SyncState>,
}

/// Decision 39: `fix<n>`, the first id free at or above `Run.fix_seq` (from 1), so
/// fix tasks count up across origins and never take an id a task already has.
pub(crate) fn next_fix_id(run: &Run) -> String {
    let n = fix_number(run);
    format!("fix{n}")
}

fn fix_number(run: &Run) -> u32 {
    (run.fix_seq.max(1)..)
        .find(|n| run.task(&format!("fix{n}")).is_none())
        .unwrap_or(u32::MAX)
}

/// Adds the fix task `spec` describes to the run: resolved exactly as a plan task is
/// (`resolve_task_lenient`), given its branch and worktree as M9's engine-made
/// integration review is, and checked against M8a's plan rules as if one plan edit had
/// added it. `Ok(id)` once added; `Err(<the first rule's message>)`, and nothing
/// changed, when a rule refuses it.
pub(crate) fn add_fix(
    run: &mut Run,
    spec: FixSpec,
    now: u64,
    _fx: &mut Vec<Effect>,
) -> Result<String, String> {
    let n = fix_number(run);
    let id = format!("fix{n}");
    let plan = PlanTask {
        id: id.clone(),
        title: spec.title,
        epic: spec.epic,
        kind: TaskKind::Code,
        size: spec.size,
        interface_change: false,
        test_mode: Some(spec.test_mode),
        test_mode_reason: spec.test_mode_reason,
        owns: spec.owns,
        deps: Vec::new(),
        priority: FIX_PRIORITY,
        brief: spec.brief,
        acceptance: spec.acceptance,
        test_to_write: None,
        scout_refs: Vec::new(),
        route: spec.route,
        budget: None,
        review_target: None,
        stage: spec.stage,
        atomic: false,
        atomic_reason: None,
        addresses: Vec::new(),
    };
    let (mut task, mut errors) = resolve_task_lenient(
        plan,
        &run.profile,
        &run.limits,
        &run.roster,
        run.limits.default_runtime,
    );
    task.branch = task_branch(&run.id, &id);
    task.worktree = task_path(&run.wt_dir, &run.id, &id);
    task.notes
        .extend(protected_notes(&task.spec.owns, &run.protected_files));
    task.origin = spec.origin;
    task.sync = spec.sync;
    let what = fix_text(run, &spec.fixes);
    task.fixes = Some(spec.fixes);
    task.history.push(TaskEvent {
        at: now,
        text: format!("added by the engine: a fix task for the {what}"),
    });
    let mut tasks = run.tasks.clone();
    tasks.push(task);
    let touched = BTreeSet::from([id.clone()]);
    errors.extend(validate_tasks_with(
        &tasks,
        &touched,
        None,
        &EditScope::Run,
        run.limits.max_tasks,
        run.limits.default_runtime,
    ));
    let implicit = implicit_deps(&tasks);
    for (task, deps) in tasks.iter_mut().zip(implicit) {
        task.implicit_deps = deps;
    }
    if errors.is_empty() {
        errors.extend(combined_cycles(&tasks));
    }
    if let Some(error) = errors.first() {
        return Err(error.message.clone());
    }
    run.tasks = tasks;
    run.fix_seq = n.saturating_add(1);
    log(run, now, format!("fix task {id} added for the {what}"));
    Ok(id)
}

/// Decision 39's display text of what a fix task fixes (`TaskInfo.fixes`). Milestone
/// 9.2's three (decision 41) are built in `run/delivery/snapshot.rs`; a review fix
/// names its threads' authors, which only the run's delivery records.
pub(crate) fn fix_text(run: &Run, fixes: &FixOf) -> String {
    match fixes {
        FixOf::Bisect { culprit, .. } => format!("bisect of {culprit}"),
        FixOf::Propagate { from, to, .. } => format!("propagate of stage {from} into stage {to}"),
        FixOf::Ci { .. } | FixOf::Review { .. } | FixOf::Base { .. } => {
            crate::run::delivery::snapshot::fix_text(run, fixes).unwrap_or_default()
        }
    }
}
