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
//! - **The cap.** At most [`HOLD_WINDOWS_CAP`] windows hold events; a new window's
//!   events past it are dropped or answered at once, as before the hold. A window
//!   holds at most [`HOLD_CAP`] events. Past it, the oldest held `Activity`,
//!   `ToolUse` or `Said` makes room: losing one costs a counter and a `last_event`
//!   that a later event refreshes. Milestone 9.0.5's unthrottled `Said` is
//!   counter-only too, so a burst of text never crowds out a `TurnEnded`. An evicted
//!   `Said` is not refreshed by later events, though: its text is lost as `activity`
//!   and `last_text` unless a later `Said` replaces it. That happens only past
//!   [`HOLD_CAP`] events before a launch binds. With none to evict, a new signal is
//!   dropped, as it was before the hold, and a new call is answered at once, unheld.
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
use crate::run::design::state::DesignAgentState;
use crate::run::model::{AgentRound, OpId, Run};
use crate::run::orch::{EpicRecord, PlannerPhase, PlannerSession};

/// The most events one window holds.
pub const HOLD_CAP: usize = 256;

/// At most this many windows hold events at once. The driver forwards every window's
/// signals, a run's or not, so a burst from many unrelated windows is dropped past it,
/// as every early event was before the hold.
pub const HOLD_WINDOWS_CAP: usize = 64;

/// How long a window's first held event waits for its launch, in the reducer's unix
/// seconds.
pub const HOLD_LIMIT_SECS: u64 = 30;

/// One held event.
#[derive(Debug, Clone, PartialEq)]
pub enum HeldEvent {
    Signal(AgentSignal),
    Tool {
        reply: ReplyId,
        call: ToolCall,
    },
    /// Milestone 9 task M9.8: a sub-planner's `submit_epic` before its `StartPlanner`
    /// result named its window.
    Orch {
        reply: ReplyId,
        call: ToolCall,
        refusals: Vec<(proto::Runtime, String)>,
    },
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

/// A sub-planner session whose `StartPlanner` is in flight in `run` (task M9.8): its
/// epic is planning, and the session has no window and has not ended.
fn planner_launching(run: &Run, epic: &EpicRecord, session: &PlannerSession) -> bool {
    !run.state.is_terminal()
        && epic.phase == PlannerPhase::Planning
        && session.window_id.is_none()
        && session.ended_at.is_none()
        && session.op.is_some_and(|op| {
            run.pending_ops
                .get(&op)
                .is_some_and(|p| matches!(p.kind, OpKind::StartPlanner { .. }))
        })
}

/// Every sub-planner launch in flight in `run`, by op.
fn planner_launches(run: &Run) -> impl Iterator<Item = OpId> + '_ {
    run.orch.epics.iter().flat_map(move |e| {
        e.sessions
            .iter()
            .filter(move |s| planner_launching(run, e, s))
            .filter_map(|s| s.op)
    })
}

/// Milestone 9.6: every design agent launch in flight in `run`, by op: a pending
/// `StartDesignAgent` whose agent runs with no window yet.
fn design_launches(run: &Run) -> impl Iterator<Item = OpId> + '_ {
    let design = run.orch.design.as_ref();
    let agents = design
        .into_iter()
        .flat_map(|d| d.brainstormers.iter().chain(&d.reviewer));
    let waiting = move |label: String, session: u32| {
        agents.clone().any(|a| {
            a.label == label
                && a.session == session
                && a.window_id.is_none()
                && a.state == DesignAgentState::Running
        })
    };
    (run.pending_ops.values())
        .filter(move |_| !run.state.is_terminal())
        .filter_map(move |p| match &p.kind {
            OpKind::StartDesignAgent { spec } => {
                waiting(spec.kind.label(), spec.session).then_some(p.op)
            }
            _ => None,
        })
}

/// The design agents' windows in `run`.
fn design_windows(run: &Run) -> impl Iterator<Item = u32> + '_ {
    let design = run.orch.design.as_ref();
    (design.into_iter())
        .flat_map(|d| d.brainstormers.iter().chain(&d.reviewer))
        .filter_map(|a| a.window_id)
}

/// Every launch in flight, `(run, op)`.
fn in_flight(state: &EngineState) -> Vec<(String, OpId)> {
    state
        .runs
        .values()
        .flat_map(|run| {
            rounds(run)
                .filter(|r| launching(run, r))
                .map(|r| r.launch_op)
                .chain(planner_launches(run))
                .chain(design_launches(run))
                .map(|op| (run.id.clone(), op))
                .collect::<Vec<_>>()
        })
        .collect()
}

fn still_in_flight(state: &EngineState, (run_id, op): &(String, OpId)) -> bool {
    state.runs.get(run_id).is_some_and(|run| {
        rounds(run).any(|r| r.launch_op == *op && launching(run, r))
            || planner_launches(run).any(|p| p == *op)
            || design_launches(run).any(|p| p == *op)
    })
}

/// A round's, or a sub-planner session's, window.
fn bound(state: &EngineState, window: u32) -> bool {
    state.runs.values().any(|run| {
        rounds(run).any(|r| r.window_id == Some(window))
            || run
                .orch
                .epics
                .iter()
                .flat_map(|e| e.sessions.iter())
                .any(|s| s.window_id == Some(window))
            || design_windows(run).any(|w| w == window)
    })
}

/// Counter-only signals, the first to make room past the cap.
fn evictable(event: &HeldEvent) -> bool {
    matches!(
        event,
        HeldEvent::Signal(
            AgentSignal::Activity | AgentSignal::ToolUse { .. } | AgentSignal::Said { .. }
        )
    )
}

/// Holds `event` for `window` if a launch is in flight; gives it back otherwise, or
/// when the window is full with nothing to evict.
fn hold(state: &mut EngineState, window: u32, event: HeldEvent, now: u64) -> Option<HeldEvent> {
    let launches = in_flight(state);
    if launches.is_empty() {
        return Some(event);
    }
    if !state.pending.contains_key(&window) && state.pending.len() >= HOLD_WINDOWS_CAP {
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
        (AgentRole::Worker | AgentRole::TestWriter | AgentRole::Racer, "task_done" | "task_blocked")
            // Milestone 9 decision 42f: a worker's note joins the hold (task M9.13a).
            | (AgentRole::Worker | AgentRole::TestWriter | AgentRole::Racer, "task_note")
            | (AgentRole::Reviewer, "submit_review")
            // Milestone 9 decision 35: a research task's report joins the hold.
            | (AgentRole::Scout, "submit_scout_report")
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

/// Whether a sub-planner's `submit_epic` waits for its window (task M9.8, joining
/// PR #22's hold so the planner's one write cannot be lost): from a window no session
/// has, naming an epic whose latest session is being launched.
pub(crate) fn holds_planner_call(state: &EngineState, call: &ToolCall) -> bool {
    let write = matches!(
        (call.role, call.tool.as_str()),
        (AgentRole::Planner, "submit_epic")
            // Milestone 9.6 (task 6's review c): a design agent's one write.
            | (AgentRole::Brainstormer, "submit_doc")
            | (AgentRole::DocReviewer, "submit_findings")
    );
    write && awaits_launch(state, call)
}

/// Task M9.11 (review finding 1): whether `call`, a sub-planner's or the
/// orchestrator's, comes from a window nothing has yet while the launch it may belong
/// to is in flight: its epic's latest session's `StartPlanner` (the rule
/// [`holds_planner_call`] holds a `submit_epic` by), or the run's `CreateOrchestrator`.
/// The driver makes such a call wait, at most [`HOLD_LIMIT_SECS`], then checks its
/// caller as usual: the reads of both roles and the orchestrator's writes (a
/// sub-planner's `submit_epic` is held here instead).
pub(crate) fn awaits_launch(state: &EngineState, call: &ToolCall) -> bool {
    if bound(state, call.window_id) {
        return false;
    }
    let Some(run) = state.runs.get(&call.run_id) else {
        return false;
    };
    match call.role {
        AgentRole::Planner => run
            .orch
            .epics
            .iter()
            .find(|e| Some(&e.epic) == call.epic.as_ref())
            .and_then(|e| e.sessions.last().map(|s| planner_launching(run, e, s)))
            .unwrap_or(false),
        // Milestone 9.6: a launch of a design agent of its role is in flight.
        AgentRole::Brainstormer | AgentRole::DocReviewer => design_launches(run).any(|op| {
            let kind = run.pending_ops.get(&op).map(|p| &p.kind);
            matches!(kind, Some(OpKind::StartDesignAgent { spec }) if spec.kind.role() == call.role)
        }),
        AgentRole::Orchestrator => run.orch.orchestrator.as_ref().is_some_and(|o| {
            !run.state.is_terminal()
                && o.window_id != Some(call.window_id)
                && o.launch_op.is_some_and(|op| {
                    run.pending_ops
                        .get(&op)
                        .is_some_and(|p| matches!(p.kind, OpKind::CreateOrchestrator { .. }))
                })
        }),
        _ => false,
    }
}

/// Holds a sub-planner's call ([`holds_planner_call`] accepted it), or answers it at
/// once when its window is full.
pub(super) fn hold_planner_call(
    state: &mut EngineState,
    reply: ReplyId,
    call: ToolCall,
    refusals: Vec<(proto::Runtime, String)>,
    now: u64,
    fx: &mut Vec<Effect>,
) {
    let event = HeldEvent::Orch {
        reply,
        call,
        refusals,
    };
    if let Some(event) = hold(state, event.window(), event, now) {
        answer_held(state, event, now, fx);
    }
}

impl HeldEvent {
    fn window(&self) -> u32 {
        match self {
            HeldEvent::Orch { call, .. } | HeldEvent::Tool { call, .. } => call.window_id,
            HeldEvent::Signal(_) => 0,
        }
    }
}

/// A held call answered as it would be now; a signal is dropped.
fn answer_held(state: &mut EngineState, event: HeldEvent, now: u64, fx: &mut Vec<Effect>) {
    match event {
        HeldEvent::Signal(_) => {}
        HeldEvent::Tool { reply, call } => done::answer(state, reply, call, now, fx),
        HeldEvent::Orch {
            reply,
            call,
            refusals,
        } => super::orch::tool(state, reply, &call, &refusals, now, fx),
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
            call => answer_held(state, call, at, fx),
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
            answer_held(state, event, now, fx);
        }
    }
}
