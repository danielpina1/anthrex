//! Milestone 9.9 decisions 10 to 14 (OFA §4.2): the orchestrator's `retry`, `override` and
//! `approve_hold`. Each runs the user path's core with `Actor::Orchestrator`: the user's
//! preconditions and refusal texts, then the run log line, the handled list and the
//! plan-edit log (`handled::record`, `edit_log::record`). An op is alone in its call.
//! `resume_run` and `accept_red` join in M9.9.3. Pure (design decision 2).

use proto::PlanEdit;

use super::actor::Actor;
use super::batch::record_rejected;
use super::orch::refuse;
use super::{Effect, ReplyId, gate_holds, gates, requests};
use crate::run::edit_log::{self, EditOutcome};
use crate::run::model::Run;
use crate::run::orch::contract::ACTION_ALONE;
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
        // M9.9.3.
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
            let result = requests::retry_on(run, task_id, Actor::Orchestrator, now, fx);
            if result.is_ok() {
                handled::record(run, now, ("retry", "retried", task_id), reason);
            }
            result
        }
        PlanEdit::Override { task_id, .. } => {
            let call = (task_id.as_str(), reason);
            match gates::override_on(run, reply, call, Actor::Orchestrator, now, fx) {
                Some(result) => result,
                // The reply, the handled record and the log entry wait for the count.
                None => return true,
            }
        }
        PlanEdit::ApproveHold { hold, .. } => {
            let result = gate_holds::decide_on(run, hold, true, Actor::Orchestrator, now, fx);
            if result.is_ok() {
                let args = ("approve_hold", "approved hold", hold.as_str());
                handled::record(run, now, args, reason);
            }
            result
        }
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
