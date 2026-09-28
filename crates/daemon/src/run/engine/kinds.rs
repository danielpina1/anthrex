//! Milestone 9 decisions 35–38, engine side: research and review tasks, and the
//! per-epic integration review. Pure (design decision 2).
//!
//! A **research** task runs a read-only scout session (`run::orch::launch::
//! research_spec`) in a reader slot, with no worktree and no branch: its round's role is
//! `Scout`, its mailbox `<task>.research`. Its `submit_scout_report` ends it `reported`
//! with the report kept on the task; a turn without one gets `SCOUT_NUDGE` once and the
//! second blocks it. A **review** task resolves its target (`OpKind::ResolveTarget`),
//! then runs M8a's review round (`review.rs`) on that range: its first verdict, whatever
//! it is, ends it `reported`, and nothing is merged or sent back. An **integration
//! review** is a review task the engine adds itself once an epic's work is merged
//! (decision 37). Research and review tasks never merge anything, integration reviews
//! never approve anything but their epic's integration state, and completion is
//! decision 38's (`may_complete`).

use proto::{
    AgentRole, BlockReason, IntegrationState, RunState, Runtime, ScoutKind, Severity, Size,
    TaskKind, TaskState, ToolCall, Verdict,
};

use super::clock::stall_due;
use super::dispatch::{block, history, new_round, window_limit_reached};
pub(super) use super::integration::{engine_owned, integration_pass, may_complete, record_merge};
use super::requests::log;
use super::schedule::{
    dispatch_order, hub_holds_slot, is_reader_task, op_in_flight, readers_busy, size_check_pending,
};
use super::signals::end_round;
use super::{Effect, OpId, OpKind, OpResult, ReplyId, emit_op, gate_holds, next_op, outbox, wake};
use crate::headless::FailureKind;
use crate::run::contract::sha7;
use crate::run::model::{ReviewLevel, Run, Task};
use crate::run::orch::contract::{integration_review_prompt, research_prompt, review_task_prompt};
use crate::run::orch::launch::research_spec;
use crate::run::phases::set_state;
use crate::run::role_launch::{jitter_ms, session_uuid_of};
use crate::scout::contract::SCOUT_NUDGE;
use crate::scout::service::REPORT_ACCEPTED;

/// Decision 35: the second turn with no report.
pub const NO_REPORT: &str = "the research task ended two turns without a report";

/// A research session whose process stopped mid-turn is resumed with this (decision
/// 35's "deaths follow M8a's worker rules").
pub const RESEARCH_RESUME_AFTER_EXIT: &str = "[anthrex] Your session's process stopped in the middle of a turn and has been resumed. Finish your research and call submit_scout_report, exactly once.";

/// After a daemon restart, a research session that owes its report is resumed with this.
pub const RESEARCH_RESUME: &str =
    "[anthrex] The daemon restarted. Finish your research and call submit_scout_report.";

/// The outbox address of research task `task`'s session (task ids cannot contain `.`).
pub(super) fn mailbox(task: &str) -> String {
    format!("{task}.research")
}

/// The task whose research session `address` is, if it is a research mailbox.
pub(super) fn mailbox_task(address: &str) -> Option<&str> {
    address.strip_suffix(".research")
}

/// Task `i`'s current research round, if it has one.
pub(super) fn research_round(run: &Run, i: usize) -> Option<usize> {
    run.tasks[i]
        .rounds
        .iter()
        .rposition(|r| r.role == AgentRole::Scout)
}

pub(super) fn is_integration(task: &Task) -> bool {
    task.orch.integration_of.is_some()
}

/// Decision 31's reader-slot order: integration reviews go with the reviewers
/// (`integration`), research and review tasks after the sub-planners and run scouts.
/// Runs only while the run runs, as every task dispatch does, and not beside a hub
/// task holding its writer slot.
pub(super) fn dispatch(run: &mut Run, now: u64, integration: bool, fx: &mut Vec<Effect>) {
    if run.state != RunState::Running || hub_holds_slot(run) {
        return;
    }
    for i in dispatch_order(run) {
        if readers_busy(run) >= usize::from(run.limits.max_readers) {
            break;
        }
        let task = &run.tasks[i];
        if !is_reader_task(task) || is_integration(task) != integration {
            continue;
        }
        // A review task whose resolved range was lost (a restart) resolves it again.
        let unresolved = task.state == TaskState::Review
            && task.orch.review_range.is_none()
            && task.gate_op.is_none()
            && !op_in_flight(run, task.id(), |_| true);
        let runnable = task.state == TaskState::Queued
            && !size_check_pending(task)
            && gate_holds::released(run, task);
        if !runnable && !unresolved {
            continue;
        }
        match task.spec.kind {
            TaskKind::Research => launch_research(run, i, now, fx),
            _ => resolve(run, i, now, fx),
        }
    }
}

/// Decision 35: research session `n + 1` of task `i`, in the user's checkout.
fn launch_research(run: &mut Run, i: usize, now: u64, fx: &mut Vec<Effect>) {
    if window_limit_reached(run, i, now) {
        return;
    }
    let op = next_op(run);
    run.tasks[i].session += 1;
    let task = &run.tasks[i];
    let spec = research_spec(run, task);
    let first_turn = research_prompt(run, task);
    let name = format!("{}/{}.s{}", run.short(), task.id(), task.session);
    let uuid = (task.route.runtime == Runtime::Claude).then(|| session_uuid_of(run, op));
    let jitter = jitter_ms(&run.id, task.id(), task.session);
    let round = new_round(
        AgentRole::Scout,
        task.session,
        task.route.clone(),
        op,
        uuid.clone(),
        now,
    );
    let (id, session) = (task.id().to_string(), task.session);
    run.tasks[i].rounds.push(round);
    run.windows_created += 1;
    set_state(&mut run.tasks[i], TaskState::Working, now);
    history(run, i, now, format!("research session {session} starting"));
    let kind = OpKind::CreateWindow {
        name,
        spec: Box::new(spec),
        session_uuid: uuid,
        first_turn,
        project: run.project.clone(),
        worktree: run.root.clone(),
        jitter_ms: jitter,
        extract: None,
    };
    emit_op(run, op, Some(&id), kind, fx);
}

/// Decision 36: a review task takes its reader slot and resolves its target first.
fn resolve(run: &mut Run, i: usize, now: u64, fx: &mut Vec<Effect>) {
    let op = next_op(run);
    let task = &run.tasks[i];
    let kind = OpKind::ResolveTarget {
        root: run.root.clone(),
        target: task.spec.review_target.clone().unwrap_or_default(),
        base_branch: run.base_branch.clone(),
    };
    let id = task.id().to_string();
    set_state(&mut run.tasks[i], TaskState::Review, now);
    run.tasks[i].gate_op = Some(op);
    history(run, i, now, "dispatched: resolving its review target");
    emit_op(run, op, Some(&id), kind, fx);
}

/// `ResolveTarget`'s result: the range is kept and its review prepared, or the task is
/// `blocked(environment)` with git's text. A launch the restart lost resolves again.
pub(super) fn target_done(
    run: &mut Run,
    i: usize,
    op: OpId,
    result: OpResult,
    now: u64,
    fx: &mut Vec<Effect>,
) {
    if run.tasks[i].gate_op != Some(op) {
        return;
    }
    run.tasks[i].gate_op = None;
    if run.tasks[i].state != TaskState::Review {
        return;
    }
    match result {
        OpResult::Target { base, head } => {
            let text = format!("review target resolved to {}..{}", sha7(&base), sha7(&head));
            run.tasks[i].orch.review_range = Some((base.clone(), head.clone()));
            history(run, i, now, text);
            prepare(run, i, (base, head), fx);
        }
        OpResult::Failed { message } => {
            let target = run.tasks[i].spec.review_target.clone().unwrap_or_default();
            let text = format!("review target {target} does not resolve: {message}");
            block(run, i, BlockReason::Environment, text, now);
        }
        _ => {
            set_state(&mut run.tasks[i], TaskState::Queued, now);
            history(run, i, now, "its review target is resolved again");
        }
    }
}

/// M8a's `PrepareReview` of a review task's resolved range; the review round follows
/// from its result (`review::review_ready`).
fn prepare(run: &mut Run, i: usize, (base, head): (String, String), fx: &mut Vec<Effect>) {
    let op = next_op(run);
    let id = run.tasks[i].id().to_string();
    let kind = OpKind::PrepareReview {
        root: run.root.clone(),
        head_ref: head,
        base_ref: base,
        path: run.review_path(&id),
    };
    run.tasks[i].gate_op = Some(op);
    emit_op(run, op, Some(&id), kind, fx);
}

/// A review task's `PrepareReview` refs (`review::dispatch_reviewers`): its resolved
/// `(head, base)`, or `None` until it is resolved.
pub(super) fn review_refs(task: &Task) -> Option<(String, String)> {
    task.orch
        .review_range
        .clone()
        .map(|(base, head)| (head, base))
}

/// A review task's reviewer (decisions 36, 37): the author and level it is recorded
/// with and the route it runs on. The task's own route, which policy filled as for any
/// task and the engine set for an integration review; `small` for an S task, `medium`
/// for any other, `frontier` for an integration review.
pub(super) fn review_route(task: &Task) -> (proto::Route, ReviewLevel, proto::Route) {
    let level = if is_integration(task) {
        ReviewLevel::Frontier
    } else if task.size == Size::S {
        ReviewLevel::Small
    } else {
        ReviewLevel::Medium
    };
    (task.route.clone(), level, task.route.clone())
}

/// A review task's first turn: `integration_review_prompt` for an integration review,
/// else `review_task_prompt`.
pub(super) fn review_first_turn(
    run: &Run,
    i: usize,
    (base, head, patch): (&str, &str, &str),
) -> String {
    let task = &run.tasks[i];
    let Some(epic) = task.orch.integration_of.as_deref() else {
        return review_task_prompt(run, task, base, head, patch);
    };
    let Some(record) = run.orch.epics.iter().find(|e| e.epic == epic) else {
        return review_task_prompt(run, task, base, head, patch);
    };
    // Its round: how many of the epic's integration reviews come up to it.
    let round = run.tasks[..=i]
        .iter()
        .filter(|t| t.orch.integration_of.as_deref() == Some(epic))
        .count() as u32;
    integration_review_prompt(run, record, round, base, head)
}

/// Decision 36: a review task's first verdict was recorded (`review::submit`): it is
/// `reported`, whatever the verdict. An integration review sets its epic's state and
/// tells the orchestrator (decisions 37, 39).
pub(super) fn reviewed(run: &mut Run, i: usize, now: u64) {
    let task = &mut run.tasks[i];
    let Some(review) = task.reviews.iter().rfind(|r| r.verdict.is_some()).cloned() else {
        return;
    };
    set_state(task, TaskState::Reported, now);
    task.block = None;
    let verdict = if review.verdict == Some(Verdict::Approve) {
        "approve"
    } else {
        "changes"
    };
    history(
        run,
        i,
        now,
        format!("reported: its review's verdict is {verdict}"),
    );
    let Some(epic) = run.tasks[i].orch.integration_of.clone() else {
        return;
    };
    let count = |s: Severity| review.findings.iter().filter(|f| f.severity == s).count();
    let (critical, important) = (count(Severity::Critical), count(Severity::Important));
    let blocking = critical + important > 0;
    if let Some(record) = run.orch.epics.iter_mut().find(|e| e.epic == epic) {
        record.integration_state = if blocking {
            IntegrationState::Changes
        } else {
            IntegrationState::Approved
        };
    }
    let outcome = if blocking {
        format!("changes ({critical} critical, {important} important)")
    } else {
        "approve".to_string()
    };
    let text = format!("integration review of epic {epic}: {outcome}");
    log(run, now, text.clone());
    wake::note(run, text);
}

fn reply(fx: &mut Vec<Effect>, reply: ReplyId, result: Result<String, String>) {
    fx.push(Effect::Reply { reply, result });
}

/// `submit_scout_report` from a research task's session (decision 35), in order: the
/// caller is the task's current, live research round; the report validates as an area
/// scout's (M8b's `scout::report::validate`). The report is kept, the task `reported`
/// and the session retired.
pub(super) fn submit_research(
    run: &mut Run,
    id: ReplyId,
    call: &ToolCall,
    now: u64,
    fx: &mut Vec<Effect>,
) {
    let task_id = call.task_id.clone().unwrap_or_default();
    let not_session = format!("this window is not the research session of task {task_id}");
    let found = run.tasks.iter().position(|t| t.id() == task_id);
    let current = found.and_then(|i| {
        let task = &run.tasks[i];
        let r = research_round(run, i)?;
        let round = &task.rounds[r];
        let live = !round.retiring && !round.ended && round.window_id == Some(call.window_id);
        (call.role == AgentRole::Scout && task.state == TaskState::Working && live)
            .then_some((i, r))
    });
    let Some((i, r)) = current else {
        return reply(fx, id, Err(not_session));
    };
    let report = match crate::scout::report::validate(&call.args, ScoutKind::Area) {
        Ok(report) => report,
        Err(text) => return reply(fx, id, Err(text)),
    };
    let task = &mut run.tasks[i];
    task.orch.research = Some(report);
    set_state(task, TaskState::Reported, now);
    task.block = None;
    let round = &mut task.rounds[r];
    round.retiring = true;
    round.resume_op = None;
    if let Some(window_id) = round.window_id {
        fx.push(Effect::RetireWindow { window_id });
    }
    drop_mail(run, i);
    history(run, i, now, "reported: its research report was recorded");
    reply(fx, id, Ok(REPORT_ACCEPTED.to_string()));
}

fn drop_mail(run: &mut Run, i: usize) {
    let address = mailbox(run.tasks[i].id());
    run.outbox.retain(|m| m.task_id != address);
}

/// Stops task `i`'s live research session: the engine's kill, and its mail dropped (a
/// cancel, a block).
pub(super) fn stop_research(run: &mut Run, i: usize, fx: &mut Vec<Effect>) {
    for round in run.tasks[i]
        .rounds
        .iter_mut()
        .filter(|r| r.role == AgentRole::Scout && !r.retiring)
    {
        round.retiring = true;
        round.resume_op = None;
        if round.ended {
            continue;
        }
        if round.route.runtime == Runtime::Codex && !round.turn_open && round.pid.is_none() {
            round.ended = true;
            continue;
        }
        if let Some(window_id) = round.window_id {
            fx.push(Effect::KillWindow { window_id });
        }
    }
    drop_mail(run, i);
}

/// The session is stopped and the task `blocked(environment)` with `text`.
fn give_up(run: &mut Run, i: usize, text: String, now: u64, fx: &mut Vec<Effect>) {
    stop_research(run, i, fx);
    block(run, i, BlockReason::Environment, text, now);
}

/// Whether round `r` of task `i` is its research session still owing its report.
fn owes_report(run: &Run, i: usize, r: usize) -> bool {
    run.tasks[i].state == TaskState::Working
        && research_round(run, i) == Some(r)
        && !run.tasks[i].rounds[r].retiring
}

/// A research session's turn ended (decision 35): with no report, `SCOUT_NUDGE` is one
/// more turn and a second such turn blocks the task. A failed turn is such a turn,
/// except an authentication, billing or sandbox failure, which blocks at once.
pub(super) fn turn_ended(
    run: &mut Run,
    i: usize,
    r: usize,
    outcome: crate::headless::TurnOutcome,
    now: u64,
    fx: &mut Vec<Effect>,
) {
    use crate::headless::TurnOutcome;
    if !owes_report(run, i, r) {
        return;
    }
    match outcome {
        TurnOutcome::Interrupted => return,
        TurnOutcome::Failed {
            error,
            kind:
                FailureKind::Authentication | FailureKind::Billing | FailureKind::SandboxUnavailable,
        } => {
            return give_up(run, i, error, now, fx);
        }
        _ => {}
    }
    let round = &mut run.tasks[i].rounds[r];
    if round.review_nudged {
        return give_up(run, i, NO_REPORT.to_string(), now, fx);
    }
    round.review_nudged = true;
    let address = mailbox(run.tasks[i].id());
    outbox::queue_to(run, &address, r, SCOUT_NUDGE.to_string(), now);
}

/// A research session's process exited without the engine killing it: between turns
/// the round is marked ended (a delivery resumes it); mid-turn, the first death in the
/// round resumes the session and the second blocks the task.
pub(super) fn exited(
    run: &mut Run,
    i: usize,
    r: usize,
    killed: bool,
    now: u64,
    fx: &mut Vec<Effect>,
) {
    let owes = owes_report(run, i, r);
    let round = &mut run.tasks[i].rounds[r];
    if killed || !owes || !round.turn_open {
        return end_round(round, now);
    }
    round.deaths = round.deaths.saturating_add(1);
    let resumable = round.window_id.zip(round.session_id.clone());
    if round.deaths >= 2 || resumable.is_none() {
        end_round(round, now);
        let text = "the research session's process exited twice in one turn".to_string();
        return give_up(run, i, text, now, fx);
    }
    let Some((window_id, session_id)) = resumable else {
        return;
    };
    round.last_event = now;
    let (id, session) = (run.tasks[i].id().to_string(), run.tasks[i].session);
    let kind = OpKind::ResumeSession {
        window_id,
        session_id,
        message: RESEARCH_RESUME_AFTER_EXIT.to_string(),
        jitter_ms: jitter_ms(&run.id, &id, session),
    };
    let op = next_op(run);
    run.tasks[i].rounds[r].resume_op = Some(op);
    emit_op(run, op, Some(&id), kind, fx);
    history(
        run,
        i,
        now,
        "its research session exited mid-turn; resuming it",
    );
}

/// A research session's `ResumeSession` result: resumed, or the task blocked.
pub(super) fn resumed(
    run: &mut Run,
    i: usize,
    r: usize,
    result: OpResult,
    now: u64,
    fx: &mut Vec<Effect>,
) {
    let round = &mut run.tasks[i].rounds[r];
    round.resume_op = None;
    let carried = std::mem::take(&mut round.carried);
    run.outbox.retain(|m| !carried.contains(&m.id));
    let error = match result {
        OpResult::Resumed => return run.tasks[i].rounds[r].delivery_failures = 0,
        OpResult::ResumeFailed { error } => error,
        OpResult::Failed { message } => message,
        _ => return,
    };
    end_round(&mut run.tasks[i].rounds[r], now);
    if owes_report(run, i, r) {
        let text = format!("the research session could not be resumed: {error}");
        give_up(run, i, text, now, fx);
    }
}

/// Every scheduler pass of a running run: an open research turn with no stream event
/// for `stall_after_secs` blocks its task; a session a restart ended while it owed its
/// report is resumed with [`RESEARCH_RESUME`], or blocks the task when it has no
/// session to resume.
pub(super) fn watch(run: &mut Run, now: u64, fx: &mut Vec<Effect>) {
    let stall_after = run.limits.stall_after_secs;
    for i in 0..run.tasks.len() {
        if run.tasks[i].spec.kind != TaskKind::Research {
            continue;
        }
        let Some(r) = research_round(run, i) else {
            continue;
        };
        if !owes_report(run, i, r) {
            continue;
        }
        let round = &run.tasks[i].rounds[r];
        let address = mailbox(run.tasks[i].id());
        let waiting = run
            .outbox
            .iter()
            .any(|m| m.task_id == address && m.delivered_at.is_none());
        let busy = round.resume_op.is_some()
            || op_in_flight(run, run.tasks[i].id(), |k| {
                matches!(
                    k,
                    OpKind::CreateWindow { .. } | OpKind::ResumeSession { .. }
                )
            });
        if round.ended && !waiting && !busy && round.relaunch.is_none() {
            if round.session_id.is_none() {
                let text = "the research session ended before it had an id to resume";
                give_up(run, i, text.to_string(), now, fx);
            } else {
                outbox::queue_to(run, &address, r, RESEARCH_RESUME.to_string(), now);
            }
            continue;
        }
        if !round.ended && round.turn_open && now >= stall_due(round, stall_after) {
            let text = format!("no stream event for {} minutes", stall_after / 60);
            give_up(
                run,
                i,
                format!("the research session stalled: {text}"),
                now,
                fx,
            );
        }
    }
}
