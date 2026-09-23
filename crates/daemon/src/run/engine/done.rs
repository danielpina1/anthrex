//! Decision 32's done gate: `task_done` and `task_blocked` from a worker (with the MCP
//! section's engine-side acceptance), `VerifyDone`'s verdict with decisions 55 and 56's
//! protected, generated and spill split. The turn-end fallback, whose claim this
//! module checks, is in `fallback.rs`. Pure (design decision 2).

use proto::{
    AgentRole, BlockReason, DoneSignal, GateKind, RunState, TaskState, TestMode, ToolCall,
};

use super::dispatch::{block, history};
use super::ladder::{self, live, worker_round};
use super::tools::{DoneArgs, parse_blocked, parse_done};
use super::{
    Effect, EngineState, OpId, OpKind, OpResult, ReplyId, emit_op, fallback, gates, next_op, outbox,
};
use crate::run::contract::{
    DONE_ACCEPTED, blocked_recorded, generated_files_message, protected_file_message,
};
use crate::run::globs::names_literally;
use crate::run::model::{DoneClaim, FallbackState, PendingClaim, Run, StallState};

fn reply(fx: &mut Vec<Effect>, reply: ReplyId, result: Result<String, String>) {
    fx.push(Effect::Reply { reply, result });
}

/// `Event::Tool`: the MCP tools' engine-side acceptance (Interfaces, "MCP tools"), in
/// order: the run, its state, the tool, the caller, the task's state, the arguments.
pub(super) fn tool(
    state: &mut EngineState,
    id: ReplyId,
    call: ToolCall,
    now: u64,
    fx: &mut Vec<Effect>,
) {
    let Some(run) = state.runs.get_mut(&call.run_id) else {
        return reply(fx, id, Err(format!("unknown run {}", call.run_id)));
    };
    match run.state {
        RunState::Running => {}
        RunState::Paused => {
            let text = format!("run {} is paused; the user must resume it", run.id);
            return reply(fx, id, Err(text));
        }
        other => {
            let text = format!("run {} is {}", run.id, other.label());
            return reply(fx, id, Err(text));
        }
    }
    match call.tool.as_str() {
        "task_done" | "task_blocked" => worker_tool(run, id, &call, now, fx),
        "submit_review" => super::review::submit(run, id, &call, now, fx),
        other => {
            let role = serde_json::to_value(call.role)
                .ok()
                .and_then(|v| v.as_str().map(str::to_string))
                .unwrap_or_default();
            reply(
                fx,
                id,
                Err(format!("tool {other} is not available to the {role} role")),
            )
        }
    }
}

fn worker_tool(run: &mut Run, id: ReplyId, call: &ToolCall, now: u64, fx: &mut Vec<Effect>) {
    let task_id = call.task_id.clone().unwrap_or_default();
    let found = run.tasks.iter().position(|t| t.id() == task_id);
    let current = found.is_some_and(|i| {
        let task = &run.tasks[i];
        call.role == AgentRole::Worker
            && worker_round(task).is_some_and(|r| {
                let round = &task.rounds[r];
                live(round) && round.window_id == Some(call.window_id)
            })
    });
    let (Some(i), true) = (found, current) else {
        let text = format!("this window is not the current worker of task {task_id}");
        return reply(fx, id, Err(text));
    };
    let state = run.tasks[i].state;
    if state != TaskState::Working {
        let text = format!(
            "{} is accepted only while the task is working (it is {})",
            call.tool,
            state.label()
        );
        return reply(fx, id, Err(text));
    }
    if call.tool == "task_blocked" {
        return match parse_blocked(&call.args) {
            Ok((kind, reason)) => task_blocked(run, i, id, kind, reason, now, fx),
            Err(e) => reply(fx, id, Err(format!("invalid arguments: {e}"))),
        };
    }
    let args = match parse_done(&call.args) {
        Ok(args) => args,
        Err(e) => return reply(fx, id, Err(format!("invalid arguments: {e}"))),
    };
    let window = Some(call.window_id);
    if run.tasks[i]
        .claim
        .as_ref()
        .is_some_and(|c| c.window_id == window)
    {
        let text = "task_done is already being checked; wait for its reply".to_string();
        return reply(fx, id, Err(text));
    }
    // Ruling T12-O1: a claim inside the interrupt grace ends the grace.
    if let Some(r) = worker_round(&run.tasks[i]) {
        let round = &mut run.tasks[i].rounds[r];
        if matches!(round.stall, StallState::Interrupted { .. }) {
            round.stall = StallState::Nudged;
        }
    }
    // A claim of an earlier session is replaced (ruling T12-I1).
    drop_claim(run, i, REPLACED, fx);
    claim(run, i, Some(id), args, DoneSignal::TaskDone, fx);
}

/// The reply to a claim whose session was replaced or stopped (ruling T12-I1).
const REPLACED: &str = "this session is being replaced; its task_done no longer applies";

/// Ruling T12-I1: ends task `i`'s claim, if any, answering its tool call with `text`.
/// Its `VerifyDone` is no longer awaited: its result is dropped (ruling T12-N).
pub(super) fn drop_claim(run: &mut Run, i: usize, text: &str, fx: &mut Vec<Effect>) {
    if let Some(PendingClaim {
        reply: Some(id), ..
    }) = run.tasks[i].claim.take()
    {
        reply(fx, id, Err(text.to_string()));
    }
}

/// The window of task `i`'s current worker session: its latest worker round, unless
/// the engine killed it or its resume failed (`retiring`). A round that ended between
/// turns is still that session, since the next delivery resumes it (ruling T12-I1).
fn session_window(run: &Run, i: usize) -> Option<u32> {
    let task = &run.tasks[i];
    worker_round(task)
        .map(|r| &task.rounds[r])
        .filter(|r| !r.retiring && (!r.ended || r.session_id.is_some()))
        .and_then(|r| r.window_id)
}

/// Records the claim and sends `VerifyDone`; the reply follows its result.
pub(super) fn claim(
    run: &mut Run,
    i: usize,
    id: Option<ReplyId>,
    args: DoneArgs,
    signal: DoneSignal,
    fx: &mut Vec<Effect>,
) {
    let task = &run.tasks[i];
    let kind = OpKind::VerifyDone {
        worktree: task.worktree.clone(),
        start: task
            .start_commit
            .clone()
            .unwrap_or_else(|| run.run_head.clone()),
        run_head: run.run_head.clone(),
        owns: task.spec.owns.clone(),
        generated: run.profile.generated.clone(),
        protected: run.profile.protected.clone(),
        spill_exempt: spill_exempt(run, i),
        red: args.red.clone(),
    };
    let task_id = task.id().to_string();
    let window_id = session_window(run, i);
    let turn = worker_round(task).map_or(0, |r| task.rounds[r].turns);
    let op = next_op(run);
    run.tasks[i].claim = Some(PendingClaim {
        window_id,
        op: Some(op),
        turn,
        reply: id,
        claim: DoneClaim {
            summary: args.summary,
            test: args.test,
            red: args.red,
            signal,
        },
    });
    emit_op(run, op, Some(&task_id), kind, fx);
}

/// Decision 35: a task the user has overridden is exempt from the protected, generated
/// and spill checks.
fn spill_exempt(run: &Run, i: usize) -> bool {
    run.tasks[i].merged_without_approval.is_some()
}

/// `task_blocked` (decision 32): `question` and `environment` block the task, the
/// session waiting for an answer; `mis_sized` is rung 3.
#[allow(clippy::too_many_arguments)]
fn task_blocked(
    run: &mut Run,
    i: usize,
    id: ReplyId,
    kind: &'static str,
    reason: String,
    now: u64,
    fx: &mut Vec<Effect>,
) {
    match kind {
        "mis_sized" => {
            let text = format!("the worker reported the task mis-sized: {reason}");
            ladder::rung3(run, i, text, now, fx);
        }
        "environment" => block(run, i, BlockReason::Environment, reason, now),
        _ => block(run, i, BlockReason::Question, reason, now),
    }
    reply(fx, id, Ok(blocked_recorded(kind)));
}

/// The first of decision 32's rejections that applies, or `None`.
fn rejection(run: &Run, i: usize, claim: &PendingClaim, result: &OpResult) -> Option<String> {
    let OpResult::DoneChecked {
        commits,
        dirty_tracked,
        merge_in_progress,
        untracked_in_owns,
        red_ok,
        head_branch,
        ..
    } = result
    else {
        return None;
    };
    let task = &run.tasks[i];
    let branch = &task.branch;
    let explicit = claim.claim.signal == DoneSignal::TaskDone;
    // Carry T8 (M8a.8 minor 11): commits made off the task's branch are not its work.
    Some(if head_branch.as_deref() != Some(branch.as_str()) {
        format!(
            "task_done rejected: HEAD is not on {branch}; commit your work on {branch} and call task_done again"
        )
    } else if *commits == 0 {
        "task_done rejected: the branch has no commit since the task started; commit your work first".into()
    } else if *dirty_tracked > 0 {
        format!(
            "task_done rejected: the tracked tree has uncommitted changes ({dirty_tracked} files); commit or revert them first"
        )
    } else if *merge_in_progress {
        "task_done rejected: a merge is in progress; finish it with git commit first".into()
    } else if !untracked_in_owns.is_empty() {
        format!(
            "task_done rejected: untracked files inside this task's owns are not committed: {}",
            untracked_in_owns.join(", ")
        )
    } else if explicit
        && task.test_mode == TestMode::Tdd
        && (claim.claim.test.is_none() || claim.claim.red.is_none())
    {
        "task_done rejected: this is a tdd task; name the test (test) and the commit where it was added and failed (red)".into()
    } else if *red_ok == Some(false) {
        format!(
            "task_done rejected: red {} is not a commit on this task's branch after its start commit",
            claim.claim.red.as_deref().unwrap_or_default()
        )
    } else {
        return None;
    })
}

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
    let (outside, generated, protected, head) = match &result {
        OpResult::DoneChecked {
            outside_owns,
            generated_outside_owns,
            protected_changed,
            head,
            ..
        } => (
            outside_owns.clone(),
            generated_outside_owns.clone(),
            protected_changed.clone(),
            head.clone(),
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
    accept(run, i, pending, head, now);
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
fn accept(run: &mut Run, i: usize, pending: PendingClaim, head: String, now: u64) {
    let task = &mut run.tasks[i];
    let signal = pending.claim.signal;
    task.done = Some(pending.claim);
    task.head = Some(head);
    if let Some(r) = worker_round(task) {
        let round = &mut task.rounds[r];
        round.turn_had_task_done = true;
        round.fallback = FallbackState::None;
        round.fallback_waiting = false;
    }
    let next = if std::mem::take(&mut task.handed_back) {
        TaskState::MergeQueue
    } else {
        gates::next_gate(run, i, None)
    };
    gates::enter(run, i, next);
    let how = match signal {
        DoneSignal::TaskDone => "task_done",
        DoneSignal::TurnEndFallback => "the turn-end fallback",
    };
    history(run, i, now, format!("done ({how}); next: {}", next.label()));
}
