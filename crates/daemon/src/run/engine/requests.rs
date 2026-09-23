//! Client requests: start and the plan gate (decision 14), approve, reject (decision 20's
//! discard), plan edits (decision 13, engine side, first part), and restore (decision 45,
//! first part). Pure (design decision 2). M8a.14 and M8a.15 add retry, override,
//! cancel, resume, finish and the rest of restore.

use proto::{PlanEdit, RunState};

use super::dispatch::{finishing_as, salvage_ref};
use super::{Effect, EngineState, OpKind, OpResult, ReplyId, emit_op, next_op, outbox};
use crate::run::edits::{EditConsequence, apply_edits};
use crate::run::env::profile_env;
use crate::run::model::{LogEntry, Run};
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

/// The result of a `Discard`.
pub(super) fn discarded(run: &mut Run, result: OpResult, now: u64) {
    match result {
        OpResult::Finished { outcome } => {
            run.state = RunState::Discarded;
            log(run, now, format!("discarded: {outcome}"));
            run.outcome = Some(outcome);
        }
        OpResult::Failed { message } => log(run, now, format!("discard failed: {message}")),
        _ => {}
    }
}

/// Decision 13, engine side (first part): the batch goes through `apply_edits`; a live
/// session of a cancelled task is killed (its worktree is removed once no session is
/// left, `dispatch::remove_cancelled_worktrees`); a message is queued, and held while the
/// task carries the N5 hold (`dispatch::enforce_holds`). `pause`, `resume` and `finish`
/// are M8a.15's.
pub(super) fn edit(
    state: &mut EngineState,
    id: ReplyId,
    run_id: &str,
    edits: &[PlanEdit],
    scope: &EditScope,
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
    let (edited, consequences) = match apply_edits(run, edits, scope, now) {
        Ok(ok) => ok,
        Err(errors) => {
            let lines: Vec<String> = errors.iter().map(ToString::to_string).collect();
            return reply(fx, id, Err(lines.join("\n")));
        }
    };
    *run = edited;
    for consequence in consequences {
        match consequence {
            EditConsequence::CancelLive { task_id } => kill_sessions(run, &task_id, fx),
            // A held task keeps its message until it resumes (`dispatch::enforce_holds`).
            EditConsequence::Deliver { task_id, text } => outbox::queue(run, &task_id, text, now),
            EditConsequence::Pause | EditConsequence::Resume | EditConsequence::Finish => {}
        }
    }
    let n = edits.len();
    log(
        run,
        now,
        format!("applied {n} plan edit{}", if n == 1 { "" } else { "s" }),
    );
    reply(
        fx,
        id,
        Ok(format!("applied {n} edit{}", if n == 1 { "" } else { "s" })),
    );
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

/// Decision 45, first part: restored runs keep their state except `running`, which
/// becomes `paused` (so `awaiting_approval` survives a restart unchanged, decision 14);
/// journaled results are replayed. M8a.15 adds the rest (sessions marked ended,
/// deadlines, re-issued ops).
pub(super) fn restore(state: &mut EngineState, runs: Vec<Run>, now: u64) {
    for mut run in runs {
        // Ruling T12-I1: no session outlives a restart, so no claim or count does
        // either; the reply ids belonged to the old daemon.
        for task in run.tasks.iter_mut() {
            task.claim = None;
            for round in task.rounds.iter_mut() {
                if round.fallback == crate::run::model::FallbackState::Counting {
                    round.fallback = crate::run::model::FallbackState::None;
                }
            }
        }
        if run.state == RunState::Running {
            run.state = RunState::Paused;
            run.paused_from = Some(RunState::Running);
            log(&mut run, now, "restored after a daemon restart; paused");
            // Decision 47: a changed run bumps its revision (review minor 5); `step`
            // leaves a run new to the state at the revision it arrived with.
            run.revision += 1;
        }
        state.runs.insert(run.id.clone(), run);
    }
}

/// A request a later task implements (M8a.12 to M8a.15).
pub(super) fn not_yet(fx: &mut Vec<Effect>, id: ReplyId, what: &str) {
    reply(fx, id, Err(format!("{what} is not available yet")));
}
