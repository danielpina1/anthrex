//! Milestone 9 decisions 37 and 38, engine side: the per-epic integration review and
//! completion with an orchestrator. Split from `kinds.rs`, which re-exports these.
//! Pure (design decision 2).

use proto::{IntegrationState, PlanEdit, PlanTask, RouteSpec, Size, TaskKind, TaskState};

use super::kinds::is_integration;
use super::requests::log;
use crate::run::model::{ReviewLevel, Run, TaskEvent, task_branch, task_path};
use crate::run::roster::pick_reviewer;
use crate::run::validate::{is_valid_id, resolve_task_lenient};

/// Decision 37: the orchestrator may not amend, split or cancel an integration review.
pub(super) fn engine_owned(run: &Run, edits: &[PlanEdit]) -> Option<String> {
    edits.iter().find_map(|edit| {
        let id = match edit {
            PlanEdit::AmendTask { task_id, .. }
            | PlanEdit::SplitTask { task_id, .. }
            | PlanEdit::CancelTask { task_id } => task_id,
            _ => return None,
        };
        run.task(id)
            .filter(|t| is_integration(t))
            .map(|_| format!("task {id} is an integration review; the engine owns it"))
    })
}

/// Decision 37: an epic task merged at `commit` from run head `from`; the epic's base is
/// the run head before its first merge. Integration reviews are not the epic's work.
pub(super) fn record_merge(run: &mut Run, i: usize, from: &str, commit: &str) {
    let task = &run.tasks[i];
    let (Some(epic), false) = (task.spec.epic.clone(), is_integration(task)) else {
        return;
    };
    let id = task.id().to_string();
    if let Some(record) = run.orch.epics.iter_mut().find(|e| e.epic == epic) {
        record.base.get_or_insert_with(|| from.to_string());
        record.merges.push((id, commit.to_string()));
    }
}

/// Decision 37, every scheduler pass of a running run: for each epic whose tasks are
/// all finished with one merged at least, whose sub-planner is not live and whose last
/// integration review has finished before its latest merge, the next round, while
/// fewer than `max_bounces + 1` rounds ran. The `finish` edit closes an epic's
/// `changes` instead (decision 38).
pub(super) fn integration_pass(run: &mut Run, now: u64) {
    // A cancelled run reviews nothing more (M9.9 review fixes, C1).
    if run.orch.orchestrator.is_none() || run.cancelled {
        return;
    }
    for k in 0..run.orch.epics.len() {
        if run.finish_edit {
            let record = &mut run.orch.epics[k];
            if record.integration_state == IntegrationState::Changes {
                record.integration_state = IntegrationState::Finished;
                let text = format!(
                    "epic {}'s integration review closed by the finish edit",
                    record.epic
                );
                log(run, now, text);
            }
            continue;
        }
        // M9.9 review fixes, M3: with no valid id, nothing is made
        // (`attention` says so).
        if due(run, k) {
            add_round(run, k, now);
        }
    }
}

fn due(run: &Run, k: usize) -> bool {
    let record = &run.orch.epics[k];
    let epic = Some(record.epic.as_str());
    let mut work = run
        .tasks
        .iter()
        .filter(|t| t.spec.epic.as_deref() == epic && !is_integration(t))
        .peekable();
    let finished = work.peek().is_some() && work.all(|t| t.state.is_finished());
    let reviewing = run
        .tasks
        .iter()
        .any(|t| t.orch.integration_of.as_deref() == epic && !t.state.is_finished());
    let rounds_left = record.integration_rounds < u32::from(run.limits.max_bounces) + 1;
    !record.phase.is_live()
        && finished
        && !reviewing
        && rounds_left
        && record.merges.len() > record.integration_reviewed as usize
        && stage_holds_epic(run, epic)
}

/// Milestone 9.1 decisions 47 and 50: the highest stage holding one of the epic's
/// merged tasks holds all of them (each propagated up to it), so its head, which the
/// review reads, contains the whole epic.
fn stage_holds_epic(run: &Run, epic: Option<&str>) -> bool {
    let merged: Vec<&crate::run::model::Task> = run
        .tasks
        .iter()
        .filter(|t| t.spec.epic.as_deref() == epic && !is_integration(t))
        .filter(|t| t.state == TaskState::Merged)
        .collect();
    let Some(top) = merged.iter().map(|t| t.stage()).max() else {
        return true;
    };
    match run.stage(top) {
        Some(stage) if run.stage_layout == crate::run::model::StageLayout::Multi => {
            merged.iter().all(|t| stage.tasks_in.contains(t.id()))
        }
        _ => true,
    }
}

/// Round `n + 1` of epic `k`'s integration review: a review task added directly, its
/// id the first free `<e>-int<n>` from the next round's number (M9.4 review fixes,
/// ruling 7: the user's `run edit` may hold one), on the peer of the epic's latest
/// merged task's route at `frontier`.
fn add_round(run: &mut Run, k: usize, now: u64) {
    let record = &run.orch.epics[k];
    let epic = record.epic.clone();
    let Some((id, n)) = free_id(run, k) else {
        return;
    };
    let base = record.base.clone().unwrap_or_else(|| run.base_sha.clone());
    let author = record
        .merges
        .last()
        .and_then(|(t, _)| run.task(t))
        .map(|t| t.route.clone());
    // Milestone 9.1 decision 47 (controller ruling C-14 (c)): the review goes into the
    // highest stage holding one of the epic's tasks, and reviews up to its head.
    let stage = run
        .tasks
        .iter()
        .filter(|t| t.spec.epic.as_deref() == Some(epic.as_str()) && !is_integration(t))
        // Controller ruling C-15 (M-5): a task that finished without merging (cancelled,
        // reported) put nothing in its stage.
        .filter(|t| t.state == TaskState::Merged || !t.state.is_finished())
        .map(|t| t.stage())
        .max()
        .unwrap_or(1);
    let head = run.stage_head(stage).unwrap_or(&run.run_head).to_string();
    let spec = PlanTask {
        id: id.clone(),
        title: format!("integration review of epic {epic}, round {n}"),
        epic: Some(epic.clone()),
        kind: TaskKind::Review,
        size: Size::M,
        interface_change: false,
        test_mode: None,
        test_mode_reason: None,
        owns: Vec::new(),
        deps: Vec::new(),
        priority: 0,
        brief: format!("Review epic {epic}'s merged work as a whole."),
        acceptance: Vec::new(),
        test_to_write: None,
        scout_refs: Vec::new(),
        route: RouteSpec::default(),
        budget: None,
        review_target: Some(format!("{base}..{head}")),
        stage,
        atomic: false,
        atomic_reason: None,
    };
    let (mut task, _) = resolve_task_lenient(
        spec,
        &run.profile,
        &run.limits,
        &run.roster,
        run.limits.default_runtime,
    );
    task.branch = task_branch(&run.id, &id);
    task.worktree = task_path(&run.wt_dir, &run.id, &id);
    let author = author.unwrap_or_else(|| task.route.clone());
    task.route = pick_reviewer(&run.roster, &author, ReviewLevel::Frontier);
    task.review_level = Some(ReviewLevel::Frontier);
    task.orch.integration_of = Some(epic.clone());
    task.history.push(TaskEvent {
        at: now,
        text: format!("added by the engine: the integration review of epic {epic}"),
    });
    run.tasks.push(task);
    let merges = run.orch.epics[k].merges.len() as u32;
    let record = &mut run.orch.epics[k];
    record.integration_rounds += 1;
    record.integration_reviewed = merges;
    record.integration_state = IntegrationState::Reviewing;
    log(
        run,
        now,
        format!("integration review {id} of epic {epic} added"),
    );
}

/// The first free `<e>-int<n>` from the next round's number, with its `n`, while it
/// is a valid task id (M9.9 review fixes, M3); `None` when it would not be.
fn free_id(run: &Run, k: usize) -> Option<(String, u32)> {
    let record = &run.orch.epics[k];
    let first = record.integration_rounds + 1;
    let n = (first..).find(|n| run.task(&format!("{}-int{n}", record.epic)).is_none())?;
    let id = format!("{}-int{n}", record.epic);
    is_valid_id(&id).then_some((id, n))
}

fn no_free_id_line(epic: &str) -> String {
    format!("epic {epic}: no free integration review id")
}

/// M9.9 review fixes, M3: an attention line for each epic whose integration review is
/// due but has no valid id to take.
pub(crate) fn attention(run: &Run) -> Vec<String> {
    if run.orch.orchestrator.is_none() || run.finish_edit || run.cancelled {
        return Vec::new();
    }
    (0..run.orch.epics.len())
        .filter(|&k| due(run, k) && free_id(run, k).is_none())
        .map(|k| no_free_id_line(&run.orch.epics[k].epic))
        .collect()
}

/// The round integration review `task` is: the `n` of its id `<e>-int<n>`.
pub(super) fn round_of(task: &crate::run::model::Task) -> Option<u32> {
    let epic = task.orch.integration_of.as_deref()?;
    task.id()
        .strip_prefix(epic)?
        .strip_prefix("-int")?
        .parse()
        .ok()
}
/// Decision 38: completion with an orchestrator also waits for every hold to be
/// decided, every sub-planner and run scout to end, every integration review to finish
/// with no `changes` left open (the `finish` edit closes them), and the plan to have
/// been submitted. A run with no orchestrator completes as M8a's does.
pub(super) fn may_complete(run: &Run) -> bool {
    use crate::run::orch::RunScoutState;
    use proto::HoldState;
    let Some(o) = &run.orch.orchestrator else {
        return true;
    };
    let orch = &run.orch;
    // M9.9 review fixes, C1: the user can always end a run. After `run cancel` or the
    // `finish` edit, the plan's submit, the holds and a `changes` verdict wait for no
    // one; the sessions and reviews still end first.
    let ended = run.cancelled || run.finish_edit;
    (ended
        || o.plan_submitted
            && !orch
                .gate_holds
                .iter()
                .any(|h| matches!(h.state, HoldState::Drafting | HoldState::Awaiting))
            && !orch
                .epics
                .iter()
                .any(|e| e.integration_state == IntegrationState::Changes))
        && !orch.epics.iter().any(|e| e.phase.is_live())
        && !orch
            .run_scouts
            .iter()
            .any(|s| matches!(s.state, RunScoutState::Queued | RunScoutState::Running))
        && !run
            .tasks
            .iter()
            .any(|t| is_integration(t) && !t.state.is_finished())
}
