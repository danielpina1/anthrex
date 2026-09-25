//! Client requests: start and the plan gate (decision 14), approve, reject (decision 20's
//! discard), plan edits (decision 13, engine side, with decision 45's `pause` and
//! `resume` edits), and `run retry` (decision 42). Pure (design decision 2). M8a.14's
//! `complete.rs` and `merge.rs` answer cancel, finish and a halted run's resume;
//! `restore.rs` (M8a.15) answers restore and a paused run's resume.

use proto::{BlockReason, PlanEdit, RunState, Runtime, Size, TaskState};

use super::dispatch::{finishing_as, history, salvage_ref};
use super::schedule::deps_done;
use super::signals::end_round;
use super::{
    Effect, EngineState, OpKind, OpResult, ReplyId, emit_op, ladder, next_op, outbox, restore,
    review,
};
use crate::run::edits::{EditConsequence, apply_edits};
use crate::run::env::profile_env;
use crate::run::model::{FreshSession, LogEntry, Run};
use crate::run::reach::reachable_runtimes;
use crate::run::roster::escalate;
use crate::run::validate::EditScope;

/// At most this many log entries per run (Interfaces, `LogEntry`).
const LOG_MAX: usize = 500;

pub(super) fn log(run: &mut Run, now: u64, text: impl Into<String>) {
    run.log.push(LogEntry {
        at: now,
        text: text.into(),
    });
    if run.log.len() > LOG_MAX {
        let excess = run.log.len() - LOG_MAX;
        run.log.drain(..excess);
    }
}

fn reply(fx: &mut Vec<Effect>, reply: ReplyId, result: Result<String, String>) {
    fx.push(Effect::Reply { reply, result });
}

fn unknown(run_id: &str) -> String {
    format!("unknown run {run_id}")
}

/// Decision 14: `run start` creates the run branch and the integration worktree at once
/// and replies with the run id; with `--yes` (recorded by `build_run` as
/// `approved_by == "--yes"`) the run is `running` from the start, else it waits at the
/// gate. Dispatch or the pre-warm follows the `CreateRunBranch` result.
pub(super) fn start(
    state: &mut EngineState,
    id: ReplyId,
    mut run: Run,
    now: u64,
    fx: &mut Vec<Effect>,
) {
    if state.runs.contains_key(&run.id) {
        return reply(fx, id, Err(format!("run {} already exists", run.id)));
    }
    if run.approved_by.as_deref() == Some("--yes") {
        run.state = RunState::Running;
        log(&mut run, now, "started; approved by --yes");
    } else {
        log(&mut run, now, "started; awaiting approval");
    }
    let op = next_op(&mut run);
    let path = run.integration_path();
    let kind = OpKind::CreateRunBranch {
        root: run.root.clone(),
        branch: run.run_branch(),
        base_sha: run.base_sha.clone(),
        path: path.clone(),
        setup: run.profile.setup.clone(),
        env: profile_env(&run.profile, &path),
    };
    emit_op(&mut run, op, None, kind, fx);
    reply(fx, id, Ok(run.id.clone()));
    state.runs.insert(run.id.clone(), run);
}

/// The integration worktree's result. A run whose integration worktree cannot be made
/// is `failed` (decision 31); a failed setup there counts the same way.
pub(super) fn run_branch_done(run: &mut Run, result: OpResult, now: u64, fx: &mut Vec<Effect>) {
    let failure = match result {
        OpResult::Worktree { .. } => {
            fx.push(Effect::WatchWorktree {
                root: run.integration_path(),
            });
            log(run, now, "integration worktree ready");
            return;
        }
        OpResult::SetupFailed { output } => {
            format!("setup failed in the integration worktree:\n{output}")
        }
        OpResult::Failed { message } => {
            format!("could not create the integration worktree: {message}")
        }
        _ => return,
    };
    run.state = RunState::Failed;
    log(run, now, failure.clone());
    run.outcome = Some(failure);
}

pub(super) fn approve(
    state: &mut EngineState,
    id: ReplyId,
    run_id: &str,
    now: u64,
    fx: &mut Vec<Effect>,
) {
    let Some(run) = state.runs.get_mut(run_id) else {
        return reply(fx, id, Err(unknown(run_id)));
    };
    if let Some(how) = finishing_as(run) {
        return reply(fx, id, Err(format!("run {run_id} is being {how}")));
    }
    if run.state != RunState::AwaitingApproval {
        return reply(
            fx,
            id,
            Err(format!("run {run_id} is {}", run.state.label())),
        );
    }
    run.state = RunState::Running;
    run.approved_by = Some("user".to_string());
    log(run, now, "approved by the user");
    reply(fx, id, Ok(format!("run {run_id} approved")));
}

/// Decision 14: `run reject` discards the run (decision 20): every worktree salvaged and
/// removed, every `anthrex/<run>/` branch deleted. The run is `discarded` when the op
/// comes back; until then nothing more starts.
pub(super) fn reject(
    state: &mut EngineState,
    id: ReplyId,
    run_id: &str,
    now: u64,
    fx: &mut Vec<Effect>,
) {
    let Some(run) = state.runs.get_mut(run_id) else {
        return reply(fx, id, Err(unknown(run_id)));
    };
    if let Some(how) = finishing_as(run) {
        return reply(fx, id, Err(format!("run {run_id} is being {how}")));
    }
    if run.state != RunState::AwaitingApproval {
        let label = run.state.label();
        let text =
            format!("run {run_id} is {label}; reject applies only while its plan awaits approval");
        return reply(fx, id, Err(text));
    }
    let mut worktrees: Vec<_> = run
        .tasks
        .iter()
        .map(|t| {
            let seq = t.salvage_refs.len() + 1;
            (t.worktree.clone(), salvage_ref(run, t.id(), seq))
        })
        .collect();
    worktrees.push((run.integration_path(), salvage_ref(run, "integration", 1)));
    let op = next_op(run);
    let kind = OpKind::Discard {
        root: run.root.clone(),
        worktrees,
        branch_prefix: format!("anthrex/{run_id}/"),
    };
    emit_op(run, op, None, kind, fx);
    log(run, now, "rejected by the user; discarding");
    reply(fx, id, Ok(format!("run {run_id} rejected; discarding it")));
}

/// Decision 13, engine side: the batch goes through `apply_edits`; a live session of a
/// cancelled task is killed (its worktree is removed once no session is left,
/// `dispatch::remove_cancelled_worktrees`); a message is queued, and held while the task
/// carries the N5 hold (`dispatch::enforce_holds`). `pause` pauses a running run with
/// its sessions alive and `resume` resumes a paused one (decision 45); a batch whose
/// `pause` or `resume` does not fit the run's state is refused whole. `finish` is
/// decision 37's (`complete::finish_pass`).
pub(super) fn edit(
    state: &mut EngineState,
    id: ReplyId,
    run_id: &str,
    (edits, scope, refusals): (&[PlanEdit], &EditScope, &[(Runtime, String)]),
    now: u64,
    fx: &mut Vec<Effect>,
) {
    let Some(run) = state.runs.get_mut(run_id) else {
        return reply(fx, id, Err(unknown(run_id)));
    };
    if run.state.is_terminal() || run.state == RunState::Complete {
        return reply(
            fx,
            id,
            Err(format!("run {run_id} is {}", run.state.label())),
        );
    }
    if let Err(text) = pause_or_resume_fits(run, edits) {
        return reply(fx, id, Err(text));
    }
    let (edited, consequences) = match apply_edits(run, edits, scope, now) {
        Ok(ok) => ok,
        Err(errors) => {
            let lines: Vec<String> = errors.iter().map(ToString::to_string).collect();
            return reply(fx, id, Err(lines.join("\n")));
        }
    };
    // Ruling T22-I1b: decisions 50 and 53 hold for every runtime the edited run can
    // reach; one it could not reach before, whose checks failed, refuses the edit.
    let before = reachable_runtimes(run);
    let widened = reachable_runtimes(&edited)
        .into_iter()
        .filter(|runtime| !before.contains(runtime));
    for runtime in widened {
        if let Some((_, text)) = refusals.iter().find(|(r, _)| *r == runtime) {
            return reply(fx, id, Err(text.clone()));
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
    reply(fx, id, Ok(text));
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

/// `run retry` (decision 42) of a blocked task that is neither L nor `dep_cancelled`:
/// `failures = 1`, `bounces`, `budget_exceeded` and `conflicts` cleared, rung 2 on
/// `roster::escalate(route)` (decision 38's rung 2), and a fresh session in the same
/// worktree from the task's own start commit (carry M8a.11), with decision 30's
/// hand-over prompt; its old session, if alive, is killed first. The hand-back context
/// ends (carry T14-R2), so the fresh session's claim passes every gate. A task that
/// never started is dispatched again instead. A held task (M8a.6 ruling N5) waits for
/// its dependencies, then has the run head handed back before the fresh session, and
/// keeps its own block until then (`holds::resume_held`, carries T11-RR and M8a.14).
pub(super) fn retry(
    state: &mut EngineState,
    id: ReplyId,
    run_id: &str,
    task_id: &str,
    now: u64,
    fx: &mut Vec<Effect>,
) {
    let Some(run) = state.runs.get_mut(run_id) else {
        return reply(fx, id, Err(unknown(run_id)));
    };
    if let Some(how) = finishing_as(run) {
        return reply(fx, id, Err(format!("run {run_id} is being {how}")));
    }
    if !matches!(run.state, RunState::Running | RunState::Paused) {
        let text = format!("run {run_id} is {}", run.state.label());
        return reply(fx, id, Err(text));
    }
    let Some(i) = run.tasks.iter().position(|t| t.id() == task_id) else {
        return reply(fx, id, Err(format!("unknown task {task_id}")));
    };
    let task = &run.tasks[i];
    let refusal = match &task.block {
        _ if task.state != TaskState::Blocked => Some(format!(
            "task {task_id} is {}; retry applies only to a blocked task",
            task.state.label()
        )),
        Some(b) if b.reason == BlockReason::DepCancelled => Some(format!(
            "task {task_id} is blocked(dep_cancelled); retry cannot bring back a cancelled dependency"
        )),
        _ if task.size == Size::L => Some(format!("task {task_id} is L; split it first")),
        _ if task.awaiting_deps && !deps_done(run, task) => Some(format!(
            "task {task_id} waits for its dependencies; retry it once they are merged"
        )),
        _ => None,
    };
    if let Some(text) = refusal {
        return reply(fx, id, Err(text));
    }
    let block = task.block.clone();
    let route = escalate(&run.roster, &task.route);
    ladder::kill_worker(run, i, fx);
    review::stop_reviewers(run, i, now, fx);
    let task = &mut run.tasks[i];
    // A launch the restart lost belongs to the session being replaced.
    for round in task.rounds.iter_mut().filter(|r| r.relaunch.is_some()) {
        round.relaunch = None;
        end_round(round, now);
    }
    task.failures = 1;
    task.bounces = Default::default();
    task.budget_exceeded = 0;
    task.conflicts = 0;
    task.rung = 2;
    task.route = route;
    // Ruling T15-C1: a new budget epoch; rung 4 counts from the fresh session.
    super::clock::new_epoch(task);
    // `kill_worker`'s `supersede` ended the hand-back context (`handed_back`,
    // `resolution`); its gates' pass goes too (carry T14-R2).
    task.gates_after_handback = false;
    let was = block.map_or_else(String::new, |b| {
        let label = serde_json::to_value(b.reason)
            .ok()
            .and_then(|v| v.as_str().map(str::to_string))
            .unwrap_or_default();
        format!(" (it was blocked({label}): {})", b.text)
    });
    let how = if task.start_commit.is_none() {
        task.state = TaskState::Queued;
        task.block = None;
        task.fresh_session = None;
        "it is dispatched again"
    } else {
        task.fresh_session = Some(FreshSession {
            reason: format!("the user retried it{was}"),
            append: None,
        });
        if task.awaiting_deps {
            task.held_answered = true;
            "the run head is merged into its worktree first"
        } else {
            task.state = TaskState::Working;
            task.block = None;
            "a fresh session starts"
        }
    };
    history(run, i, now, format!("retried by the user at rung 2{was}"));
    log(run, now, format!("{task_id} retried at rung 2"));
    reply(
        fx,
        id,
        Ok(format!("task {task_id} retried at rung 2: {how}")),
    );
}
