//! Milestone 9.5 decision 38 (task M9.5.5a, review ruling C1), engine side: the
//! orchestrator's first turn. A launch or relaunch starts its window with no prompt
//! (`orch_window.rs`); the anthrex server's `McpReady` arms the first turn, which
//! `wake::effect` emits as a whole-paste wake-up until its `OrchestratorWoken`; past
//! [`FIRST_TURN_WAIT_SECS`] the run says so and keeps waiting. Pure (design decision 2).

use super::OpKind;
use super::requests::log;
use crate::run::model::Run;

/// How long after a launch the first turn may wait before the run says so.
pub const FIRST_TURN_WAIT_SECS: u64 = 60;

/// Fix round 1, ruling T5a-1: how long after the window's first signal the first turn
/// waits for the server's notice; past it the turn goes anyway (rule 4's retry line
/// covers a tool the session does not have yet). The signal stays required (ruling C1).
pub const MCP_READY_GRACE_SECS: u64 = 30;

/// Milestone 9.5 decision 38: the driver's `McpReady` from `window_id`, the run's
/// orchestrator window, or any window while a fresh launch has none yet (the notice can
/// beat the launch's `Window` result; the driver checked the manager's record of it).
/// Fix round 1 (m4): while a restart of the window is in flight the notice is the old
/// session's (its server outlives the relaunch's reset by a moment) and is ignored; the
/// new session's server sends its own after the restart, and the grace covers a race.
pub(super) fn mcp_ready(run: &mut Run, window_id: u32) {
    let restarting = run
        .pending_ops
        .values()
        .any(|p| matches!(p.kind, OpKind::RestartOrchestrator { .. }));
    let ours = run
        .orch
        .orchestrator
        .as_ref()
        .is_some_and(|o| match o.window_id {
            Some(w) => w == window_id && !restarting,
            None => o.launch_op.is_some(),
        });
    run.orch.mcp_ready |= ours;
}

/// Ruling T5a-1: the driver saw the window, at its current launch, with a signal while
/// the first turn waits; the grace counts from the first such report.
pub(super) fn signalled(run: &mut Run, (window_id, launch): (u32, u64), now: u64) {
    let current = run.orch.orchestrator.as_ref().is_some_and(|o| {
        o.first_turn_pending && o.window_id == Some(window_id) && o.launches == launch
    });
    if current && !super::orch_window::launching(run) && run.orch.first_signal_at.is_none() {
        run.orch.first_signal_at = Some(now);
    }
}

/// Fix round 1 (m1, m2): a restart started a fresh session, which takes the first prompt
/// again; its wait starts now (the relaunch already reset the notice).
pub(super) fn fresh_session(run: &mut Run, now: u64) {
    if let Some(o) = run.orch.orchestrator.as_mut() {
        o.first_turn_pending = true;
    }
    run.orch.first_turn_since.get_or_insert(now);
}

/// Decision 38: after a daemon restart nothing of the old session's server counts (what
/// `run.json` never holds); `run resume`'s relaunch starts the wait again.
pub(super) fn restored(run: &mut Run) {
    run.orch.mcp_ready = false;
    run.orch.first_turn_since = None;
    run.orch.first_turn_late = false;
    run.orch.first_signal_at = None;
}

/// Decision 38: the first turn was pasted; nothing else changes (its notes stay).
pub(super) fn woken(run: &mut Run) {
    if let Some(o) = run.orch.orchestrator.as_mut() {
        o.first_turn_pending = false;
    }
    run.orch.first_turn_since = None;
    run.orch.first_turn_late = false;
    run.orch.first_signal_at = None;
}

/// Decision 38's bound, on every `Tick`: a first turn still waiting
/// [`FIRST_TURN_WAIT_SECS`] after its launch is late; the run says so once, and the
/// snapshot shows `START_PROMPT` while it is. Nothing is pasted. Ruling T5a-1: a
/// signalled window still without the notice [`MCP_READY_GRACE_SECS`] after its first
/// signal gets the first turn anyway, and the run says so once.
pub(super) fn tick(run: &mut Run, now: u64) {
    grace(run, now);
    let pending = run
        .orch
        .orchestrator
        .as_ref()
        .is_some_and(|o| o.first_turn_pending);
    let Some(since) = run.orch.first_turn_since.filter(|_| pending) else {
        return;
    };
    if run.orch.first_turn_late || run.state.is_terminal() || now < since + FIRST_TURN_WAIT_SECS {
        return;
    }
    run.orch.first_turn_late = true;
    let why = match run.orch.mcp_ready {
        true => "the window is not ready yet",
        false => "no MCP ready notice yet",
    };
    let text = format!("first turn still waiting {FIRST_TURN_WAIT_SECS} s after launch: {why}");
    log(run, now, text);
}

/// Ruling T5a-1's release: past the grace the notice counts as come.
fn grace(run: &mut Run, now: u64) {
    let pending = run
        .orch
        .orchestrator
        .as_ref()
        .is_some_and(|o| o.first_turn_pending);
    let Some(at) = run.orch.first_signal_at.filter(|_| pending) else {
        return;
    };
    if run.orch.mcp_ready || run.state.is_terminal() || now < at + MCP_READY_GRACE_SECS {
        return;
    }
    run.orch.mcp_ready = true;
    let text =
        format!("first turn sent without the MCP ready notice after {MCP_READY_GRACE_SECS} s");
    log(run, now, text);
}
