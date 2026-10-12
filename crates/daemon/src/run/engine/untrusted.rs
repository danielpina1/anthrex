//! Milestone 9.13 decision 21 (NA §2 D4): once the session has read text from outside
//! the run (`RunOrch.untrusted`), the orchestrator's `override` and `accept_red` become
//! a question to the user, and its `approve_hold` is refused. A refused op is refused,
//! never asked: the caller runs the user path's rule and the user-only refusal first.
//! The user's answer runs the op as the user's own, or drops it. Pure.

use proto::PlanEdit;

use super::actor::Actor;
use super::requests::log;
use super::{Effect, ReplyId, full, gates, wake};
use crate::run::engine::actions::rules;
use crate::run::model::Run;

/// The longest question, in characters.
const QUESTION_MAX: usize = 500;

/// The orchestrator's reply when its op became the user's question.
pub(super) const ASKED: &str = "asked the user to decide: this session read text from outside the run, so override and accept_red are theirs now; their answer arrives as a message";

/// The tail of a tainted `approve_hold`'s refusal, after `hold {id} `.
pub(super) const HOLD_REFUSAL: &str =
    "waits for the user: this session read text from outside the run";

/// The answer's two options: `Some(0)` runs the op, `Some(1)` declines it.
const OPTIONS: [&str; 2] = ["do it", "leave it"];

/// `approve_hold`'s refusal while the run is tainted; `None` while it is not.
pub(super) fn hold_refusal(run: &Run, hold: &str) -> Option<String> {
    run.orch
        .untrusted
        .is_some()
        .then(|| format!("hold {hold} {HOLD_REFUSAL}"))
}

/// `{op}` of a proposal: `override {task}` or `accept_red stage {n}`.
fn op_text(edit: &PlanEdit) -> Option<String> {
    match edit {
        PlanEdit::Override { task_id, .. } => Some(format!("override {task_id}")),
        PlanEdit::AcceptRed { stage, .. } => Some(format!("accept_red stage {stage}")),
        _ => None,
    }
}

/// The question for `edit`, an `override` or `accept_red` the user path's rules let
/// through, while the run is tainted: `(question, options, context)`. `None` when the
/// run is not tainted or `edit` is no proposal.
pub(super) fn propose(
    run: &Run,
    edit: &PlanEdit,
    reason: &str,
) -> Option<(String, Vec<String>, String)> {
    let record = run.orch.untrusted.as_ref()?;
    let op = op_text(edit)?;
    let question = crate::run::orch::json::cut(&format!("{op}? {reason}"), QUESTION_MAX);
    let options = OPTIONS.iter().map(|o| o.to_string()).collect();
    let context = format!(
        "the orchestrator read {} at {}, so you decide",
        record.source,
        common::text::hh_mm(record.at)
    );
    Some((question, options, context))
}

/// [`propose`] as the orchestrator's op: the question is stored with `edit` as its
/// proposal (replacing any pending one) and the orchestrator is answered at once.
/// Nothing is handled or logged as an accepted plan edit. `false` when not tainted.
pub(super) fn divert(
    run: &mut Run,
    reply: ReplyId,
    (edit, reason): (&PlanEdit, &str),
    now: u64,
    fx: &mut Vec<Effect>,
) -> bool {
    let Some(ask) = propose(run, edit, reason) else {
        return false;
    };
    super::asks::store(run, ask, Some(edit.clone()), now);
    fx.push(Effect::Reply {
        reply,
        result: Ok(ASKED.into()),
    });
    true
}

/// The wake note of the user's "do it": approved, or approved and refused.
pub(super) fn approved_note(run: &mut Run, op: &str, result: &Result<String, String>) {
    let text = match result {
        Ok(_) => format!("the user approved: {op}"),
        Err(refusal) => format!("the user approved: {op}, but it was refused: {refusal}"),
    };
    wake::note(run, text);
}

/// `AnswerAsk` of a question that carries `proposal`: `Some(0)` runs it as the user,
/// `Some(1)` declines it, `None` (answered in the window) drops it. The reply's result,
/// or `None` when it waits for an override's commit count (whose landing writes the
/// note, [`gates::override_counted`]).
pub(super) fn answered_proposal(
    run: &mut Run,
    reply: ReplyId,
    (proposal, choice): (&PlanEdit, Option<u32>),
    now: u64,
    fx: &mut Vec<Effect>,
) -> Option<Result<String, String>> {
    let op = op_text(proposal).unwrap_or_default();
    match choice {
        None => Some(Ok("answered in the window".into())),
        Some(0) => {
            let result = match proposal {
                PlanEdit::Override { task_id, reason } => {
                    let call = (task_id.as_str(), reason.as_str());
                    gates::override_on(run, reply, call, (Actor::User, true), now, fx)?
                }
                PlanEdit::AcceptRed { stage, .. } => accept_red(run, *stage, now),
                _ => Err(format!("{op} is not the user's to decide")),
            };
            approved_note(run, &op, &result);
            Some(result)
        }
        Some(_) => {
            let note = format!("the user declined: {op}");
            log(run, now, note.clone());
            wake::note(run, note);
            Some(Ok(format!("declined: {op}")))
        }
    }
}

/// The user's `accept_red` of stage `n`: the rule, then the core, which logs one line
/// naming the user (C21).
fn accept_red(run: &mut Run, n: u16, now: u64) -> Result<String, String> {
    if let Some(refusal) = rules::accept_red(run, n) {
        return Err(refusal);
    }
    let sha = full::accept_red(run, n, Actor::User, now).ok_or(rules::DRIFT)?;
    Ok(format!(
        "stage {n}'s red tier 3 on {sha} is accepted; completion and delivery go on"
    ))
}
