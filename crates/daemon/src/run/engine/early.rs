//! A session's events that arrive before its round knows its window (the followups
//! file's "A session's signals and tool calls before its `CreateWindow` result are
//! lost"). The engine learns a round's window only from the `Window` result, which the
//! driver sends once the op's `done` line is synced; the session's process has started
//! by then, and a fast one can finish its whole turn first. Pure (design decision 2).
//!
//! - **Holding.** A signal for a window no round has, or a worker's or reviewer's tool
//!   call from one, is held under that window while a launch is in flight (a round with
//!   no window, not ended, whose `CreateWindow` is pending in a run that has not ended).
//!   With none in flight, a signal is dropped and a call refused, as before. A call is
//!   held only when its own task has a round of its role in flight. Which launch the
//!   window belongs to is not guessed: the window is created by an op already emitted,
//!   so it is one of the launches in flight when its first event is held, which the
//!   entry records.
//! - **Replay.** When a `Window` result binds the window to a round, its events are
//!   applied in their original order with their original `now`, right after
//!   `dispatch::window_done`: a held `task_done` needs the `working` state that
//!   `window_done` sets. A replayed call is answered as any call would be then.
//! - **Discarding.** Every step, an entry drops the launches that are no longer in
//!   flight (failed, ended, dropped with their run or task). An entry left with none, or
//!   held [`HOLD_LIMIT_SECS`], is discarded: its signals are dropped and each call gets
//!   the answer it gets unheld, the refusal of a window no round has.
//! - **The cap.** A window holds at most [`HOLD_CAP`] events. Past it, the oldest held
//!   `Activity` or `ToolUse` makes room: losing one costs a counter and a `last_event`
//!   that a later event refreshes. With none to evict, a new signal is dropped, as it
//!   was before the hold, and a new call is answered at once, unheld.
//! - **Bounds on a held reply.** A held call is answered by the replay, by the discard
//!   of its entry (its launch failed or ended, or [`HOLD_LIMIT_SECS`] passed; the
//!   driver's one-second `Tick` steps the engine), or with "the daemon is shutting down"
//!   when the driver stops. [`HOLD_LIMIT_SECS`] stays well under the MCP server's
//!   `TOOL_REPLY_TIMEOUT` (100 s), so the caller is answered before it gives up.
//! - **Restart.** `EngineState::pending` is in memory only. After a daemon restart it is
//!   empty, which is right: the sessions whose events it held died with the daemon, and
//!   decision 48's reconcile relaunches the rounds whose windows were lost.

use proto::{AgentRole, ToolCall};

use super::{AgentSignal, Effect, EngineState, OpKind, ReplyId, done, signals};
use crate::run::model::{AgentRound, OpId, Run};

/// The most events one window holds.
pub const HOLD_CAP: usize = 256;

/// How long a window's first held event waits for its launch, in the reducer's unix
/// seconds.
pub const HOLD_LIMIT_SECS: u64 = 30;

/// One held event.
#[derive(Debug, Clone, PartialEq)]
pub enum HeldEvent {
    Signal(AgentSignal),
    Tool { reply: ReplyId, call: ToolCall },
}

/// A window's held events, oldest first, each with the `now` it arrived at.
#[derive(Debug, Clone, PartialEq)]
pub struct HeldWindow {
    /// When the first event was held.
    pub since: u64,
    /// The launches that were in flight then, `(run, op)`: the window is one of theirs.
    pub launches: Vec<(String, OpId)>,
    pub events: Vec<(HeldEvent, u64)>,
}

/// A round whose `CreateWindow` is in flight in `run`.
fn launching(run: &Run, round: &AgentRound) -> bool {
    !run.state.is_terminal()
        && round.window_id.is_none()
        && !round.ended
        && run
            .pending_ops
            .get(&round.launch_op)
            .is_some_and(|p| matches!(p.kind, OpKind::CreateWindow { .. }))
}

fn rounds(run: &Run) -> impl Iterator<Item = &AgentRound> {
    run.tasks.iter().flat_map(|t| t.rounds.iter())
}

/// Every launch in flight, `(run, op)`.
fn in_flight(state: &EngineState) -> Vec<(String, OpId)> {
    state
        .runs
        .values()
        .flat_map(|run| {
            rounds(run)
                .filter(|r| launching(run, r))
                .map(|r| (run.id.clone(), r.launch_op))
        })
        .collect()
}

fn still_in_flight(state: &EngineState, (run_id, op): &(String, OpId)) -> bool {
    state
        .runs
        .get(run_id)
        .is_some_and(|run| rounds(run).any(|r| r.launch_op == *op && launching(run, r)))
}

fn bound(state: &EngineState, window: u32) -> bool {
    state
        .runs
        .values()
        .any(|run| rounds(run).any(|r| r.window_id == Some(window)))
}

/// Counter-only signals, the first to make room past the cap.
fn evictable(event: &HeldEvent) -> bool {
    matches!(
        event,
        HeldEvent::Signal(AgentSignal::Activity | AgentSignal::ToolUse { .. })
    )
}

/// Holds `event` for `window` if a launch is in flight; gives it back otherwise, or
/// when the window is full with nothing to evict.
fn hold(state: &mut EngineState, window: u32, event: HeldEvent, now: u64) -> Option<HeldEvent> {
    let launches = in_flight(state);
    if launches.is_empty() {
        return Some(event);
    }
    let held = state.pending.entry(window).or_insert_with(|| HeldWindow {
        since: now,
        launches,
        events: Vec::new(),
    });
    if held.events.len() >= HOLD_CAP {
        match held.events.iter().position(|(e, _)| evictable(e)) {
            Some(k) => {
                held.events.remove(k);
            }
            None => return Some(event),
        }
    }
    held.events.push((event, now));
    None
}

/// `signals::on_signal` found no round with `window`: held while a launch is in
/// flight, else dropped.
pub(super) fn hold_signal(state: &mut EngineState, window: u32, signal: AgentSignal, now: u64) {
    let _dropped = hold(state, window, HeldEvent::Signal(signal), now);
}

/// Whether `call` waits for its window: a worker or reviewer tool from a window no
/// round has, whose task has a round of the caller's role in flight.
pub(super) fn holds_call(state: &EngineState, call: &ToolCall) -> bool {
    let known = matches!(
        (call.role, call.tool.as_str()),
        (AgentRole::Worker, "task_done" | "task_blocked") | (AgentRole::Reviewer, "submit_review")
    );
    if !known || bound(state, call.window_id) {
        return false;
    }
    let Some(run) = state.runs.get(&call.run_id) else {
        return false;
    };
    let Some(task) = call.task_id.as_deref().and_then(|id| run.task(id)) else {
        return false;
    };
    task.rounds
        .iter()
        .any(|r| r.role == call.role && launching(run, r))
}

/// Holds `call` (which [`holds_call`] accepted), or answers it at once when its window
/// is full.
pub(super) fn hold_call(
    state: &mut EngineState,
    reply: ReplyId,
    call: ToolCall,
    now: u64,
    fx: &mut Vec<Effect>,
) {
    let window = call.window_id;
    if let Some(HeldEvent::Tool { reply, call }) =
        hold(state, window, HeldEvent::Tool { reply, call }, now)
    {
        done::answer(state, reply, call, now, fx);
    }
}

/// A `Window` result bound `window`: its held events, in order, with their own `now`.
pub(super) fn replay(state: &mut EngineState, window: u32, fx: &mut Vec<Effect>) {
    if !bound(state, window) {
        return;
    }
    let Some(held) = state.pending.remove(&window) else {
        return;
    };
    for (event, at) in held.events {
        match event {
            HeldEvent::Signal(signal) => signals::on_signal(state, window, signal, at, fx),
            HeldEvent::Tool { reply, call } => done::answer(state, reply, call, at, fx),
        }
    }
}

/// Every step: discards the entries whose launches are all over, or that were held too
/// long, answering their calls unheld.
pub(super) fn sweep(state: &mut EngineState, now: u64, fx: &mut Vec<Effect>) {
    if state.pending.is_empty() {
        return;
    }
    for (window, mut held) in std::mem::take(&mut state.pending) {
        held.launches.retain(|l| still_in_flight(state, l));
        let expired = now >= held.since.saturating_add(HOLD_LIMIT_SECS);
        if !held.launches.is_empty() && !expired {
            state.pending.insert(window, held);
            continue;
        }
        for (event, _) in held.events {
            if let HeldEvent::Tool { reply, call } = event {
                done::answer(state, reply, call, now, fx);
            }
        }
    }
}
