//! Decision 13, engine side: one plan-edit batch applied to a run, for every source
//! (the user's `run edit`, and milestone 9's orchestrator through `edit_plan`, decision
//! 19). Moved out of `requests.rs` by task M9.7, which shares it. Pure (design decision
//! 2).

use proto::{BlockReason, PlanEdit, RunState, Runtime, TaskState};

use super::actions::rules;
use super::requests::log;
use super::{Effect, deciders, outbox, restore, worker_messages};
use crate::run::edit_log;
use crate::run::edits::{EditConsequence, apply_edits};
use crate::run::edits_orch::MessageOutcome;
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

/// An applied batch: the reply's text, the ids of the tasks it added, and a
/// `message`'s recipients (milestone 9 decision 42b).
pub(super) struct Applied {
    pub text: String,
    pub added: Vec<String>,
    pub message: Option<MessageOutcome>,
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
    // Milestone 9 decision 25: the mis-sized tasks this batch rewrote.
    let rewritten = rewritten(run, &edited);
    *run = edited;
    let mut message = None;
    for consequence in consequences {
        match consequence {
            EditConsequence::CancelLive { task_id } => kill_sessions(run, &task_id, fx),
            // A held task keeps its message until it resumes (`dispatch::enforce_holds`).
            EditConsequence::Deliver { task_id, text } => outbox::queue(run, &task_id, text, now),
            // Milestone 9 decision 42b: linked to the message the task recorded.
            EditConsequence::Message { task_id, text } => {
                worker_messages::queue(run, &task_id, text, now)
            }
            EditConsequence::Recipients(outcome) => message = Some(outcome),
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
            // Milestone 9 decision 42c: the run's `resume` releases its paused tasks.
            EditConsequence::Resume => {
                worker_messages::release_all(run, now);
                restore::unpause(run, now, fx);
            }
        }
    }
    deciders::cross_check(run, &touched, now, fx);
    restart_rewritten(run, &rewritten, source, now, fx);
    let recipients = message.as_ref().map(|m| m.delivered.clone());
    let outcome = edit_log::EditOutcome::Accepted {
        recipients: recipients.unwrap_or_default(),
    };
    edit_log::record(run, edits, now, source, outcome);
    // Milestone 9 decision 39: the orchestrator sees what the user changed.
    if *source == EditSource::User {
        let text = format!("the user edited the plan: {}", edit_log::describe(edits));
        super::wake::note(run, text);
    }
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
    if let Some(outcome) = &message {
        text.push_str(&format!("; message for {}", outcome.delivered.join(", ")));
        for (_, reason) in &outcome.refused {
            text.push_str(&format!("\nnot delivered: {reason}"));
        }
    }
    Ok(Applied {
        text,
        added,
        message,
    })
}

/// Decision 40: a rejected batch of the orchestrator's or a sub-planner's is logged
/// with its first error; a call with no edit (a lone `submit`) logs nothing.
pub(super) fn record_rejected(
    run: &mut Run,
    edits: &[PlanEdit],
    source: &EditSource,
    error: String,
    now: u64,
) {
    if !edits.is_empty() {
        edit_log::record(
            run,
            edits,
            now,
            source,
            edit_log::EditOutcome::Rejected { error },
        );
    }
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
        let (refusal, to) = match edit {
            PlanEdit::Pause => (rules::pause_in(&run.id, state), RunState::Paused),
            PlanEdit::Resume => (rules::unpause_in(&run.id, state), RunState::Running),
            _ => continue,
        };
        if let Some(text) = refusal {
            return Err(text);
        }
        state = to;
    }
    Ok(())
}

/// M9.9 review fixes, M2: decision 25 restarts a task at most this many times for the
/// orchestrator's and sub-planners' rewrites; the next blocks it for the user, whose
/// own rewrite is never counted and whose `run retry` starts the count again.
pub const MAX_REWRITE_RESTARTS: u32 = 3;

/// Milestone 9 decision 25: the tasks in `blocked(mis_sized)` whose `brief`,
/// `acceptance`, `size` or `route` the batch that made `edited` from `run` changed, and
/// which are still so blocked. Every source's batch (`batch::apply_batch`).
fn rewritten(run: &Run, edited: &Run) -> Vec<String> {
    let mis_sized = |t: &crate::run::model::Task| {
        t.state == TaskState::Blocked
            && t.block
                .as_ref()
                .is_some_and(|b| b.reason == BlockReason::MisSized)
    };
    edited
        .tasks
        .iter()
        .filter(|t| mis_sized(t))
        .filter(|t| {
            run.task(t.id()).is_some_and(|old| {
                mis_sized(old)
                    && (old.spec.brief != t.spec.brief
                        || old.spec.acceptance != t.spec.acceptance
                        || old.spec.size != t.spec.size
                        || old.spec.route != t.spec.route)
            })
        })
        .map(|t| t.id().to_string())
        .collect()
}

/// Decision 25: a rewritten mis-sized task re-enters exactly as `run retry` does (a
/// fresh session at rung 2, `failures = 1`), once the batch is applied. One that is
/// still L, or that waits for new dependencies, keeps its block, as `run retry` would
/// refuse it. `blocked(human)`, `(conflict)` and `(environment)` are never restarted
/// by an edit: only `run retry` lifts them.
fn restart_rewritten(
    run: &mut Run,
    ids: &[String],
    source: &EditSource,
    now: u64,
    fx: &mut Vec<Effect>,
) {
    // M9.9 second review, M-c: the cap stops a model's rewrite loop; the user's own
    // rewrite always restarts, and is not counted.
    let counted = *source != EditSource::User;
    for id in ids {
        let Some(i) = run.tasks.iter().position(|t| t.id() == id) else {
            continue;
        };
        let task = &run.tasks[i];
        if task.size == proto::Size::L
            || (task.awaiting_deps && !super::schedule::deps_done(run, task))
        {
            continue;
        }
        // M9.9 review fixes, M2: past MAX_REWRITE_RESTARTS the rewrite stands and
        // the user decides.
        if counted && task.orch.rewrite_restarts >= MAX_REWRITE_RESTARTS {
            let text = format!("rewritten {MAX_REWRITE_RESTARTS} times; the user decides");
            super::dispatch::block(run, i, BlockReason::Environment, text, now);
            continue;
        }
        if counted {
            run.tasks[i].orch.rewrite_restarts += 1;
        }
        let task = &run.tasks[i];
        let text = task
            .block
            .as_ref()
            .map_or_else(String::new, |b| b.text.clone());
        let why = format!("its plan was rewritten (it was blocked(mis_sized): {text})");
        let how = super::requests::rung2(run, i, why, now, fx);
        super::dispatch::history(
            run,
            i,
            now,
            format!("rewritten by a plan edit; restarted at rung 2: {how}"),
        );
    }
}
