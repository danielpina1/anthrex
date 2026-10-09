//! Milestone 9.9 decisions 10 to 14 (OFA §4.2): the orchestrator's `retry`, `override` and
//! `approve_hold`. Each runs the user path's core with `Actor::Orchestrator`: the user's
//! preconditions and refusal texts, then the run log line, the handled list and the
//! plan-edit log (`handled::record`, `edit_log::record`). An op is alone in its call.
//! `resume_run` and `accept_red` (decision 6) join them, and [`admitted`] opens a halted
//! run's gate to the lone `resume_run`. Pure (design decision 2).

use proto::{PlanEdit, RunState, ToolCall};

use super::actor::Actor;
use super::batch::record_rejected;
use super::orch::refuse;
use super::{Effect, ReplyId, delivery, full, gate_holds, gates, requests, restore, user_only};
use crate::run::edit_log::{self, EditOutcome};
use crate::run::engine::actions::rules;
use crate::run::model::Run;
use crate::run::orch::contract::ACTION_ALONE;
use crate::run::orch::tools::{OrchCall, parse_call};
use crate::run::orch::{EditSource, handled};

/// The longest `reason`, in characters (decision 10).
pub const REASON_MAX_CHARS: usize = 500;

/// Whether `edit` is one of the orchestrator's five ops.
fn is_op(edit: &PlanEdit) -> bool {
    matches!(
        edit,
        PlanEdit::Retry { .. }
            | PlanEdit::Override { .. }
            | PlanEdit::ResumeRun { .. }
            | PlanEdit::ApproveHold { .. }
            | PlanEdit::AcceptRed { .. }
    )
}

/// `edit_plan`'s hook, after the complete-run check: `true` when the call held an op and
/// is answered here. `other` is whether the call also carries a summary or responses.
pub(super) fn intercept(
    run: &mut Run,
    reply: ReplyId,
    (edits, submit, other): (&[PlanEdit], bool, bool),
    (now, base): (u64, &mut Option<Run>),
    fx: &mut Vec<Effect>,
) -> bool {
    if !edits.iter().any(is_op) {
        return false;
    }
    let source = EditSource::Orchestrator;
    // Decision 11: alone in its call.
    if edits.len() != 1 || submit || other {
        record_rejected(run, edits, &source, ACTION_ALONE.into(), now);
        refuse(fx, reply, ACTION_ALONE);
        return true;
    }
    let (op, reason) = match &edits[0] {
        PlanEdit::Retry { reason, .. } => ("retry", reason),
        PlanEdit::Override { reason, .. } => ("override", reason),
        PlanEdit::ApproveHold { reason, .. } => ("approve_hold", reason),
        PlanEdit::ResumeRun { reason, .. } => ("resume_run", reason),
        PlanEdit::AcceptRed { reason, .. } => ("accept_red", reason),
        _ => return false,
    };
    let reason = reason.trim();
    let refusal = match reason.chars().count() {
        0 => Some(format!("{op} needs a reason the user can read")),
        n if n > REASON_MAX_CHARS => Some(format!("reason: at most {REASON_MAX_CHARS} characters")),
        _ => None,
    };
    if let Some(text) = refusal {
        record_rejected(run, edits, &source, text.clone(), now);
        refuse(fx, reply, text);
        return true;
    }
    let result = match &edits[0] {
        PlanEdit::Retry { task_id, .. } => {
            let result = match user_only_task(run, task_id, rules::retry(run, task_id)) {
                Some(refusal) => Err(refusal),
                None => requests::retry_on(run, task_id, Actor::Orchestrator, now, fx),
            };
            if result.is_ok() {
                handled::record(run, now, ("retry", "retried", task_id), reason);
            }
            result
        }
        PlanEdit::Override { task_id, .. } => {
            let call = (task_id.as_str(), reason);
            let rule = rules::override_task(run, task_id);
            if let Some(refusal) = user_only_task(run, task_id, rule) {
                record_rejected(run, edits, &source, refusal.clone(), now);
                refuse(fx, reply, refusal);
                return true;
            }
            match gates::override_on(run, reply, call, Actor::Orchestrator, now, fx) {
                Some(result) => result,
                // The reply, the handled record and the log entry wait for the count.
                None => return true,
            }
        }
        PlanEdit::ApproveHold { hold, .. } => {
            // Final review I-2: a promotion round is the user's plan approval (OFA §4.2).
            let result = if gate_holds::promotion(run, hold) {
                Err(format!(
                    "hold {hold} approves a promoted run's plan; only the user can"
                ))
            } else {
                gate_holds::decide_on(run, hold, true, Actor::Orchestrator, now, fx)
            };
            if result.is_ok() {
                let args = ("approve_hold", "approved hold", hold.as_str());
                handled::record(run, now, args, reason);
            }
            result
        }
        PlanEdit::ResumeRun { stage, .. } => resume_run(run, *stage, reason, now, fx),
        PlanEdit::AcceptRed { stage, .. } => accept_red(run, *stage, reason, now),
        _ => return false,
    };
    match result {
        Ok(text) => {
            edit_log::record(run, edits, now, &source, EditOutcome::accepted());
            // The call's own change is the quiet base; the step's pass settles after it.
            *base = Some(run.clone());
            fx.push(Effect::Reply {
                reply,
                result: Ok(text),
            });
        }
        Err(text) => {
            record_rejected(run, edits, &source, text.clone(), now);
            refuse(fx, reply, text);
        }
    }
    true
}

/// Decision 10: the user path's own refusal first, then a block only the user can fix.
fn user_only_task(run: &Run, task_id: &str, rule: Option<String>) -> Option<String> {
    if rule.is_some() {
        return rule;
    }
    let block = run
        .tasks
        .iter()
        .find(|t| t.id() == task_id)?
        .block
        .as_ref()?;
    block
        .user_only
        .then(|| user_only::refusal(&format!("task {task_id}"), &block.text))
}

/// `resume_run` (decision 6): without `stage`, `run resume` as the user's, never a
/// rebaseline; with one, only that held stage is released.
fn resume_run(
    run: &mut Run,
    stage: Option<u16>,
    reason: &str,
    now: u64,
    fx: &mut Vec<Effect>,
) -> Result<String, String> {
    let (text, target, line) = match stage {
        None => {
            if let Some(refusal) = rules::resume(run, false) {
                return Err(refusal);
            }
            if user_only::halt(run) {
                let reason = run.halted_reason.clone().unwrap_or_default();
                return Err(user_only::refusal(&format!("run {}", run.id), &reason));
            }
            let text = restore::resume_on(run, None, now, fx)?;
            (text, "run".to_string(), "resumed the run".to_string())
        }
        Some(n) => {
            if let Some(refusal) = rules::resume_stage(run, n) {
                return Err(refusal);
            }
            let held = (usize::from(n).checked_sub(1))
                .and_then(|i| run.delivery.stages.get(i))
                .and_then(|s| s.held.as_deref());
            if let Some(held) = held.filter(|h| user_only::marked(h)) {
                return Err(user_only::refusal(&format!("stage {n}"), held));
            }
            // As the user's `run resume`: every hold on the stage, tier 3 and push.
            full::retry_stage(run, n, now);
            delivery::release_stage(run, n, now);
            let text = format!("run {}: stage {n} released", run.id);
            (text, format!("stage {n}"), format!("resumed stage {n}"))
        }
    };
    handled::record_line(run, now, ("resume_run", &target, &line), reason);
    Ok(text)
}

/// `accept_red` (decision 6): the stage's red tier 3 on its head passes.
fn accept_red(run: &mut Run, n: u16, reason: &str, now: u64) -> Result<String, String> {
    if let Some(refusal) = rules::accept_red(run, n) {
        return Err(refusal);
    }
    let sha = full::accept_red(run, n, now).ok_or(rules::DRIFT)?;
    let target = format!("stage {n}");
    let line = format!("accepted the red tier 3 of stage {n}");
    handled::record_line(run, now, ("accept_red", &target, &line), reason);
    Ok(format!(
        "stage {n}'s red tier 3 on {sha} is accepted; completion and delivery go on"
    ))
}

/// The gate for a halted or paused run (decision 6): a halted run takes `ask_user` and
/// an `edit_plan` of exactly one `resume_run`; a paused one takes `ask_user` only.
/// Anything else keeps the state's own gate (`orch.rs`). Final review I-3: the call is
/// admitted by what it parses to, so a `submit` of false or an empty `responses` (the
/// schema's defaults) does not shut it out.
pub(super) fn admitted(state: RunState, call: &ToolCall) -> bool {
    if call.tool == "ask_user" {
        return true;
    }
    if state != RunState::Halted || call.tool != "edit_plan" {
        return false;
    }
    let Ok(OrchCall::EditPlan {
        edits,
        submit,
        summary,
        iterate,
        responses,
    }) = parse_call(call.role, &call.tool, &call.args)
    else {
        return false;
    };
    matches!(&edits[..], [PlanEdit::ResumeRun { .. }])
        && !submit
        && summary.is_none()
        && iterate.is_none()
        && responses.is_empty()
}

/// The orchestrator's refusal on a halted run it may not act on (final review I-3): it
/// names what the run does take.
pub(super) fn halted_refusal(run_id: &str) -> String {
    format!("run {run_id} is halted; a lone resume_run (or ask_user) is all it takes")
}

/// An override that waited for its commit count landed (or failed): the orchestrator's
/// plan-edit log entry (decision 12), written when the reply is known.
pub(super) fn override_landed(
    run: &mut Run,
    (task_id, reason): (&str, &str),
    result: &Result<String, String>,
    now: u64,
) {
    let edits = [PlanEdit::Override {
        task_id: task_id.into(),
        reason: reason.into(),
    }];
    let source = EditSource::Orchestrator;
    match result {
        Ok(_) => edit_log::record(run, &edits, now, &source, EditOutcome::accepted()),
        Err(text) => record_rejected(run, &edits, &source, text.clone(), now),
    }
}
