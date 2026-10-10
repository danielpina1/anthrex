//! Milestone 9.9 decisions 15 and 16 (OFA §4.3): `ask_user`, the orchestrator's one
//! question to the user, kept as `RunOrch.ask` and shown in the snapshot, and the user's
//! `AnswerAsk`, which reaches the orchestrator as a wake note. A tool call never waits
//! for the user: it returns at once. The question and the chosen option are the
//! orchestrator's own text and stay data; the wake note makes them one line. Pure.

use proto::RunState;

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

/// Stores `(question, options, context)` as the pending question, replacing an
/// unanswered one, and answers at once.
pub(super) fn ask(
    run: &mut Run,
    reply: ReplyId,
    (question, options, context): (String, Vec<String>, String),
    now: u64,
    fx: &mut Vec<Effect>,
) {
    run.orch.ask_seq = run.orch.ask_seq.saturating_add(1);
    let old = run.orch.ask.replace(AskRecord {
        id: run.orch.ask_seq,
        question: question.clone(),
        options,
        context,
        asked_at: now,
    });
    let (text, reply_text) = match old {
        Some(old) => (
            format!(
                "orchestrator asks: {} (replaces \"{}\")",
                quoted(&question),
                quoted(&old.question)
            ),
            "asked, replacing your earlier question; the answer arrives as a message",
        ),
        None => (
            format!("orchestrator asks: {}", quoted(&question)),
            "asked; the answer arrives as a message",
        ),
    };
    log(run, now, text);
    fx.push(Effect::Reply {
        reply,
        result: Ok(reply_text.into()),
    });
}

/// `AnswerAsk`: `choice` is an option's 0-based index. `Some` wakes the orchestrator with
/// the option and clears the question; `None` clears it (the user typed in the window).
pub(super) fn answer(
    state: &mut EngineState,
    reply: ReplyId,
    (run_id, id, choice): (&str, u64, Option<u32>),
    now: u64,
    fx: &mut Vec<Effect>,
) {
    let result = match state.runs.get_mut(run_id) {
        Some(run) => answered(run, id, choice, now),
        None => Err(format!("unknown run {run_id}")),
    };
    fx.push(Effect::Reply { reply, result });
}

fn answered(run: &mut Run, id: u64, choice: Option<u32>, now: u64) -> Result<String, String> {
    let Some(ask) = run.orch.ask.as_ref() else {
        return Err(format!("run {} has no pending question", run.id));
    };
    if ask.id != id {
        return Err("that question was replaced; answer the new one".into());
    }
    let picked = match choice {
        None => None,
        Some(i) => match ask.options.get(i as usize) {
            Some(option) => Some(option.clone()),
            None if ask.options.is_empty() => return Err("this question has no options".into()),
            None => return Err(format!("choose 1 to {}", ask.options.len())),
        },
    };
    run.orch.ask = None;
    match picked {
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
    }
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
