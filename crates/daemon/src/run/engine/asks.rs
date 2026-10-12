//! Milestone 9.9 decisions 15 and 16 (OFA §4.3): `ask_user`, the orchestrator's one
//! question to the user, kept as `RunOrch.ask` and shown in the snapshot, and the user's
//! `AnswerAsk`, which reaches the orchestrator as a wake note. A tool call never waits
//! for the user: it returns at once. The question and the chosen option are the
//! orchestrator's own text and stay data; the wake note makes them one line. Pure.

use proto::{PlanEdit, RunState};

use super::requests::log;
use super::{Effect, EngineState, ReplyId, wake};
use crate::run::model::Run;
use crate::run::orch::AskRecord;

/// A question quoted in the run log, in characters.
const LOG_QUESTION_MAX: usize = 120;

/// Untrusted text as one bounded run-log line.
fn quoted(text: &str) -> String {
    let line = crate::run::messages::one_line(text);
    crate::run::orch::json::cut(&line, LOG_QUESTION_MAX)
}

/// Stores `(question, options, context)` as the pending question, with `proposal`
/// (milestone 9.13 decision 21), replacing an unanswered one, and logs it. The
/// replaced question, if any.
pub(super) fn store(
    run: &mut Run,
    (question, options, context): (String, Vec<String>, String),
    proposal: Option<PlanEdit>,
    now: u64,
) -> Option<AskRecord> {
    run.orch.ask_seq = run.orch.ask_seq.saturating_add(1);
    let old = run.orch.ask.replace(AskRecord {
        id: run.orch.ask_seq,
        question: question.clone(),
        options,
        context,
        asked_at: now,
        proposal,
    });
    let text = match &old {
        Some(old) => format!(
            "orchestrator asks: {} (replaces \"{}\")",
            quoted(&question),
            quoted(&old.question)
        ),
        None => format!("orchestrator asks: {}", quoted(&question)),
    };
    log(run, now, text);
    old
}

/// `ask_user`: [`store`] the question, and answer at once.
pub(super) fn ask(
    run: &mut Run,
    reply: ReplyId,
    ask: (String, Vec<String>, String),
    now: u64,
    fx: &mut Vec<Effect>,
) {
    let reply_text = match store(run, ask, None, now) {
        Some(_) => "asked, replacing your earlier question; the answer arrives as a message",
        None => "asked; the answer arrives as a message",
    };
    fx.push(Effect::Reply {
        reply,
        result: Ok(reply_text.into()),
    });
}

/// `AnswerAsk`: `choice` is an option's 0-based index. `Some` wakes the orchestrator with
/// the option and clears the question; `None` clears it (the user typed in the window).
/// A question that carries a proposal runs, declines or drops it (milestone 9.13
/// decision 21); a "do it" override's reply waits for its commit count.
pub(super) fn answer(
    state: &mut EngineState,
    reply: ReplyId,
    (run_id, id, choice): (&str, u64, Option<u32>),
    now: u64,
    fx: &mut Vec<Effect>,
) {
    let result = match state.runs.get_mut(run_id) {
        Some(run) => answered(run, reply, (id, choice), now, fx),
        None => Some(Err(format!("unknown run {run_id}"))),
    };
    if let Some(result) = result {
        fx.push(Effect::Reply { reply, result });
    }
}

fn answered(
    run: &mut Run,
    reply: ReplyId,
    (id, choice): (u64, Option<u32>),
    now: u64,
    fx: &mut Vec<Effect>,
) -> Option<Result<String, String>> {
    let Some(ask) = run.orch.ask.as_ref() else {
        return Some(Err(format!("run {} has no pending question", run.id)));
    };
    if ask.id != id {
        return Some(Err("that question was replaced; answer the new one".into()));
    }
    let picked = match choice {
        None => None,
        Some(i) => match ask.options.get(i as usize) {
            Some(option) => Some(option.clone()),
            None if ask.options.is_empty() => {
                return Some(Err("this question has no options".into()));
            }
            None => return Some(Err(format!("choose 1 to {}", ask.options.len()))),
        },
    };
    let ask = run.orch.ask.take()?;
    if let Some(proposal) = &ask.proposal {
        let call = (proposal, choice);
        return super::untrusted::answered_proposal(run, reply, call, now, fx);
    }
    Some(match picked {
        Some(option) => {
            wake::note(run, format!("the user chose: {option}"));
            log(
                run,
                now,
                format!("the user answered the orchestrator: {}", quoted(&option)),
            );
            Ok(format!("answered: {option}"))
        }
        None => Ok("answered in the window".into()),
    })
}

/// Post-step (decision 16, ruling R5): the question is dropped when the run enters
/// `Complete` (`before` was not), and whenever the run is `Accepted`, `Discarded` or
/// `Failed`. One asked while `Complete` waits until the run leaves it.
pub(super) fn drop_settled(before: Option<&Run>, run: &mut Run) {
    if run.orch.ask.is_none() {
        return;
    }
    let entered =
        run.state == RunState::Complete && before.is_some_and(|b| b.state != RunState::Complete);
    if run.state.is_terminal() || entered {
        run.orch.ask = None;
    }
}
