//! Client requests: start and the plan gate (decision 14), approve, reject (decision 20's
//! discard), `run edit` (decision 13; the batch itself is `batch.rs`), and `run retry`
//! (decision 42). Pure (design decision 2). M8a.14's
//! `complete.rs` and `merge.rs` answer cancel, finish and a halted run's resume;
//! `restore.rs` (M8a.15) answers restore and a paused run's resume.

use crate::run::phases::set_state;
use proto::{DeciderSource, PlanEdit, RunPath, RunState, Runtime, SizeCheckInfo, TaskState};

use super::actions::rules;
use super::batch::{Refused, apply_batch};
use super::dispatch::{history, salvage_ref};
use super::signals::end_round;
use super::stages;
use super::{
    Effect, EngineState, OpKind, OpResult, ReplyId, deciders, emit_op, ladder, next_op, review,
};
use crate::decider::fallback::SIZED_BY_TRIAGE;
use crate::run::env::profile_env;
use crate::run::model::{FreshSession, LogEntry, Run, SizeCheckState};
use crate::run::orch::EditSource;
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
    // Milestone 9.3 decision 18: every run starts in round 1.
    super::goal_rounds::ensure_first(&mut run);
    // Milestone 9.3 decision 19: a run with an orchestrator starts its chain, unless it
    // continues one (decisions 23, 24: it joins it, or is refused).
    let continued = run.chain.is_some();
    super::chains::assign(&mut run);
    let fast = run.path == Some(RunPath::Fast);
    // Review I1: the engine's own barrier. A fast-path run is one task, neither hub nor
    // L, whatever its caller checked; any other is refused and nothing is created.
    if let Some(reason) = fast.then(|| fast_refusal(&run)).flatten() {
        return reply(fx, id, Err(reason));
    }
    let join = match super::chains::join(state, &mut run, continued, now, fx) {
        Ok(join) => join,
        Err(text) => return reply(fx, id, Err(text)),
    };
    if fast {
        // M8b decision 24: a fast-path run has no plan gate.
        run.state = RunState::Running;
        run.approved_at = Some(now);
        stages::fix_layout(&mut run, now);
        log(&mut run, now, "started on the fast path; no plan gate");
    } else if matches!(run.state, RunState::Planning | RunState::Brainstorming) {
        // Milestone 9 decision 26: no task and no gate yet; the orchestrator plans (or,
        // milestone 9.6 decision 4, brainstorms first).
        let text = format!("started; {} with its orchestrator", run.state.label());
        log(&mut run, now, text);
    } else if run.approved_by.as_deref() == Some("--yes") {
        run.state = RunState::Running;
        run.approved_at = Some(now);
        stages::fix_layout(&mut run, now);
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
    let planned = matches!(run.state, RunState::Planning | RunState::Brainstorming);
    if planned && join != super::chains::Join::Adopted {
        super::orch_window::launch(&mut run, now, fx);
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
    if let Some(text) = rules::approve(run) {
        return reply(fx, id, Err(text));
    }
    // Milestone 9.6 decision 7: a design run's plan gate is its document gate's.
    if super::design_gate::waiting(run).is_some() {
        let (kind, action) = (proto::DocGateKind::Plan, proto::DocGateAction::Approve);
        let result = super::design_gate::act(run, kind, action, now, fx);
        return reply(fx, id, result);
    }
    run.state = RunState::Running;
    // Milestone 9.1 decision 46 (the layout) and task 12 review m7 (a decider queued
    // at the gate waits from now); milestone 9.3 decision 12: round 1's approval only.
    super::goal_rounds_end::approved(run, "user", now);
    log(run, now, "approved by the user");
    // Milestone 9 decisions 30, 39.
    super::wake::note(run, "the user approved the plan".to_string());
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
    // Milestone 9 decision 26: a run still being planned is discarded too.
    if let Some(text) = rules::reject(run) {
        return reply(fx, id, Err(text));
    }
    // Milestone 9.6 ruling T1-O1: at a document gate, the gate's reject.
    if let Some(kind) = super::design_gate::waiting(run).map(|g| g.kind) {
        let action = proto::DocGateAction::Reject;
        let result = super::design_gate::act(run, kind, action, now, fx);
        return reply(fx, id, result);
    }
    // Milestone 9.6 (fix round 1, m6): before its plan, the phase's document.
    let what = super::design_gate::phase_doc(run).map_or("plan", |k| k.label());
    let note = format!("the user rejected the {what}; run discarded");
    let text = reject_run(run, &note, now, fx);
    reply(fx, id, Ok(text));
}

/// The rejected run's discard (decision 20), or its later round's reject (milestone
/// 9.3 decision 12), with the orchestrator's `note`; the reply's text.
pub(super) fn reject_run(run: &mut Run, note: &str, now: u64, fx: &mut Vec<Effect>) -> String {
    // Milestone 9.3 decision 12: a later round's reject keeps the run.
    if super::goal_rounds_end::open_round(run) {
        return super::goal_rounds_end::reject_round(run, now, fx);
    }
    let run_id = run.id.clone();
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
    // Milestone 9.6: a run rejected while it brainstorms stops its brainstormers.
    super::design_agents::halt_all(run, super::design_agents::RUN_REJECTED, fx);
    let op = next_op(run);
    let kind = OpKind::Discard {
        root: run.root.clone(),
        worktrees,
        branch_prefix: format!("anthrex/{run_id}/"),
    };
    emit_op(run, op, None, kind, fx);
    log(run, now, "rejected by the user; discarding");
    super::wake::note(run, note.to_string());
    format!("run {run_id} rejected; discarding it")
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
    // Milestone 9 decision 42's one-edit rule; a fast-path run runs one task (milestone
    // 9's promotion, not an edit, makes it a planned run); a cancelled run only loses work.
    if let Some(text) = rules::edit_run(run, edits, submit) {
        return reply(fx, id, Err(text));
    }
    let batch = (edits, scope, refusals);
    if submit {
        let result = submit_edit(run, batch, now, fx);
        return reply(fx, id, result);
    }
    // Milestone 9.6 ruling T7-7: an edit at the open plan gate is its next version.
    let at_gate = super::design::plan::open_gate(run).then(|| run.tasks.clone());
    match apply_batch(run, batch, &EditSource::User, now, fx) {
        Ok(applied) => {
            let text =
                super::design::plan::user_edited(run, at_gate.as_deref(), applied.text, now, fx);
            reply(fx, id, Ok(text))
        }
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
    // M9.9 review fixes, C1: a promoted running run whose orchestrator has not
    // submitted is submitted by the user as the orchestrator's `submit` would.
    let promoted = rules::promoted_unsubmitted(run);
    if let Some(text) = rules::submit(run) {
        return Err(text);
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
    // Milestone 9.6 decisions 18 and 21: a design run's checks, and its plan gate.
    let author = Some(proto::DocAuthor::User);
    super::design::plan::submit(&mut edited, author, &[], now, &mut effects)?;
    *run = edited;
    fx.extend(effects);
    let awaiting = |id: &str| {
        run.orch
            .gate_holds
            .iter()
            .any(|h| h.id == id && h.state == proto::HoldState::Awaiting)
    };
    let outcome = if promoted && awaiting("promotion") {
        "hold promotion awaits approval"
    } else if promoted {
        "it runs"
    } else if run.state == RunState::Running {
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
/// `route_pick::rung2_route` (decision 38's rung 2; milestone 9.5's lists and RL-1), and
/// a fresh session in the same worktree from the task's own start commit (carry M8a.11),
/// with decision 30's hand-over prompt; its old session, if alive, is killed first. The
/// hand-back context ends (carry T14-R2), so the fresh session's claim passes every
/// gate. A task that never started is dispatched again instead. A held task (M8a.6
/// ruling N5) waits for its dependencies, then has the run head handed back before the
/// fresh session, and keeps its own block until then (`holds::resume_held`, carries
/// T11-RR and M8a.14).
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
    if let Some(text) = rules::retry(run, task_id) {
        return reply(fx, id, Err(text));
    }
    let Some(i) = run.tasks.iter().position(|t| t.id() == task_id) else {
        return reply(fx, id, Err(rules::refused(rules::retry(run, task_id))));
    };
    let task = &run.tasks[i];
    let was = task.block.clone().map_or_else(String::new, |b| {
        let label = serde_json::to_value(b.reason)
            .ok()
            .and_then(|v| v.as_str().map(str::to_string))
            .unwrap_or_default();
        format!(" (it was blocked({label}): {})", b.text)
    });
    // Milestone 9.5 decision 21: a winner whose crown failed is crowned again.
    if super::race_end::retry_crown(run, i, now) {
        history(
            run,
            i,
            now,
            format!("retried by the user: its crown is sent again{was}"),
        );
        log(
            run,
            now,
            format!("{task_id} retried: its crown is sent again"),
        );
        let text = format!("task {task_id} retried: its race's winner is crowned again");
        return reply(fx, id, Ok(text));
    }
    // M9.9 second review, M-c: the user's retry lifts decision 25's cap.
    run.tasks[i].orch.rewrite_restarts = 0;
    let how = rung2(run, i, format!("the user retried it{was}"), now, fx);
    history(run, i, now, format!("retried by the user at rung 2{was}"));
    log(run, now, format!("{task_id} retried at rung 2"));
    reply(
        fx,
        id,
        Ok(format!("task {task_id} retried at rung 2: {how}")),
    );
}

/// Decision 42's re-entry at rung 2 of blocked task `i`, which `run retry` and
/// milestone 9 decision 25's rewrite of a mis-sized task share: `failures = 1`, the
/// counters cleared, the route escalated, and a fresh session (`why` is its hand-over
/// reason), or a dispatch again for a task that never started. Returns how it goes on.
pub(super) fn rung2(
    run: &mut Run,
    i: usize,
    why: String,
    now: u64,
    fx: &mut Vec<Effect>,
) -> &'static str {
    // Milestone 9.5 decision 26: while the test is being written, the test writer's
    // route escalates and the implementer's stays.
    let writer = super::pair::escalate_writer(run, i, now);
    let (route, step) = match writer {
        true => (run.tasks[i].route.clone(), None),
        false => crate::run::route_pick::rung2_route(run, i),
    };
    if let Some(text) = (!writer)
        .then(|| crate::run::route_pick::every_route_failed(run, i, &route))
        .flatten()
    {
        log(run, now, text);
    }
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
    if !writer {
        task.escalated_from = Some(std::mem::replace(&mut task.route, route));
    }
    // A research or review task's pick is its `list_pick` (its review records it,
    // ruling RL-4); a worker's escalation is recorded at its next launch.
    match super::schedule::is_reader_task(task) {
        true => task.list_pick = step.or(task.list_pick.take()),
        false => task.list_escalation = step,
    }
    // Ruling T15-C1: a new budget epoch; rung 4 counts from the fresh session.
    super::clock::new_epoch(task);
    // `kill_worker`'s `supersede` ended the hand-back context (`handed_back`,
    // `resolution`); its gates' pass goes too (carry T14-R2).
    task.gates_after_handback = false;
    // A research or review task has no worktree and never a start commit: it is
    // dispatched again, as that kind starts (a fresh research session, a fresh review
    // of its target), never a worker's fresh session (M9.9 review fixes).
    if task.start_commit.is_none() || super::schedule::is_reader_task(task) {
        set_state(task, TaskState::Queued, now);
        task.block = None;
        task.fresh_session = None;
        // Ruling T17a-1: a new dispatch decides the race again.
        task.race_decision = None;
        // Task 17a's re-review (b): a race that ended with both lanes out runs single.
        if let Some(race) = task.race.as_mut().filter(|r| r.winner.is_none()) {
            race.ended = true;
        }
        "it is dispatched again"
    } else {
        task.fresh_session = Some(FreshSession {
            reason: why,
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
    }
}
