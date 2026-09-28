//! Decision 13, engine side: one plan-edit batch applied to a run, for every source
//! (the user's `run edit`, and milestone 9's orchestrator through `edit_plan`, decision
//! 19). Moved out of `requests.rs` by task M9.7, which shares it. Pure (design decision
//! 2).

use proto::{PlanEdit, RunState, Runtime, TaskState};

use super::requests::log;
use super::{Effect, deciders, outbox, restore};
use crate::run::edit_log;
use crate::run::edits::{EditConsequence, apply_edits};
use crate::run::model::Run;
use crate::run::orch::EditSource;
use crate::run::plan::PlanError;
use crate::run::reach::reachable_runtimes;
use crate::run::validate::EditScope;

/// Why [`apply_batch`] refused a batch: decision 13's plan errors, or a refusal of the
/// whole batch (a `pause` or `resume` that does not fit, a runtime check).
pub(super) enum Refused {
    Plan(Vec<PlanError>),
    Text(String),
}

/// An applied batch: the reply's text, and the ids of the tasks it added.
pub(super) struct Applied {
    pub text: String,
    pub added: Vec<String>,
}

/// Decision 13, engine side, for every source (milestone 9 decision 19 applies the
/// orchestrator's `edit_plan` through it): the batch goes through `apply_edits`; a live
/// session of a cancelled task is killed (its worktree is removed once no session is
/// left, `dispatch::remove_cancelled_worktrees`); a message is queued, and held while
/// the task carries the N5 hold (`dispatch::enforce_holds`). `pause` pauses a running
/// run with its sessions alive and `resume` resumes a paused one (decision 45); a batch
/// whose `pause` or `resume` does not fit the run's state is refused whole. `finish` is
/// decision 37's (`complete::finish_pass`). A refused batch changes nothing.
pub(super) fn apply_batch(
    run: &mut Run,
    (edits, scope, refusals): (&[PlanEdit], &EditScope, &[(Runtime, String)]),
    source: &EditSource,
    now: u64,
    fx: &mut Vec<Effect>,
) -> Result<Applied, Refused> {
    pause_or_resume_fits(run, edits).map_err(Refused::Text)?;
    let (edited, consequences) =
        apply_edits(run, edits, scope, source, now).map_err(Refused::Plan)?;
    // Ruling T22-I1b: decisions 50 and 53 hold for every runtime the edited run can
    // reach; one it could not reach before, whose checks failed, refuses the edit.
    let before = reachable_runtimes(run);
    let widened = reachable_runtimes(&edited)
        .into_iter()
        .filter(|runtime| !before.contains(runtime));
    for runtime in widened {
        if let Some((_, text)) = refusals.iter().find(|(r, _)| *r == runtime) {
            return Err(Refused::Text(text.clone()));
        }
    }
    // Ruling T14-R2 (#4): the reply says which cancels wait on an in-flight merge.
    let deferred: Vec<String> = edited
        .tasks
        .iter()
        .filter(|t| t.cancel_deferred)
        .filter(|t| {
            !run.tasks
                .iter()
                .any(|was| was.id() == t.id() && was.cancel_deferred)
        })
        .map(|t| t.id().to_string())
        .collect();
    let added: Vec<String> = edited
        .tasks
        .iter()
        .filter(|t| run.task(t.id()).is_none())
        .map(|t| t.id().to_string())
        .collect();
    // M8b decision 19: the unstarted tasks this batch added or amended.
    let touched = touched_unstarted(&edited, edits);
    *run = edited;
    for consequence in consequences {
        match consequence {
            EditConsequence::CancelLive { task_id } => kill_sessions(run, &task_id, fx),
            // A held task keeps its message until it resumes (`dispatch::enforce_holds`).
            EditConsequence::Deliver { task_id, text } => outbox::queue(run, &task_id, text, now),
            // Decision 37: the scheduler's `complete::finish_pass` does the rest.
            EditConsequence::Finish => {
                run.finish_edit = true;
                log(run, now, "the finish edit: no new task starts");
            }
            // Decision 45: dispatch, gates and deliveries stop; a turn already open runs
            // to its end, and every session stays alive.
            EditConsequence::Pause => {
                run.state = RunState::Paused;
                run.paused_from = Some(RunState::Running);
                log(run, now, "paused by a plan edit");
            }
            EditConsequence::Resume => restore::unpause(run, now, fx),
        }
    }
    deciders::cross_check(run, &touched, now, fx);
    edit_log::record(run, edits, now);
    let n = edits.len();
    log(
        run,
        now,
        format!("applied {n} plan edit{}", if n == 1 { "" } else { "s" }),
    );
    let mut text = format!("applied {n} edit{}", if n == 1 { "" } else { "s" });
    for task in &deferred {
        text.push_str(&super::complete::deferred_note(task));
    }
    Ok(Applied { text, added })
}

/// M8b decision 19's edit side: decision 13's touched set (the tasks the batch added,
/// split into, amended or gave a dependency), those still pending or queued (none
/// dispatched), in plan order.
fn touched_unstarted(edited: &Run, edits: &[PlanEdit]) -> Vec<String> {
    // Keep in sync with `run::edits::Batch.touched`, which records the same set.
    let mut touched: Vec<&str> = Vec::new();
    for edit in edits {
        match edit {
            PlanEdit::AddTask { task } => touched.push(&task.id),
            PlanEdit::SplitTask { into, .. } => touched.extend(into.iter().map(|t| t.id.as_str())),
            PlanEdit::AmendTask { task_id, .. } | PlanEdit::AddDep { task_id, .. } => {
                touched.push(task_id)
            }
            _ => {}
        }
    }
    edited
        .tasks
        .iter()
        .filter(|t| touched.contains(&t.id()))
        .filter(|t| matches!(t.state, TaskState::Pending | TaskState::Queued))
        .map(|t| t.id().to_string())
        .collect()
}

fn kill_sessions(run: &mut Run, task_id: &str, fx: &mut Vec<Effect>) {
    let Some(task) = run.tasks.iter_mut().find(|t| t.spec.id == task_id) else {
        return;
    };
    for round in task.rounds.iter_mut().filter(|r| !r.ended) {
        if let Some(window_id) = round.window_id {
            round.retiring = true;
            fx.push(Effect::KillWindow { window_id });
        }
    }
}

/// Decision 45: a `pause` edit applies only to a running run and a `resume` edit only to
/// a paused one, taken in batch order.
fn pause_or_resume_fits(run: &Run, edits: &[PlanEdit]) -> Result<(), String> {
    let mut state = run.state;
    for edit in edits {
        let (from, to, verb) = match edit {
            PlanEdit::Pause => (
                RunState::Running,
                RunState::Paused,
                "only a running run can be paused",
            ),
            PlanEdit::Resume => (
                RunState::Paused,
                RunState::Running,
                "only a paused run can be resumed",
            ),
            _ => continue,
        };
        if state != from {
            return Err(format!("run {} is {}; {verb}", run.id, state.label()));
        }
        state = to;
    }
    Ok(())
}
