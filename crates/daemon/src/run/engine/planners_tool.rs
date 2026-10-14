//! The `epic` tool's handling, split out of `planners.rs` (milestone 9.16 ruling C7,
//! move-only): a sub-planner's tool call, its confinement to its own round, the ops it
//! may not use, and `submit_epic`'s acceptance or rejection (decision 22). Pure (design
//! decision 2).

use proto::{PlanEdit, RunState, Runtime, TaskState, ToolCall};

use super::batch::{Applied, Refused, apply_batch, record_rejected};
use super::orch::{refuse, rejected, settle};
use super::planners::{EPIC_RECORDED, fail};
use super::requests::log;
use super::{Effect, ReplyId, gate_holds, history, kinds, wake};
use crate::run::edits_orch::planner_confinement;
use crate::run::model::Run;
use crate::run::orch::contract_rounds::ITERATE_BY_PLANNER;
use crate::run::orch::tools::{OrchCall, parse_call};
use crate::run::orch::{EditSource, PlannerPhase};
use crate::run::plan::PlanError;
use crate::run::validate::EditScope;

/// A sub-planner's tool (decisions 15, 22), after the run-state gate: the caller must
/// be its epic's live session; then `submit_epic`. Its reads are the driver's.
pub(super) fn tool(
    run: &mut Run,
    reply: ReplyId,
    call: &ToolCall,
    refusals: &[(Runtime, String)],
    now: u64,
    fx: &mut Vec<Effect>,
) {
    if run.state == RunState::Complete {
        let text = format!("run {} is {}", run.id, run.state.label());
        return refuse(fx, reply, text);
    }
    let epic = call.epic.clone().unwrap_or_default();
    let Some(k) = run.orch.epics.iter().position(|e| e.epic == epic) else {
        return refuse(fx, reply, format!("unknown epic {epic}"));
    };
    let record = &run.orch.epics[k];
    let latest = record.sessions.last();
    let window = latest.and_then(|s| s.window_id);
    if window == Some(call.window_id) && record.phase == PlannerPhase::Finished {
        let text = format!("submit_epic was already accepted for epic {epic}");
        return refuse(fx, reply, text);
    }
    // M9.8 review, ruling 3: only the latest session, live and not ended, may call; a
    // queued re-plan's epic has no such session yet.
    if !record.is_live_caller(call.window_id) {
        let text = format!(
            "this window is not the sub-planner of epic {epic} of run {}",
            run.id
        );
        return refuse(fx, reply, text);
    }
    match parse_call(call.role, &call.tool, &call.args) {
        Ok(OrchCall::SubmitEpic { edits, note }) => submit_epic(
            run,
            reply,
            (k, call.window_id),
            (&edits, note),
            refusals,
            now,
            fx,
        ),
        Ok(_) => refuse(
            fx,
            reply,
            format!("tool {} is not available yet", call.tool),
        ),
        Err(text) => refuse(fx, reply, text),
    }
}

/// M9.8 review, ruling 2, as the second review rules it: a sub-planner changes only
/// its current session's round. It may `amend_task`, `split_task` or `add_dep` (as
/// the dependent) a task of its epic only when the task sits in the `Drafting` round
/// its session's additions join, and `cancel_task` one only when it is neither released
/// nor started, since cancelling unapproved work only reduces it. Before the gate
/// nothing is approved, so any unstarted task of its epic may change. Another epic's
/// task is `planner_confinement`'s; the orchestrator's and the user's rights are
/// decision 19's.
fn outside_its_round(run: &Run, edits: &[PlanEdit], epic: &str) -> Vec<PlanError> {
    let round = gate_holds::session_round(run, epic);
    let may_change = |t: &crate::run::model::Task, cancel: bool| {
        let started = t.session > 0
            || !matches!(
                t.state,
                TaskState::Pending | TaskState::Queued | TaskState::Blocked
            );
        if started {
            false
        } else if cancel {
            !gate_holds::released_past_gate(run, t)
        } else {
            !gate_holds::past_gate(run) || (t.orch.gate_hold.is_some() && t.orch.gate_hold == round)
        }
    };
    edits
        .iter()
        .filter_map(|edit| match edit {
            PlanEdit::AmendTask { task_id, .. }
            | PlanEdit::SplitTask { task_id, .. }
            | PlanEdit::AddDep { task_id, .. } => Some((task_id, false)),
            PlanEdit::CancelTask { task_id } => Some((task_id, true)),
            _ => None,
        })
        .filter(|(id, cancel)| {
            run.task(id)
                .is_some_and(|t| t.spec.epic.as_deref() == Some(epic) && !may_change(t, *cancel))
        })
        .map(|(id, _)| {
            let text = format!("task {id}: a sub-planner changes only its own round's tasks");
            PlanError::new(Some(id), "", "2.epic", text)
        })
        .collect()
}

/// The edit ops a sub-planner may not use (decision 22, TT §12.1), each with its
/// refusal; milestone 9.3 decision 30's `iterate` in its own words.
fn forbidden(edit: &PlanEdit) -> Option<String> {
    let op = match edit {
        PlanEdit::Answer { .. } => "answer",
        PlanEdit::Pause => "pause",
        PlanEdit::Resume => "resume",
        PlanEdit::Finish => "finish",
        PlanEdit::Message { .. } => "message",
        PlanEdit::Refresh { .. } => "refresh",
        PlanEdit::Iterate { .. } => return Some(ITERATE_BY_PLANNER.to_string()),
        // Milestone 9.9 decision 11: the orchestrator's action ops.
        PlanEdit::Retry { .. } => "retry",
        PlanEdit::Override { .. } => "override",
        PlanEdit::ResumeRun { .. } => "resume_run",
        PlanEdit::ApproveHold { .. } => "approve_hold",
        PlanEdit::AcceptRed { .. } => "accept_red",
        _ => return None,
    };
    Some(format!("op {op} is not available to a sub-planner"))
}

/// Decision 22's `submit_epic` of epic `k`, from its live session in `window`: one
/// batch under the epic's area and decision 23's rules, confined to the epic's own
/// tasks, then held as the orchestrator's additions are. Accepted, the planner is
/// finished and its session retired; rejected, the error list, and the planner fails
/// at `max_rejections`.
fn submit_epic(
    run: &mut Run,
    reply: ReplyId,
    (k, window): (usize, u32),
    (edits, note): (&[PlanEdit], Option<String>),
    refusals: &[(Runtime, String)],
    now: u64,
    fx: &mut Vec<Effect>,
) {
    let epic = run.orch.epics[k].epic.clone();
    let source = EditSource::Planner { epic: epic.clone() };
    let refused = edits
        .iter()
        .find_map(forbidden)
        .or_else(|| kinds::engine_owned(run, edits));
    if let Some(text) = refused {
        record_rejected(run, edits, &source, text.clone(), now);
        return refuse(fx, reply, text);
    }
    let mut confined = planner_confinement(run, edits, &epic);
    confined.extend(outside_its_round(run, edits, &epic));
    if !confined.is_empty() {
        let first = confined
            .first()
            .map(ToString::to_string)
            .unwrap_or_default();
        record_rejected(run, edits, &source, first, now);
        return reject(run, reply, (k, window), &confined, now, fx);
    }
    let mut edited = run.clone();
    let mut effects = Vec::new();
    let scope = EditScope::Area {
        globs: run.orch.epics[k].area.clone(),
    };
    let batch = (edits, &scope, refusals);
    let added = match apply_batch(&mut edited, batch, &source, now, &mut effects) {
        Ok(Applied { added, .. }) => added,
        Err(Refused::Text(text)) => {
            record_rejected(run, edits, &source, text.clone(), now);
            return refuse(fx, reply, text);
        }
        Err(Refused::Plan(errors)) => {
            let first = errors.first().map(ToString::to_string).unwrap_or_default();
            record_rejected(run, edits, &source, first, now);
            return reject(run, reply, (k, window), &errors, now, fx);
        }
    };
    gate_holds::assign(&mut edited, edits, &added, now);
    if let Some(text) = super::protected_grants::unheld(&edited, run, &added) {
        record_rejected(run, edits, &source, text.clone(), now);
        return refuse(fx, reply, text);
    }
    let record = &mut edited.orch.epics[k];
    record.phase = PlannerPhase::Finished;
    record.ended_at = Some(now);
    record.edits_accepted += edits.len() as u32;
    if note.is_some() {
        record.note = note;
    }
    if let Some(session) = record.sessions.last_mut() {
        session.ended_at = Some(now);
    }
    history::planner_accepted(&mut edited, &epic);
    gate_holds::submit_epic_round(&mut edited, &epic, now);
    let n = edited
        .tasks
        .iter()
        .filter(|t| t.spec.epic.as_deref() == Some(epic.as_str()))
        .filter(|t| t.state != proto::TaskState::Cancelled)
        .count();
    let plural = if n == 1 { "" } else { "s" };
    log(
        &mut edited,
        now,
        format!("sub-planner {epic} submitted its epic"),
    );
    wake::note(
        &mut edited,
        format!("sub-planner {epic} finished with {n} task{plural}"),
    );
    *run = edited;
    fx.extend(effects);
    settle(run, now, fx);
    fx.push(Effect::Reply {
        reply,
        result: Ok(EPIC_RECORDED.to_string()),
    });
    fx.push(Effect::PlannerAccepted { window_id: window });
}

/// A rejected `submit_epic` (decision 22): counted, its first error kept, and the
/// planner failed and stopped once its session reached `max_rejections`.
fn reject(
    run: &mut Run,
    reply: ReplyId,
    (k, window): (usize, u32),
    errors: &[crate::run::plan::PlanError],
    now: u64,
    fx: &mut Vec<Effect>,
) {
    let max = run.limits.orch.planners.max_rejections;
    let record = &mut run.orch.epics[k];
    record.edits_rejected += 1;
    record.last_rejection = errors.first().map(|e| e.message.clone());
    let rejections = record.sessions.last_mut().map_or(0, |s| {
        s.rejections += 1;
        s.rejections
    });
    if rejections >= max {
        let reason = format!("the sub-planner's epic was rejected {rejections} times");
        fail(run, k, reason.clone(), now);
        fx.push(Effect::StopPlanner {
            window_id: window,
            reason,
        });
    }
    fx.push(Effect::Reply {
        reply,
        result: Err(rejected(errors)),
    });
}
