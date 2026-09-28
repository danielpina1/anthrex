//! Client requests: start and the plan gate (decision 14), approve, reject (decision 20's
//! discard), `run edit` (decision 13; the batch itself is `batch.rs`), and `run retry`
//! (decision 42). Pure (design decision 2). M8a.14's
//! `complete.rs` and `merge.rs` answer cancel, finish and a halted run's resume;
//! `restore.rs` (M8a.15) answers restore and a paused run's resume.

use crate::run::phases::set_state;
use proto::{
    BlockReason, DeciderSource, PlanEdit, RunPath, RunState, Runtime, Size, SizeCheckInfo,
    TaskState,
};

use super::batch::{Refused, apply_batch};
use super::dispatch::{finishing_as, history, salvage_ref};
use super::schedule::deps_done;
use super::signals::end_round;
use super::{
    Effect, EngineState, OpKind, OpResult, ReplyId, deciders, emit_op, ladder, next_op, review,
};
use crate::decider::fallback::SIZED_BY_TRIAGE;
use crate::run::env::profile_env;
use crate::run::model::{FreshSession, LogEntry, Run, SizeCheckState};
use crate::run::orch::EditSource;
use crate::run::roster::escalate;
use crate::run::triage::fast_refusal;
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
    let fast = run.path == Some(RunPath::Fast);
    // Review I1: the engine's own barrier. A fast-path run is one task, neither hub nor
    // L, whatever its caller checked; any other is refused and nothing is created.
    if let Some(reason) = fast.then(|| fast_refusal(&run)).flatten() {
        return reply(fx, id, Err(reason));
    }
    if fast {
        // M8b decision 24: a fast-path run has no plan gate.
        run.state = RunState::Running;
        run.approved_at = Some(now);
        log(&mut run, now, "started on the fast path; no plan gate");
    } else if run.state == RunState::Planning {
        // Milestone 9 decision 26: no task and no gate yet; the orchestrator plans.
        log(&mut run, now, "started; planning with its orchestrator");
    } else if run.approved_by.as_deref() == Some("--yes") {
        run.state = RunState::Running;
        run.approved_at = Some(now);
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
    if run.state == RunState::Planning {
        super::orch_window::launch(&mut run, fx);
    }
    // M8b decision 19: every task is cross-checked; one waiting is not runnable. The
    // fast path's task is not: triage sized it a moment earlier (ruling R-T13-1).
    if fast {
        for task in &mut run.tasks {
            task.size_check = Some(SizeCheckState::Done(SizeCheckInfo {
                engine: task.size,
                decided: None,
                agreed: true,
                reason: SIZED_BY_TRIAGE.into(),
                source: DeciderSource::Fallback,
            }));
        }
    } else {
        let ids: Vec<String> = run.tasks.iter().map(|t| t.id().to_string()).collect();
        deciders::cross_check(&mut run, &ids, now, fx);
    }
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
    if run.state == RunState::Planning {
        let text = format!(
            "run {run_id} is still being planned; approve it when the orchestrator has submitted the plan"
        );
        return reply(fx, id, Err(text));
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
    run.approved_at = Some(now);
    // Task 12 review m7: a decider queued at the gate (a size check) could not start
    // there; its slot wait counts from now.
    for q in &mut run.decider_queue {
        q.queued_at = now;
    }
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
    // Milestone 9 decision 26: a run still being planned is discarded too, also when a
    // daemon restart paused it there (M9.7 review fixes, ruling 4).
    let planning = run.state == RunState::Planning
        || (run.state == RunState::Paused && run.paused_from == Some(RunState::Planning));
    if !planning && run.state != RunState::AwaitingApproval {
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
    // Task 12 review m7: a size check queued at the gate never starts beside the
    // discard (none can be in flight: deciders start only while the run runs).
    run.decider_queue.clear();
    for task in &mut run.tasks {
        task.drop_pending_size_check();
    }
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

/// `run edit` (decision 13): the user's batch, through `batch::apply_batch`, and with
/// `submit`, milestone 9 decision 13's user submit of a planning run's plan.
pub(super) fn edit(
    state: &mut EngineState,
    id: ReplyId,
    run_id: &str,
    (edits, scope, refusals, submit): (&[PlanEdit], &EditScope, &[(Runtime, String)], bool),
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
    // Whole-branch review m1: a fast-path run runs one task; milestone 9's promotion,
    // not an edit, turns it into a planned run (`promote.rs`; a promoted run's path is
    // `plan`, so this no longer applies to it).
    let adds = |e: &PlanEdit| matches!(e, PlanEdit::AddTask { .. } | PlanEdit::SplitTask { .. });
    if run.path == Some(RunPath::Fast) && edits.iter().any(adds) {
        let text = format!(
            "run {run_id} is on the fast path: it runs one task; start a planned run instead"
        );
        return reply(fx, id, Err(text));
    }
    let batch = (edits, scope, refusals);
    if submit {
        let result = submit_edit(run, batch, now, fx);
        return reply(fx, id, result);
    }
    match apply_batch(run, batch, &EditSource::User, now, fx) {
        Ok(applied) => reply(fx, id, Ok(applied.text)),
        Err(Refused::Text(text)) => reply(fx, id, Err(text)),
        Err(Refused::Plan(errors)) => {
            let lines: Vec<String> = errors.iter().map(ToString::to_string).collect();
            reply(fx, id, Err(lines.join("\n")))
        }
    }
}

/// Decision 13's user submit (M9.7 review fixes, ruling 5): a planning run whose
/// orchestrator cannot go on is completed by the user alone. The batch and the submit
/// are one unit, applied to a copy, as the orchestrator's `edit_plan` is; the submit is
/// the orchestrator's (`orch::submit_plan`), and the user then approves as usual.
fn submit_edit(
    run: &mut Run,
    batch: (&[PlanEdit], &EditScope, &[(Runtime, String)]),
    now: u64,
    fx: &mut Vec<Effect>,
) -> Result<String, String> {
    if run.state != RunState::Planning {
        let label = run.state.label();
        return Err(format!(
            "run {} is {label}; only a run being planned can be submitted",
            run.id
        ));
    }
    let mut edited = run.clone();
    let mut effects = Vec::new();
    let mut text = None;
    if !batch.0.is_empty() {
        match apply_batch(&mut edited, batch, &EditSource::User, now, &mut effects) {
            Ok(applied) => text = Some(applied.text),
            Err(Refused::Text(text)) => return Err(text),
            Err(Refused::Plan(errors)) => {
                let lines: Vec<String> = errors.iter().map(ToString::to_string).collect();
                return Err(lines.join("\n"));
            }
        }
    }
    super::orch::submit_plan(&mut edited, "the user", now)?;
    *run = edited;
    fx.extend(effects);
    let outcome = if run.state == RunState::Running {
        "approved by --yes, it runs"
    } else {
        "it awaits approval"
    };
    let submitted = format!("the plan of run {} was submitted: {outcome}", run.id);
    Ok(match text {
        Some(text) => format!("{text}; {submitted}"),
        None => submitted,
    })
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
    // Review I1: an earlier block's classification no longer applies.
    task.pending_classification = None;
    task.bounces = Default::default();
    task.budget_exceeded = 0;
    task.conflicts = 0;
    task.rung = 2;
    // M8b decision 33a: the next worker launch records this escalation, its pool
    // stepping from the route the selector stepped from (a second escalation before
    // the launch overwrites the first: the intermediate route never ran).
    task.escalated_from = Some(std::mem::replace(&mut task.route, route));
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
        set_state(task, TaskState::Queued, now);
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
            set_state(task, TaskState::Working, now);
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
