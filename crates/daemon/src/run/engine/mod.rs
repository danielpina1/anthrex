//! The run reducer (design decision 2): `step(state, event) -> (state, effects)`. Pure —
//! no `std::fs`, `std::process`, `std::thread`, `tokio` or `std::time::SystemTime`. The
//! clock arrives in [`Event::now`]; every side effect leaves as an [`Effect`], and the
//! result of an [`Effect::Op`] comes back as [`EventKind::OpDone`].
//!
//! M8a.11 builds the skeleton and the first slice of behaviour: run start and the plan
//! gate (decision 14), approve and reject, the critical-path scheduler with its writer
//! and reader slots (decision 41), worktree preparation and the dispatch of headless
//! worker and reviewer windows (decisions 16, 19, 24–26), turn-based delivery's queue
//! (decision 29's gate, first part), and the revision counter (decision 47). Files:
//! `requests.rs` (client requests), `schedule.rs` (runnability, ordering and slots,
//! pure functions of a run), `dispatch.rs` (what the scheduler starts, and the results
//! of the ops it emits), `outbox.rs` (the message queue and its delivery gate).
//!
//! M8a.12 adds `done.rs` (the done gate, `task_blocked` and the turn-end fallback),
//! `tools.rs` (the worker tools' arguments), `signals.rs` (decision 32's session rules
//! and the stall watchdog) and `ladder.rs` (decision 38's ladder and decision 40's
//! budgets). Later tasks add `gates.rs`, `merge.rs` and `restore.rs`.

use std::collections::BTreeMap;
use std::path::PathBuf;

use proto::{FinishAction, PlanEdit, TokenUsage, ToolCall};

use super::model::{OpId, PendingOp, Run};
use super::validate::EditScope;

mod dispatch;
mod done;
mod holds;
pub(crate) mod ladder;
mod ops;
mod outbox;
mod requests;
pub(crate) mod schedule;
mod signals;
mod tools;

pub use crate::headless::TurnOutcome;
pub use ops::{OpKind, OpResult};
pub use signals::INTERRUPT_GRACE_SECS;

/// Identifies a client request waiting for its [`Effect::Reply`].
pub type ReplyId = u64;

/// Everything the engine knows. `revision` is decision 47's global counter, bumped with
/// every run's own.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct EngineState {
    pub runs: BTreeMap<String, Run>,
    pub revision: u64,
    pub stopped: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Event {
    /// Unix seconds, read by the driver.
    pub now: u64,
    pub kind: EventKind,
}

#[derive(Debug, Clone, PartialEq)]
pub enum EventKind {
    Start {
        reply: ReplyId,
        /// Boxed: a whole run dwarfs every other event.
        run: Box<Run>,
    },
    Approve {
        reply: ReplyId,
        run_id: String,
    },
    Reject {
        reply: ReplyId,
        run_id: String,
    },
    Edit {
        reply: ReplyId,
        run_id: String,
        edits: Vec<PlanEdit>,
        scope: EditScope,
    },
    Retry {
        reply: ReplyId,
        run_id: String,
        task_id: String,
    },
    Override {
        reply: ReplyId,
        run_id: String,
        task_id: String,
        reason: String,
    },
    Cancel {
        reply: ReplyId,
        run_id: String,
    },
    /// `rebaseline`: the (base sha, run head) the driver read.
    Resume {
        reply: ReplyId,
        run_id: String,
        rebaseline: Option<(String, String)>,
    },
    /// Decision 21: a guard saw the base branch advance.
    BaseAdvanced {
        run_id: String,
        to: String,
        commits: u32,
    },
    Finish {
        reply: ReplyId,
        run_id: String,
        action: FinishAction,
    },
    Tool {
        reply: ReplyId,
        call: ToolCall,
    },
    OpDone {
        run_id: String,
        op: OpId,
        result: OpResult,
    },
    Signal {
        window_id: u32,
        signal: AgentSignal,
    },
    Delivered {
        run_id: String,
        message_ids: Vec<u64>,
        ok: bool,
        error: Option<String>,
    },
    Restore {
        runs: Vec<Run>,
        replay: Vec<(String, OpId, OpResult)>,
    },
    Stop,
    Tick,
}

/// The driver's translation of a window's session events (decision 27).
#[derive(Debug, Clone, PartialEq)]
pub enum AgentSignal {
    Init {
        session_id: String,
    },
    TurnStarted,
    ToolUse {
        name: String,
    },
    /// `denials`: the tool names of the result's `permission_denials` (M8a.12 fix round
    /// 1, review m-4: the Interfaces' `u32` became the names, which `denied_text` needs).
    TurnEnded {
        outcome: TurnOutcome,
        usage: Option<TokenUsage>,
        denials: Vec<String>,
    },
    ApiRetry {
        error: String,
        delay_ms: u64,
    },
    PermissionDenied {
        tool: String,
        reason: String,
    },
    SubagentStart {
        agent_id: String,
    },
    SubagentStop {
        agent_id: String,
    },
    /// Any other event: text, a tool result, compaction, an unknown line.
    Activity,
    ProcessExited {
        code: Option<i32>,
        killed_by_engine: bool,
        pid: u32,
    },
    ProcessStarted {
        pid: u32,
    },
}

/// `Op` carries a whole `OpKind` (about 256 bytes); effects live only between a step
/// and the driver, so boxing every op would buy nothing.
#[allow(clippy::large_enum_variant)]
#[derive(Debug, Clone, PartialEq)]
pub enum Effect {
    Reply {
        reply: ReplyId,
        result: Result<String, String>,
    },
    Op {
        run_id: String,
        op: OpId,
        kind: OpKind,
    },
    /// One new turn (decision 29).
    Deliver {
        run_id: String,
        message_ids: Vec<u64>,
        window_id: u32,
        text: String,
    },
    Interrupt {
        window_id: u32,
    },
    KillWindow {
        window_id: u32,
    },
    RetireWindow {
        window_id: u32,
    },
    RemoveWindow {
        window_id: u32,
    },
    WatchWorktree {
        root: PathBuf,
    },
    UnwatchWorktree {
        root: PathBuf,
    },
    Persist {
        run_id: String,
        urgent: bool,
    },
    WriteReport {
        run_id: String,
    },
    Publish {
        structural: bool,
    },
}

/// One reducer step: apply the event, run the scheduler on every run, then bump the
/// revision of every run that changed (decision 47) and put `Persist` first and
/// `Publish` last, the order the driver executes them in (decision 43).
pub fn step(mut state: EngineState, event: Event) -> (EngineState, Vec<Effect>) {
    if state.stopped {
        return (state, Vec::new());
    }
    let before = state.runs.clone();
    let before_revision = state.revision;
    let now = event.now;
    let mut fx = Vec::new();
    match event.kind {
        EventKind::Start { reply, run } => requests::start(&mut state, reply, *run, now, &mut fx),
        EventKind::Approve { reply, run_id } => {
            requests::approve(&mut state, reply, &run_id, now, &mut fx)
        }
        EventKind::Reject { reply, run_id } => {
            requests::reject(&mut state, reply, &run_id, now, &mut fx)
        }
        EventKind::Edit {
            reply,
            run_id,
            edits,
            scope,
        } => requests::edit(&mut state, reply, &run_id, &edits, &scope, now, &mut fx),
        EventKind::Retry { reply, .. } => requests::not_yet(&mut fx, reply, "run retry"),
        EventKind::Override { reply, .. } => requests::not_yet(&mut fx, reply, "run override"),
        EventKind::Cancel { reply, .. } => requests::not_yet(&mut fx, reply, "run cancel"),
        EventKind::Resume { reply, .. } => requests::not_yet(&mut fx, reply, "run resume"),
        EventKind::Finish { reply, .. } => requests::not_yet(&mut fx, reply, "run finish"),
        EventKind::Tool { reply, call } => done::tool(&mut state, reply, call, now, &mut fx),
        EventKind::BaseAdvanced { .. } => {}
        EventKind::OpDone { run_id, op, result } => {
            op_done(&mut state, &run_id, op, result, now, &mut fx)
        }
        EventKind::Signal { window_id, signal } => {
            signals::on_signal(&mut state, window_id, signal, now, &mut fx)
        }
        EventKind::Delivered {
            run_id,
            message_ids,
            ok,
            error,
        } => {
            if let Some(run) = state.runs.get_mut(&run_id) {
                outbox::delivered(run, &message_ids, ok, error, now);
            }
        }
        EventKind::Restore { runs, replay } => {
            requests::restore(&mut state, runs, now);
            for (run_id, op, result) in replay {
                op_done(&mut state, &run_id, op, result, now, &mut fx);
            }
        }
        EventKind::Stop => {
            state.stopped = true;
            return (state, fx);
        }
        EventKind::Tick => {}
    }
    for run in state.runs.values_mut() {
        dispatch::schedule(run, now, &mut fx);
    }
    finish(&mut state, &before, before_revision, fx)
}

/// Decision 47: a run that changed gets its revision bumped, and the global one with it;
/// a run new to the state keeps the revision it arrived with. A change to a round's
/// counters alone (`last_event`, `tool_calls`, `usage`) is persisted lazily and published
/// as a counter update (decisions 43, 47); any other change is urgent and structural.
/// `Persist` goes first and `Publish` last.
fn finish(
    state: &mut EngineState,
    before: &BTreeMap<String, Run>,
    before_revision: u64,
    fx: Vec<Effect>,
) -> (EngineState, Vec<Effect>) {
    let mut persist = Vec::new();
    let mut structural = false;
    for (id, run) in state.runs.iter_mut() {
        let urgent = match before.get(id) {
            Some(old) if old == run => continue,
            Some(old) => {
                run.revision += 1;
                without_counters(old) != without_counters(run)
            }
            None => true,
        };
        structural |= urgent;
        state.revision += 1;
        persist.push(Effect::Persist {
            run_id: id.clone(),
            urgent,
        });
    }
    let changed = state.revision != before_revision;
    let mut out = persist;
    out.extend(fx);
    if changed {
        out.push(Effect::Publish { structural });
    }
    (std::mem::take(state), out)
}

/// `run` with every round's and task's counters zeroed and its revision fixed, to tell
/// a counter-only change from a structural one.
fn without_counters(run: &Run) -> Run {
    let mut run = run.clone();
    run.revision = 0;
    for task in run.tasks.iter_mut() {
        task.spent_total = Default::default();
    }
    for round in run.tasks.iter_mut().flat_map(|t| t.rounds.iter_mut()) {
        round.last_event = 0;
        round.tool_calls = 0;
        round.usage = Default::default();
    }
    run
}

/// Routes an op's result by the kind of the op it answers. A result for an op the run
/// no longer has pending (stale, or replayed twice) is ignored.
fn op_done(
    state: &mut EngineState,
    run_id: &str,
    op: OpId,
    result: OpResult,
    now: u64,
    fx: &mut Vec<Effect>,
) {
    let Some(run) = state.runs.get_mut(run_id) else {
        return;
    };
    let Some(pending) = run.pending_ops.remove(&op) else {
        return;
    };
    let task = pending
        .task_id
        .as_deref()
        .and_then(|id| run.tasks.iter().position(|t| t.id() == id));
    match (pending.kind, task) {
        (OpKind::CreateRunBranch { .. }, _) => requests::run_branch_done(run, result, now, fx),
        (OpKind::Discard { .. }, _) => requests::discarded(run, result, now),
        (OpKind::PrepareWorktree { from, .. }, Some(i)) => {
            dispatch::worktree_done(run, i, from, result, now, fx)
        }
        (OpKind::CreateWindow { .. }, Some(i)) => {
            dispatch::window_done(run, i, op, result, now, fx)
        }
        (OpKind::PrepareReview { .. }, Some(i)) => dispatch::review_ready(run, i, result, now, fx),
        (OpKind::HandBack { .. }, Some(i)) => holds::handed_back(run, i, result, now, fx),
        (OpKind::AbortMerge { .. }, Some(i)) => holds::merge_aborted(run, i, result, now),
        (OpKind::RemoveWorktree { .. }, Some(i)) => dispatch::removed(run, i, result, now),
        (OpKind::VerifyDone { .. }, Some(i)) => done::checked(run, i, result, now, fx),
        (OpKind::CountCommits { .. }, Some(i)) => done::counted(run, i, result, now, fx),
        (OpKind::DiffSoFar { .. }, Some(i)) => ladder::fresh_diff(run, i, result, now, fx),
        (OpKind::ResumeSession { .. }, Some(i)) => outbox::resumed(run, i, result, now),
        // The other kinds' results are handled by M8a.13 to M8a.15.
        _ => {}
    }
}

/// Allocates the next op id and records the op as pending, then emits it.
pub(crate) fn emit_op(
    run: &mut Run,
    op: OpId,
    task_id: Option<&str>,
    kind: OpKind,
    fx: &mut Vec<Effect>,
) {
    run.pending_ops.insert(
        op,
        PendingOp {
            op,
            task_id: task_id.map(str::to_string),
            kind: kind.clone(),
        },
    );
    fx.push(Effect::Op {
        run_id: run.id.clone(),
        op,
        kind,
    });
}

/// The next op id of `run`, consumed.
pub(crate) fn next_op(run: &mut Run) -> OpId {
    let id = run.next_op;
    run.next_op += 1;
    id
}

#[cfg(test)]
mod tests;
