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
//! budgets).
//!
//! M8a.13 adds `gates.rs` (the test proof, the check, the gate order and `run
//! override`) and `review.rs` (the review gate and its reviewer sessions).
//!
//! M8a.14 adds `merge.rs` (decision 36's merge queue and hand-back, decision 21's ref
//! guard and `run resume --rebaseline`) and `complete.rs` (decision 37's completion,
//! the `finish` edit, `run cancel`, and `run accept`/`discard` of a complete run).
//! M8a.15 adds `restore.rs` (restore after a daemon restart, `run resume` and the
//! launches a restart lost), `run retry` in `requests.rs`, and the rest of `run
//! override` in `gates.rs`.
//!
//! M8b.12 adds `deciders.rs` (M8b decisions 18, 20 and 21: decider ops in reader
//! slots, the check summary a failed check's rung waits for, and the classification of
//! a `task_blocked` with no kind).
//!
//! M8b.16 adds `history.rs` (M8b decisions 32 and 33: a task's diff and the records of
//! `history.jsonl`, as journaled ops).

use std::collections::BTreeMap;

use proto::{FinishAction, PlanEdit, TokenUsage, ToolCall};

use super::model::{OpId, PendingOp, Run};
use super::validate::EditScope;

mod batch;
mod clock;
mod complete;
pub(crate) mod deciders;
mod deciders_size;
mod dispatch;
mod done;
pub(crate) mod early;
mod effect;
mod fallback;
// Milestone 9.1 decisions 17-19: tier 3 (`request` is 9.2's entry).
pub(crate) mod full;
mod gate_holds;
mod gates;
mod history;
mod holds;
mod integration;
mod kinds;
pub(crate) mod ladder;
mod merge;
mod ops;
mod orch;
mod orch_window;
mod outbox;
mod planners;
mod promote;
mod requests;
mod research;
mod restore;
mod results;
mod review;
mod rounds;
mod run_scouts;
pub(crate) mod schedule;
mod signals;
pub(crate) mod stages;
mod tiers;
mod tools;
mod wake;
mod worker_messages;

pub use crate::headless::TurnOutcome;
pub(crate) use clock::epoch_spend;
pub use clock::{BudgetEpoch, TaskClock};
pub use early::{HOLD_CAP, HOLD_LIMIT_SECS, HOLD_WINDOWS_CAP, HeldEvent, HeldWindow};
pub use effect::Effect;
pub(crate) use full::attention as full_attention;
pub use history::HISTORY_FILE;
pub(crate) use integration::attention as integration_attention;
pub use ops::{OpKind, OpResult, OverrideCount, ResolutionAt, ScratchAt};
pub use orch::{OrchEvent, ScoutEnd};
pub use signals::INTERRUPT_GRACE_SECS;
pub use stages::Rebaseline;
pub use wake::notes_seq;

/// Identifies a client request waiting for its [`Effect::Reply`].
pub type ReplyId = u64;

/// Everything the engine knows. `revision` is decision 47's global counter, bumped with
/// every run's own.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct EngineState {
    pub runs: BTreeMap<String, Run>,
    pub revision: u64,
    pub stopped: bool,
    /// The events of windows no round has yet, held while a launch is in flight
    /// (`early.rs`). In memory only: never persisted, empty after a restart.
    pub pending: BTreeMap<u32, HeldWindow>,
    /// The run as the orchestrator's own tool call left it, before its handler's
    /// scheduler pass (`orch::settle_quiet`); taken by the step that set it.
    pub quiet_base: Option<Run>,
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
        /// Ruling T22-I1b: decision 50's or 53's refusal for each runtime the run could
        /// not reach when the request came in and whose checks fail; an edit that makes
        /// one of them reachable is refused with its text.
        refusals: Vec<(proto::Runtime, String)>,
        /// Milestone 9 decision 13: the user submits a planning run's plan after the
        /// batch (M9.7 review fixes, ruling 5).
        submit: bool,
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
    /// `rebaseline`: the refs the driver read (milestone 9.1: every stage head too).
    Resume {
        reply: ReplyId,
        run_id: String,
        rebaseline: Option<Rebaseline>,
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
    /// M8b decision 25: `run promote`, which milestone 9 performs (decision 29) with the
    /// orchestrator the user chose, if any.
    Promote {
        reply: ReplyId,
        run_id: String,
        orchestrator: Option<proto::OrchestratorChoice>,
    },
    /// M8b decision 30: the OTLP ledger's new total for `(run, "orchestrator")`. It
    /// replaces the one before, on top of the usage restored at the daemon's start.
    OrchestratorUsage {
        run_id: String,
        usage: TokenUsage,
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
        /// Ops kept pending with no answer yet: the driver answers them later with an
        /// ordinary `OpDone` (ruling T22-N3: a replayed accept's clean-up, run once the
        /// socket is bound).
        held: Vec<(String, OpId)>,
    },
    Stop,
    Tick,
    /// Milestone 9: the orchestrator's and sub-planners' events (`orch.rs`).
    Orch(OrchEvent),
}

/// The driver's translation of a window's session events (decision 27).
///
/// **Ordering contract** (ruling T13-P1), which `signals::apply` relies on and the
/// session driver guarantees (`headless::session`'s module doc, carried on the manager's
/// feed as `WindowSignal.pid`):
///
/// - every signal of a process carries, or is sent for, that process's pid;
/// - `ProcessStarted { pid }` for a new process is delivered before any other signal of
///   that process;
/// - a process's `ProcessExited` is delivered after its last stream signal, including a
///   `TurnEnded { Failed { SandboxUnavailable } }` the driver synthesises from stderr
///   before `Init` (M8a.12's carry).
///
/// Signals of two different processes of one window (a Codex turn's process and the
/// next, a killed Claude process and its `--resume`) may interleave; the pid tells them
/// apart. M8a.22's driver forwards the feed in the order it receives it.
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
    /// The usage of a turn Claude Code started by itself while a delivered turn waits
    /// (`WindowSignalKind::Unprompted`, ruling T7-N1): spend, never a turn end.
    Spend {
        usage: TokenUsage,
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
    // Milestone 9 decision 39: the orchestrator's own edits add no wake note.
    let quiet = matches!(&event.kind, EventKind::Orch(OrchEvent::Tool { call, .. })
        if call.role == proto::AgentRole::Orchestrator);
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
            refusals,
            submit,
        } => requests::edit(
            &mut state,
            reply,
            &run_id,
            (&edits, &scope, &refusals, submit),
            now,
            &mut fx,
        ),
        EventKind::Retry {
            reply,
            run_id,
            task_id,
        } => requests::retry(&mut state, reply, &run_id, &task_id, now, &mut fx),
        EventKind::Override {
            reply,
            run_id,
            task_id,
            reason,
        } => gates::override_task(&mut state, reply, &run_id, &task_id, &reason, now, &mut fx),
        EventKind::Cancel { reply, run_id } => {
            complete::cancel(&mut state, reply, &run_id, now, &mut fx)
        }
        EventKind::Resume {
            reply,
            run_id,
            rebaseline,
        } => restore::resume(&mut state, reply, &run_id, rebaseline, now, &mut fx),
        EventKind::Finish {
            reply,
            run_id,
            action,
        } => complete::finish(&mut state, reply, &run_id, action, now, &mut fx),
        EventKind::Tool { reply, call } => done::tool(&mut state, reply, call, now, &mut fx),
        EventKind::Promote {
            reply,
            run_id,
            orchestrator,
        } => promote::request(&mut state, reply, &run_id, orchestrator, now, &mut fx),
        EventKind::Orch(event) => orch::on_orch_event(&mut state, event, now, &mut fx),
        EventKind::BaseAdvanced {
            run_id,
            to,
            commits,
        } => merge::base_advanced(&mut state, &run_id, to, commits, now),
        EventKind::OrchestratorUsage { run_id, usage } => {
            // A run that ended keeps the usage it ended with (M8b.15 re-review): a
            // total that raced its end is dropped.
            let open = state
                .runs
                .get_mut(&run_id)
                .filter(|r| !r.state.is_terminal());
            if let Some(run) = open {
                run.orchestrator_usage = run.orchestrator_base;
                run.orchestrator_usage += usage;
            }
        }
        EventKind::OpDone { run_id, op, result } => {
            results::op_done(&mut state, &run_id, op, result, now, &mut fx)
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
        EventKind::Restore { runs, replay, held } => {
            restore::restore(&mut state, runs, (replay, held), now, &mut fx)
        }
        EventKind::Stop => {
            state.stopped = true;
            return (state, fx);
        }
        // Milestone 9 decision 29: a promotion recorded before milestone 9.
        EventKind::Tick => promote::on_tick(&mut state, now, &mut fx),
    }
    // M9.9 review fixes, I2 and M-b: an orchestrator call's own blocks are compared
    // away (the run before its handler's scheduler pass, `quiet_base`); what the
    // scheduler's passes block, a stall among them, is still noted.
    let base = state.quiet_base.take();
    let applied = quiet.then(|| {
        let mut runs = state.runs.clone();
        if let Some(base) = base {
            runs.insert(base.id.clone(), base);
        }
        runs
    });
    for (id, run) in state.runs.iter_mut() {
        dispatch::schedule(run, now, &mut fx);
        orch_window::ended(run);
        gate_holds::drop_empty_rounds(run, now);
        // M8b decision 33: the history records that are due, whatever the run's state.
        history::pass(run, now, &mut fx);
        // Milestone 9 decision 39: a note for each task this step blocked.
        let since = applied.as_ref().unwrap_or(&before);
        wake::blocked_notes(since.get(id), run);
    }
    // Held events whose launches are over, or that waited too long (`early.rs`).
    early::sweep(&mut state, now, &mut fx);
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
    let mut wakes = Vec::new();
    let mut structural = false;
    for (id, run) in state.runs.iter_mut() {
        let urgent = match before.get(id) {
            Some(old) if old == run => continue,
            Some(old) => {
                run.revision += 1;
                let urgent = without_counters(old) != without_counters(run);
                // Decision 16: the digest's revision moves only with its fingerprint. A
                // run new to the state (a restore) is left as loaded (final review B-5).
                if urgent {
                    super::orch::digest::note_change(run);
                }
                // Decision 39: a wake-up is due when the orchestrator should see its
                // notes.
                wakes.extend(wake::effect(run));
                urgent
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
    out.extend(wakes);
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
    // M8b decision 29: usage from OTLP and run scouts is a counter too.
    run.orchestrator_usage = Default::default();
    run.scout_usage = Default::default();
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
