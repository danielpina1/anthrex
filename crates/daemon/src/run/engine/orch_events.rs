//! Milestone 9, engine side: the orchestrator's events (`OrchEvent`), dispatched to
//! their handlers. Moved out of `orch.rs` (milestone 9.13) so that file stays focused on
//! the orchestrator's tools. Pure (design decision 2).

use super::orch::OrchEvent;
use super::orch::tool;
use super::{Effect, EngineState, gate_holds, planners, run_scouts, wake};

pub(super) fn on_orch_event(
    state: &mut EngineState,
    event: OrchEvent,
    now: u64,
    fx: &mut Vec<Effect>,
) {
    match event {
        // Milestone 9 decision 42f: a worker's `task_note` passes M8a's run gate and
        // early hold like its other tools (`done.rs`).
        OrchEvent::Tool { reply, call, .. } if crate::run::model::writes_task(call.role) => {
            super::done::tool(state, reply, call, now, fx)
        }
        OrchEvent::Tool {
            reply,
            call,
            refusals,
        } if super::early::holds_planner_call(state, &call) => {
            super::early::hold_planner_call(state, reply, call, refusals, now, fx)
        }
        OrchEvent::Tool {
            reply,
            call,
            refusals,
        } => tool(state, reply, &call, &refusals, now, fx),
        OrchEvent::ApproveHold {
            reply,
            run_id,
            hold,
        } => gate_holds::verdict(state, reply, (&run_id, &hold), true, now, fx),
        OrchEvent::RejectHold {
            reply,
            run_id,
            hold,
        } => gate_holds::verdict(state, reply, (&run_id, &hold), false, now, fx),
        // Milestone 9.9 decision 16: the user's answer to `ask_user`.
        OrchEvent::AnswerAsk {
            reply,
            run_id,
            ask,
            choice,
        } => super::asks::answer(state, reply, (&run_id, ask, choice), now, fx),
        OrchEvent::ScoutEnded {
            run_id,
            scout_id,
            outcome,
            usage,
        } => {
            if let Some(run) = state.runs.get_mut(&run_id) {
                run_scouts::ended(run, &scout_id, (outcome, usage), now, fx);
            }
        }
        OrchEvent::PlannerEnded {
            run_id,
            epic,
            session,
            outcome,
            usage,
        } => {
            if let Some(run) = state.runs.get_mut(&run_id) {
                planners::ended(run, (&epic, session), (outcome, usage), now, fx);
            }
        }
        // Milestone 9.6 decision 9: a design agent's session ended.
        ended @ OrchEvent::DesignAgentEnded { .. } => {
            super::design_agents::on_ended(state, ended, now, fx)
        }
        OrchEvent::OrchestratorWoken {
            run_id,
            digest_revision,
            notes_seq,
            request,
            first_turn,
        } => {
            if let Some(run) = state.runs.get_mut(&run_id) {
                run.orch.wake_held = false;
                match first_turn {
                    true => super::first_turn::woken(run),
                    false => wake::woken(run, digest_revision, notes_seq, request),
                }
            }
        }
        OrchEvent::McpReady { run_id, window_id } => {
            if let Some(run) = state.runs.get_mut(&run_id) {
                super::first_turn::mcp_ready(run, window_id);
            }
        }
        OrchEvent::FirstSignal {
            run_id,
            window_id,
            launch,
        } => {
            if let Some(run) = state.runs.get_mut(&run_id) {
                super::first_turn::signalled(run, (window_id, launch), now);
            }
        }
        OrchEvent::ChainWindowGone { chain, window_id } => {
            super::chains::window_gone(state, &chain, window_id)
        }
        OrchEvent::AdoptLost {
            run_id,
            window_id,
            first_prompt,
        } => super::chains::adopt_lost(state, &run_id, window_id, first_prompt, now),
        OrchEvent::WakeHeld { run_id, held } => {
            if let Some(run) = state.runs.get_mut(&run_id) {
                run.orch.wake_held = held;
            }
        }
        OrchEvent::StartPrompt { run_id, waiting } => {
            if let Some(run) = state.runs.get_mut(&run_id) {
                run.orch.start_prompt = waiting;
            }
        }
        OrchEvent::DigestRead {
            run_id,
            notes_seq,
            at,
            ..
        } => {
            if let Some(run) = state.runs.get_mut(&run_id) {
                wake::digest_read(run, notes_seq, at);
            }
        }
        OrchEvent::OrchestratorWindow {
            run_id,
            window_id,
            live,
            launch,
        } => {
            if let Some(run) = state.runs.get_mut(&run_id) {
                super::orch_window::window_seen(run, (window_id, launch), (live, now), fx);
            }
        }
        OrchEvent::OtlpToken { run_id, token } => {
            let record = state.runs.get_mut(&run_id);
            if let Some(o) = record.and_then(|run| run.orch.orchestrator.as_mut())
                && o.otlp_token.is_empty()
            {
                o.otlp_token = token;
            }
        }
        OrchEvent::Installed {
            run_id,
            installed,
            window,
        } => {
            if let Some(run) = state.runs.get_mut(&run_id)
                && run.orch.orchestrator.is_none()
            {
                run.orch.installed = installed;
                run.orch.promote_window = Some(window);
            }
        }
        OrchEvent::RoleRoute {
            reply,
            run_id,
            decision,
            log,
        } => {
            let result = match state.runs.get_mut(&run_id) {
                Some(run) => super::history::keep(run, *decision, (log, now)),
                None => Err(format!("unknown run {run_id}")),
            };
            fx.push(Effect::Reply { reply, result });
        }
        // Milestone 9.13 decision 20: the first read is kept, with one log line.
        OrchEvent::UntrustedRead { run_id, at, source } => {
            if let Some(run) = state.runs.get_mut(&run_id)
                && run.orch.untrusted.is_none()
            {
                let line = crate::run::orch::untrusted::read_line(&source);
                run.orch.untrusted =
                    Some(crate::run::orch::untrusted::UntrustedRecord { at, source });
                super::requests::log(run, now, line);
            }
        }
        OrchEvent::RoleRouteEnded {
            run_id,
            record_id,
            outcome,
            result,
        } => {
            if let Some(run) = state.runs.get_mut(&run_id) {
                super::history::close(run, &record_id, (outcome, result), fx);
            }
        }
    }
}
