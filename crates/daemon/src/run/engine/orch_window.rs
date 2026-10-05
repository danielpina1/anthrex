//! The orchestrator window's lifecycle, engine side (milestone 9 decisions 5, 11 and
//! 26): launched with a planned run or a promotion, counted toward `max_windows`,
//! restored dormant after a daemon restart and restarted by `run resume` with a new
//! session. Pure (design decision 2).

use proto::AgentRole;

use super::requests::log;
use super::{Effect, OpId, OpKind, OpResult, emit_op, history, next_op};
use crate::run::model::Run;
use crate::run::orch::contract::orchestrator_first_prompt;
use crate::run::orch::contract_design::{session_prompt, where_the_run_is};
use crate::run::orch::launch::{first_turn_pasted, orchestrator_role, orchestrator_window_spec};

/// Decisions 5 and 26: the orchestrator's window, and its first prompt (the planned
/// run's, unless a promotion or a chain set its own). Decision 43: its session's record
/// is opened first, `start` (`promote` for a promotion), or `retry` after an earlier
/// session. Milestone 9.5 decision 38: a Claude window starts with no prompt; the first
/// prompt waits as the first turn ([`first_turn_waits`]). Ruling T5a-2: a Codex window
/// has it on its command line, as in 9.3.
pub(super) fn launch(run: &mut Run, now: u64, fx: &mut Vec<Effect>) {
    let Some(o) = run.orch.orchestrator.as_ref() else {
        return;
    };
    let route = o.route.clone();
    let first = if o.first_prompt.is_empty() {
        orchestrator_first_prompt(run)
    } else {
        o.first_prompt.clone()
    };
    // Ruling T14-1: a design run's session also gets where the run is, built now.
    let spec = orchestrator_window_spec(run, &route, &session_prompt(run, &first));
    let role = orchestrator_role(run, &route);
    let earlier = run
        .role_routing_decisions
        .iter()
        .any(|d| d.role == AgentRole::Orchestrator);
    let trigger = match (earlier, run.promote_requested_at.is_some()) {
        (true, _) => "retry",
        (false, true) => "promote",
        (false, false) => "start",
    };
    history::orchestrator_dispatched(run, trigger, now);
    let op = next_op(run);
    if let Some(o) = run.orch.orchestrator.as_mut() {
        o.first_prompt = first;
        o.launch_op = Some(op);
        o.first_turn_pending = first_turn_pasted(&route);
    }
    first_turn_waits(run, now);
    let kind = OpKind::CreateOrchestrator {
        spec: Box::new(spec),
        role: Box::new(role),
        project: run.project.clone(),
    };
    emit_op(run, op, None, kind, fx);
}

/// `CreateOrchestrator`'s result: the window counts toward `max_windows` (decision 5).
pub(super) fn launched(run: &mut Run, op: OpId, result: OpResult, now: u64, fx: &mut Vec<Effect>) {
    let Some(o) = run.orch.orchestrator.as_mut() else {
        return;
    };
    if o.launch_op != Some(op) {
        return;
    }
    o.launch_op = None;
    match result {
        OpResult::Window { window_id, .. } => {
            o.window_id = Some(window_id);
            o.live = true;
            o.exited_at = None;
            o.start_error = None;
            o.launches += 1;
            run.windows_created += 1;
            own_session(run);
            log(
                run,
                now,
                format!("the orchestrator started in window {window_id}"),
            );
        }
        OpResult::Failed { message } => {
            // Decision 13: an attention line too, until a launch succeeds.
            o.start_error = Some(message.clone());
            let text = format!("the orchestrator could not start: {message}");
            history::orchestrator_ended(run, Some(&text), fx);
            log(run, now, text);
        }
        _ => {}
    }
}

/// Decision 13: the driver saw the orchestrator's window exit, or come back after an
/// exit (the user's `anthrex restart`). An exit suspends wake-ups (the record is not
/// live) and shows the attention line; the run goes on. A report about any other
/// window, a return on a finished run, or a report the driver made before the last
/// launch or restart (`launch`, M9.13 review) changes nothing. Decision 43: an exit
/// ends the session's record; a return is a new session, `restart`.
pub(super) fn window_seen(
    run: &mut Run,
    (window_id, launch): (u32, u64),
    (live, now): (bool, u64),
    fx: &mut Vec<Effect>,
) {
    let terminal = run.state.is_terminal();
    let Some(o) = run.orch.orchestrator.as_mut() else {
        return;
    };
    let stale = o.launches != launch;
    if o.window_id != Some(window_id) || o.live == live || (live && terminal) || stale {
        return;
    }
    o.live = live;
    if live {
        o.exited_at = None;
        history::orchestrator_dispatched(run, "restart", now);
        log(
            run,
            now,
            format!("the orchestrator's window {window_id} is back"),
        );
    } else {
        o.exited_at = Some(now);
        history::orchestrator_ended(run, None, fx);
        log(
            run,
            now,
            format!("the orchestrator's window {window_id} exited"),
        );
    }
}

/// Decisions 11 and 39: a restarted orchestrator's first wake-up note.
const RESUMED_NOTE: &str = "the daemon restarted and your session was resumed";

/// `RestartOrchestrator`'s result.
pub(super) fn restarted(run: &mut Run, result: OpResult, now: u64, fx: &mut Vec<Effect>) {
    let Some(o) = run.orch.orchestrator.as_mut() else {
        return;
    };
    match result {
        OpResult::Restarted | OpResult::RestartedFresh => {
            o.live = true;
            o.exited_at = None;
            o.launches += 1;
            let pasted = first_turn_pasted(&o.route);
            own_session(run);
            log(run, now, "the orchestrator restarted");
            // Milestone 9.5 fix round 1 (m1, m2): a fresh session takes the first
            // prompt again, and was not resumed. Ruling T5a-2: Claude's only; a Codex
            // restart is 9.3's (its window's spec carries the first prompt).
            if result == OpResult::RestartedFresh && pasted {
                super::first_turn::fresh_session(run, now);
                super::wake::unnote(run, RESUMED_NOTE);
            }
            // Milestone 9.6 review focus 1 (fix round 1): any fresh session, Claude's or
            // Codex's, is told its revision again.
            if result == OpResult::RestartedFresh {
                super::design_gate::renote(run);
                // Ruling T14-1: a Codex window restarts on its launch's command line, so
                // its fresh session is told where the run is now.
                if let Some(line) = where_the_run_is(run).filter(|_| !pasted) {
                    super::wake::note(run, line);
                }
            }
        }
        OpResult::Failed { message } => {
            let text = format!("the orchestrator could not restart: {message}");
            history::orchestrator_ended(run, Some(&text), fx);
            log(run, now, text);
            // The final fix wave (review A, M3): the window was gone, and the driver's
            // `AdoptLost` took it off the record with the run's handoff as its first
            // prompt; the session starts fresh at once.
            if run
                .orch
                .orchestrator
                .as_ref()
                .is_some_and(|o| o.window_id.is_none())
            {
                // W1 fix round 2: the restart's note is untrue of a fresh session.
                super::wake::unnote(run, RESUMED_NOTE);
                relaunch(run, now, fx);
            }
        }
        _ => {}
    }
}

/// Whether a `CreateOrchestrator` or `RestartOrchestrator` is in flight.
pub(super) fn launching(run: &Run) -> bool {
    run.pending_ops.values().any(|p| {
        matches!(
            p.kind,
            OpKind::CreateOrchestrator { .. } | OpKind::RestartOrchestrator { .. }
        )
    })
}

/// Decision 11: a resumed run's dormant orchestrator (restored after a daemon restart,
/// or exited) restarts in its window with its role and a new session; one whose window
/// was never made is launched again. Returns whether anything was issued.
pub(super) fn relaunch(run: &mut Run, now: u64, fx: &mut Vec<Effect>) -> bool {
    if !relaunchable(run) {
        return false;
    }
    match run.orch.orchestrator.as_ref().and_then(|o| o.window_id) {
        Some(window_id) => {
            if let Some(o) = run.orch.orchestrator.as_mut() {
                o.session += 1;
            }
            first_turn_waits(run, now);
            history::orchestrator_dispatched(run, "restart", now);
            let op = next_op(run);
            emit_op(run, op, None, OpKind::RestartOrchestrator { window_id }, fx);
            log(
                run,
                now,
                format!("restarting the orchestrator in window {window_id}"),
            );
            // Decisions 11, 39: its first wake-up says so.
            super::wake::note(run, RESUMED_NOTE.to_string());
        }
        None => launch(run, now, fx),
    }
    true
}

/// Milestone 9.5 decision 38: a launch or relaunch starts a new anthrex server, whose
/// `McpReady` is still to come; a first turn still pending waits from now.
fn first_turn_waits(run: &mut Run, now: u64) {
    run.orch.mcp_ready = false;
    run.orch.first_turn_late = false;
    run.orch.first_signal_at = None;
    let pending = run
        .orch
        .orchestrator
        .as_ref()
        .is_some_and(|o| o.first_turn_pending);
    run.orch.first_turn_since = pending.then_some(now);
}

/// Milestone 9.0.6 decision 42: whether [`relaunch`] would issue anything, changing
/// nothing: an orchestrator that is not live, of a run that has not ended, with no
/// launch in flight.
pub(super) fn relaunchable(run: &Run) -> bool {
    run.orch.orchestrator.as_ref().is_some_and(|o| !o.live)
        && !run.state.is_terminal()
        && !launching(run)
}

/// After a daemon restart the orchestrator's window is dormant (decision 11).
/// Milestone 9.5 decision 37: the new daemon's OTLP totals count from zero on top of
/// the restored usage, so an adopted session's counter starts again at zero.
pub(super) fn restored(run: &mut Run) {
    super::first_turn::restored(run);
    if let Some(o) = run.orch.orchestrator.as_mut() {
        o.live = false;
        if let Some(at) = o.usage_at_adopt.as_mut() {
            *at = Default::default();
        }
    }
}

/// Milestone 9.5 decision 37 (review I1): the run's session posts under the run's own
/// id from now on (a launch, a lost adoption, or a restart, whose `refresh_otlp` re-keys
/// the window), from zero: what an adopted session was credited becomes the base.
pub(super) fn own_session(run: &mut Run) {
    if let Some(o) = run.orch.orchestrator.as_mut()
        && o.usage_at_adopt.take().is_some()
    {
        run.orchestrator_base = run.orchestrator_usage;
    }
}

/// Decisions 11 and 30: a run that ended (accepted, discarded or failed) has no live
/// orchestrator; its window becomes a plain one. Run after every event, so a
/// `Restarted` or `Window` result that comes back after the end cannot set it again
/// (M9.7 second review, ruling 5).
pub(super) fn ended(run: &mut Run) {
    if run.state.is_terminal()
        && let Some(o) = run.orch.orchestrator.as_mut()
    {
        o.live = false;
    }
}

/// A `CreateOrchestrator` the restart lost: `run resume` launches it again.
pub(super) fn launch_lost(run: &mut Run, op: OpId) {
    if let Some(o) = run.orch.orchestrator.as_mut()
        && o.launch_op == Some(op)
    {
        o.launch_op = None;
    }
}
