//! Decision 32's done gate, second half: `VerifyDone`'s verdict (decisions 55 and 56),
//! the bounce of a gate failure of `done`, and the accepted claim's move to its first
//! gate. Pure (design decision 2).

use proto::{DoneSignal, GateKind, TaskState};

use super::dispatch::history;
use super::done::{REPLACED, rejection, reply, session_window, spill_exempt};
use super::ladder::{self, worker_round};
use super::{Effect, OpId, OpResult, fallback, gates, outbox};
use crate::run::contract::{DONE_ACCEPTED, generated_files_message, protected_file_message};
use crate::run::globs::names_literally;
use crate::run::model::{FallbackState, PendingClaim, Run};

/// `VerifyDone`'s result (decisions 32, 55, 56). A rejection leaves the task working
/// and counts nothing; a protected or generated path is a gate failure of `done`; a
/// non-generated path outside `owns` is rung 3; otherwise the claim is accepted and
/// the task moves to its first gate. The turn-end fallback's claim has no reply: its
/// rejection or bounce text alone is queued as the worker's next turn instead (ruling
/// T12-N4). Only the result of the claim's own op settles it (ruling T12-N).
pub(super) fn checked(
    run: &mut Run,
    i: usize,
    op: OpId,
    result: OpResult,
    now: u64,
    fx: &mut Vec<Effect>,
) {
    if run.tasks[i].claim.as_ref().and_then(|c| c.op) != Some(op) {
        return;
    }
    let Some(pending) = run.tasks[i].claim.take() else {
        return;
    };
    let answer =
        |fx: &mut Vec<Effect>, run: &mut Run, text: Result<String, String>| match pending.reply {
            Some(id) => reply(fx, id, text),
            None => {
                if let Err(text) = text {
                    let task_id = run.tasks[i].id().to_string();
                    outbox::queue(run, &task_id, format!("[anthrex] {text}"), now);
                }
            }
        };
    let state = run.tasks[i].state;
    if state != TaskState::Working {
        let text = format!(
            "task_done is accepted only while the task is working (it is {})",
            state.label()
        );
        if let Some(id) = pending.reply {
            reply(fx, id, Err(text));
        }
        return;
    }
    // Ruling T12-I1: a claim is its session's; a session killed or replaced lost it.
    if pending.window_id.is_none() || pending.window_id != session_window(run, i) {
        if let Some(id) = pending.reply {
            reply(fx, id, Err(REPLACED.to_string()));
        }
        return;
    }
    // Ruling T12-later: the stall clock waited for the check; it runs again from here.
    let Some(r) = worker_round(&run.tasks[i]) else {
        return;
    };
    run.tasks[i].rounds[r].last_event = now;
    // Ruling T12-R4: the fallback's claim is its turn's; once a later turn has started,
    // that turn's end decides afresh.
    if pending.reply.is_none() && run.tasks[i].rounds[r].turns != pending.turn {
        return fallback::drop_stale(run, i, r, fx);
    }
    let (outside, generated, protected, head, resolution_only, signals) = match &result {
        OpResult::DoneChecked {
            outside_owns,
            generated_outside_owns,
            protected_changed,
            head,
            resolution_only,
            signals,
            ..
        } => (
            outside_owns.clone(),
            generated_outside_owns.clone(),
            protected_changed.clone(),
            head.clone(),
            *resolution_only,
            signals.as_deref().cloned().unwrap_or_default(),
        ),
        OpResult::Failed { message } => {
            let text = format!("task_done could not be checked: {message}; call task_done again");
            answer(fx, run, Err(text.clone()));
            return reengage(run, i, &pending, text, now);
        }
        _ => return,
    };
    if let Some(text) = rejection(run, i, &pending, &result) {
        history(run, i, now, text.clone());
        answer(fx, run, Err(text.clone()));
        return reengage(run, i, &pending, text, now);
    }
    let told = pending.reply.is_some();
    if !spill_exempt(run, i) {
        // Decision 56 first: a protected path `owns` does not name literally.
        let owns = &run.tasks[i].spec.owns;
        let caught: Vec<String> = protected
            .into_iter()
            .filter(|p| !names_literally(owns, p))
            .collect();
        if !caught.is_empty() {
            let text = protected_file_message(&caught);
            return bounce(run, i, &pending, text, told, now, fx);
        }
        // Milestone 9.1 decision 41: a deleted test file `owns` does not name exactly.
        if let Some(text) = super::weakening::bounce(run, i, &signals) {
            return bounce(run, i, &pending, text, told, now, fx);
        }
        // Decision 55: any non-generated path outside `owns` is rung 3.
        if !outside.is_empty() {
            let text = format!("changed files outside owns: {}", outside.join(", "));
            if let Some(id) = pending.reply {
                reply(fx, id, Err(text.clone()));
            }
            return ladder::rung3(run, i, text, now, fx);
        }
        if !generated.is_empty() {
            let text = generated_files_message(&generated);
            return bounce(run, i, &pending, text, told, now, fx);
        }
    }
    let id = pending.reply;
    super::weakening::keep(&mut run.tasks[i], signals);
    accept(run, i, pending, head, resolution_only, now);
    if let Some(id) = id {
        reply(fx, id, Ok(DONE_ACCEPTED.to_string()));
    }
}

/// A gate failure of `done` (decisions 55, 56). At rung 1 the reply is the message
/// itself; at rung 2 or 3 the session is being replaced or stopped, and the reply says
/// so instead of asking for another try (review m-7).
#[allow(clippy::too_many_arguments)]
fn bounce(
    run: &mut Run,
    i: usize,
    pending: &PendingClaim,
    text: String,
    told: bool,
    now: u64,
    fx: &mut Vec<Effect>,
) {
    let rung = ladder::gate_failure(run, i, GateKind::Done, text.clone(), told, now, fx);
    let Some(id) = pending.reply else {
        return;
    };
    let text = match rung {
        1 => {
            reengage(run, i, pending, text.clone(), now);
            text
        }
        2 => "task_done rejected again: this session is being replaced by a fresh one; stop now"
            .to_string(),
        _ => {
            let cause = run.tasks[i]
                .block
                .as_ref()
                .map(|b| b.text.clone())
                .unwrap_or_default();
            format!("task_done rejected: the task is blocked (mis_sized): {cause}; stop now")
        }
    };
    reply(fx, id, Err(text));
}

/// Ruling T12-N2: a claim answered after its turn ended (or its process exited) leaves
/// the worker waiting for nothing: the reply text goes to it as its next turn, which
/// resumes the session when its process has ended. Within the claiming turn the tool
/// reply is enough; a later turn open meanwhile gets the text at its end (ruling
/// T12-O2). The fallback's claim (no reply) had its text queued already.
fn reengage(run: &mut Run, i: usize, pending: &PendingClaim, text: String, now: u64) {
    let task = &run.tasks[i];
    let claiming_turn = |r: usize| {
        let round = &task.rounds[r];
        round.turn_open && round.turns == pending.turn
    };
    if pending.reply.is_none() || worker_round(task).is_none_or(claiming_turn) {
        return;
    }
    let text = if text.starts_with("[anthrex]") {
        text
    } else {
        format!("[anthrex] {text}")
    };
    let id = task.id().to_string();
    outbox::queue(run, &id, text, now);
}

/// Decision 32: the claim is recorded and the task moves to its first gate — `proof`
/// (tdd), `check` (a check in the profile), `review`, or the merge queue; a handed-back
/// task goes straight back to the merge queue (decision 36).
fn accept(
    run: &mut Run,
    i: usize,
    pending: PendingClaim,
    head: String,
    resolution_only: Option<bool>,
    now: u64,
) {
    let task = &mut run.tasks[i];
    // Ruling T14-I3: an accepted claim ends the conflict it resolved.
    task.resolving = false;
    let signal = pending.claim.signal;
    task.done = Some(pending.claim);
    task.head = Some(head);
    if let Some(r) = worker_round(task) {
        let round = &mut task.rounds[r];
        round.turn_had_task_done = true;
        round.fallback = FallbackState::None;
        round.fallback_waiting = false;
    }
    if task.handback_due {
        let how = match signal {
            DoneSignal::TaskDone => "task_done",
            DoneSignal::TurnEndFallback => "the turn-end fallback",
        };
        history(run, i, now, format!("done ({how}); the run head first"));
        return super::merge::hand_back_due(run, i, now);
    }
    let task = &mut run.tasks[i];
    // Ruling T14-R2: straight back to the queue only for the conflict's resolution and
    // nothing more; any other claim passes every gate.
    let resolved = std::mem::take(&mut task.handed_back) && resolution_only == Some(true);
    task.resolution = None;
    let next = if resolved {
        TaskState::MergeQueue
    } else {
        gates::next_gate(run, i, None)
    };
    gates::enter(run, i, next, now);
    let how = match signal {
        DoneSignal::TaskDone => "task_done",
        DoneSignal::TurnEndFallback => "the turn-end fallback",
    };
    history(run, i, now, format!("done ({how}); next: {}", next.label()));
}
