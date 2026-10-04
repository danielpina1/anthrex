//! Decision 32's done gate: `task_done` and `task_blocked` from a worker (with the MCP
//! section's engine-side acceptance), `VerifyDone`'s verdict with decisions 55 and 56's
//! protected, generated and spill split. The turn-end fallback, whose claim this
//! module checks, is in `fallback.rs`. Pure (design decision 2).

use proto::{BlockReason, DoneSignal, RunState, TaskState, TestMode, ToolCall};

use super::dispatch::block;
use super::ladder::{self, live, worker_round};
use super::tools::{DoneArgs, parse_blocked, parse_done};
use super::{Effect, EngineState, OpKind, OpResult, ReplyId, emit_op, next_op};
use crate::run::contract::{BLOCKED_CLASSIFYING, blocked_recorded};
use crate::run::model::{DoneClaim, PendingClaim, Run, StallState};
use crate::run::orch::contract::STOP_AND_WAIT_REFUSAL;

pub(super) use super::done_checked::checked;

pub(super) fn reply(fx: &mut Vec<Effect>, reply: ReplyId, result: Result<String, String>) {
    fx.push(Effect::Reply { reply, result });
}

/// `Event::Tool`: a call from a window whose launch is still in flight waits for it
/// (`early.rs`); any other is answered now.
pub(super) fn tool(
    state: &mut EngineState,
    id: ReplyId,
    call: ToolCall,
    now: u64,
    fx: &mut Vec<Effect>,
) {
    let running = state
        .runs
        .get(&call.run_id)
        .is_some_and(|r| r.state == RunState::Running);
    if running && super::early::holds_call(state, &call) {
        return super::early::hold_call(state, id, call, now, fx);
    }
    answer(state, id, call, now, fx)
}

/// The MCP tools' engine-side acceptance (Interfaces, "MCP tools"), in order: the run,
/// its state, the tool, the caller, the task's state, the arguments.
pub(super) fn answer(
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
    // Milestone 9.5 decision 20: a lane's session is answered in its lane's view; a
    // lane that left the race is refused (Interfaces, "MCP").
    if let Some((i, lane)) = super::race::lane_of_window(run, call.window_id) {
        if let Some(text) = super::race::refusal(run, i, lane, &call.tool) {
            return reply(fx, id, Err(text));
        }
        let call = |run: &mut Run, fx: &mut Vec<Effect>| tool_call(run, id, &call, now, fx);
        super::race::with_lane(run, i, lane, now, fx, call);
        return;
    }
    tool_call(run, id, &call, now, fx)
}

/// The tool, by name.
fn tool_call(run: &mut Run, id: ReplyId, call: &ToolCall, now: u64, fx: &mut Vec<Effect>) {
    match call.tool.as_str() {
        "task_done" | "task_blocked" => worker_tool(run, id, call, now, fx),
        "submit_review" => super::review::submit(run, id, call, now, fx),
        // Milestone 9 decision 42f, past M8a's run gate above.
        "task_note" => super::worker_messages::task_note(run, id, call, now, fx),
        // Milestone 9 decision 35: a research task's report.
        "submit_scout_report" => super::kinds::submit_research(run, id, call, now, fx),
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
    // Milestone 9.5 decision 25: a paired task's test writer is its writer meanwhile.
    let current = found.is_some_and(|i| {
        let task = &run.tasks[i];
        worker_round(task).is_some_and(|r| {
            let round = &task.rounds[r];
            round.role == call.role && live(round) && round.window_id == Some(call.window_id)
        })
    });
    let (Some(i), true) = (found, current) else {
        let text = format!("this window is not the current worker of task {task_id}");
        return reply(fx, id, Err(text));
    };
    // Milestone 9 decision 42c: the fastest stop, ahead of the working-only check.
    if call.tool == "task_done" && crate::run::edits_state::is_paused(&run.tasks[i]) {
        return reply(fx, id, Err(STOP_AND_WAIT_REFUSAL.to_string()));
    }
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
    // Decision 26: an implementer names the test writer's test and red, or neither.
    if let Some(text) = super::pair::implementer_mismatch(&run.tasks[i], &args) {
        return reply(fx, id, Err(text));
    }
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
pub(super) const REPLACED: &str = "this session is being replaced; its task_done no longer applies";

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
pub(super) fn session_window(run: &Run, i: usize) -> Option<u32> {
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
    let args = super::pair::fill(&run.tasks[i], args);
    let task = &run.tasks[i];
    let kind = OpKind::VerifyDone {
        worktree: task.worktree.clone(),
        start: task
            .start_commit
            .clone()
            .unwrap_or_else(|| run.head_for(task).to_string()),
        run_head: run.head_for(task).to_string(),
        owns: task.spec.owns.clone(),
        generated: run.profile.generated.clone(),
        protected: run.profile.protected.clone(),
        spill_exempt: spill_exempt(run, i),
        red: args.red.clone(),
        // Set exactly while `handed_back` is (merge::handed_back, ladder::end_hand_back).
        resolution: task.resolution.clone().map(Box::new),
        not_own: super::worker_messages::not_own(task),
        not_run: super::worker_messages::not_run(task),
        signals: super::weakening::spec(run, task),
        // Milestone 9.1 decision 51: a sync task's spill is against its merge.
        spill_base: task.sync.as_ref().map(|s| s.base_tree.clone()),
        // Controller ruling C-21 (3, 5).
        sync: task.sync.as_ref().map(|s| {
            Box::new(crate::run::model::SyncCheck {
                onto: s.onto.clone(),
                to_head: s.to_head.clone(),
                upper: s.handed.last().cloned(),
            })
        }),
    };
    let task_id = task.id().to_string();
    let window_id = session_window(run, i);
    let turn = worker_round(task).map_or(0, |r| task.rounds[r].turns);
    let session = worker_round(task).map(|r| task.rounds[r].session);
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
            session,
        },
    });
    emit_op(run, op, Some(&task_id), kind, fx);
}

/// Decision 35: a task the user has overridden is exempt from the protected, generated
/// and spill checks.
pub(super) fn spill_exempt(run: &Run, i: usize) -> bool {
    run.tasks[i].merged_without_approval.is_some()
}

/// `task_blocked` (decision 32): `question` and `environment` block the task, the
/// session waiting for an answer; `mis_sized` is rung 3. With no kind (M8b decision 21)
/// the task is a question at once, and a decider classifies it.
#[allow(clippy::too_many_arguments)]
fn task_blocked(
    run: &mut Run,
    i: usize,
    id: ReplyId,
    kind: Option<&'static str>,
    reason: String,
    now: u64,
    fx: &mut Vec<Effect>,
) {
    // Review I1: a new block ends any earlier one's classification.
    run.tasks[i].pending_classification = None;
    run.tasks[i].block_source = None;
    match kind {
        Some("mis_sized") => mis_sized(run, i, &reason, now, fx),
        Some("environment") => block(run, i, BlockReason::Environment, reason, now),
        Some(_) => block(run, i, BlockReason::Question, reason, now),
        None => {
            block(run, i, BlockReason::Question, reason.clone(), now);
            // Milestone 9.5 decision 20: a lane is out of its race whatever the kind; the
            // final fix wave's m8: an adoption classifies it after the crown.
            let racing = run.tasks[i].lane_view.is_some();
            run.tasks[i].lane_unclassified = racing;
            if !racing && super::deciders::classify(run, i, reason, now, fx) {
                return reply(fx, id, Ok(BLOCKED_CLASSIFYING.to_string()));
            }
        }
    }
    reply(fx, id, Ok(blocked_recorded(kind.unwrap_or("question"))));
}

/// A `mis_sized` block (decision 32): rung 3, whether the worker typed it or a decider
/// classified it (M8b decision 21).
pub(super) fn mis_sized(run: &mut Run, i: usize, reason: &str, now: u64, fx: &mut Vec<Effect>) {
    let text = format!("the worker reported the task mis-sized: {reason}");
    ladder::rung3(run, i, text, now, fx);
}

/// The first of decision 32's rejections that applies, or `None`.
pub(super) fn rejection(
    run: &Run,
    i: usize,
    claim: &PendingClaim,
    result: &OpResult,
) -> Option<String> {
    let OpResult::DoneChecked {
        commits,
        dirty_tracked,
        merge_in_progress,
        untracked_in_owns,
        red_ok,
        head,
        head_branch,
        sync_kept,
        ..
    } = result
    else {
        return None;
    };
    let task = &run.tasks[i];
    let branch = &task.branch;
    // Milestone 9.5 decision 25: a test writer's claim needs its test and red however
    // it came.
    let explicit = claim.claim.signal == DoneSignal::TaskDone || super::pair::writing(task);
    // Carry T8 (M8a.8 minor 11), as final fix batch F1b recasts it: the worker commits
    // on a detached `HEAD`, which the engine records on the task's branch; a `HEAD` that
    // names a branch, or a stopped rebase, is not a claim the branch can carry.
    Some(if head_branch.as_deref() != Some(branch.as_str()) {
        "task_done rejected: this worktree's HEAD must be a detached commit with no rebase in progress; run git checkout --detach (or finish the rebase), commit, and call task_done again".to_string()
    } else if *commits == 0 {
        // Milestone 9 decision 42e: the count leaves out refresh merges (`not_own`).
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
    } else if let Some(text) = super::pair::writer_rejection(task, claim.claim.red.as_deref(), head)
    {
        text
    } else if *sync_kept == Some(false) {
        // Controller ruling C-21 (5).
        super::propagate::lost_merge(run, task)
    } else if *red_ok == Some(false) {
        format!(
            "task_done rejected: red {} is not a commit on this task's branch after its start commit",
            claim.claim.red.as_deref().unwrap_or_default()
        )
    } else {
        return None;
    })
}
