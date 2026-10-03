//! Milestone 9.5 decision 38 (task M9.5.5a, review ruling C1), engine side: the
//! orchestrator's first turn. A launch or relaunch starts its window with no prompt
//! (`orch_window.rs`); the anthrex server's `McpReady` arms the first turn, which
//! `wake::effect` emits as a whole-paste wake-up until its `OrchestratorWoken`; past
//! [`FIRST_TURN_WAIT_SECS`] the run says so and keeps waiting. Pure (design decision 2).

use super::requests::log;
use crate::run::model::Run;

/// How long after a launch the first turn may wait before the run says so.
pub const FIRST_TURN_WAIT_SECS: u64 = 60;

/// Milestone 9.5 decision 38: the driver's `McpReady` from `window_id`, the run's
/// orchestrator window, or any window while a fresh launch has none yet (the notice can
/// beat the launch's `Window` result; the driver checked the manager's record of it).
pub(super) fn mcp_ready(run: &mut Run, window_id: u32) {
    let ours = run
        .orch
        .orchestrator
        .as_ref()
        .is_some_and(|o| match o.window_id {
            Some(w) => w == window_id,
            None => o.launch_op.is_some(),
        });
    run.orch.mcp_ready |= ours;
}

/// Decision 38: after a daemon restart nothing of the old session's server counts (what
/// `run.json` never holds); `run resume`'s relaunch starts the wait again.
pub(super) fn restored(run: &mut Run) {
    run.orch.mcp_ready = false;
    run.orch.first_turn_since = None;
    run.orch.first_turn_late = false;
}

/// Decision 38: the first turn was pasted; nothing else changes (its notes stay).
pub(super) fn woken(run: &mut Run) {
    if let Some(o) = run.orch.orchestrator.as_mut() {
        o.first_turn_pending = false;
    }
    run.orch.first_turn_since = None;
    run.orch.first_turn_late = false;
}

/// Decision 38's bound, on every `Tick`: a first turn still waiting
/// [`FIRST_TURN_WAIT_SECS`] after its launch is late; the run says so once, and the
/// snapshot shows `START_PROMPT` while it is. Nothing is pasted.
pub(super) fn tick(run: &mut Run, now: u64) {
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
