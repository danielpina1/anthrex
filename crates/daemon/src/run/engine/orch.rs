//! Milestone 9, engine side (decision 1): the orchestrator's events, its tools' writes
//! (decisions 15, 19, 21 and 27: `edit_plan` with `submit` and `summary`, and the
//! engine's half of `spawn_subplanner`). Its window's lifecycle is `orch_window.rs`,
//! the holds `gate_holds.rs`, the promotion `promote.rs`. Pure (design decision 2).
//!
//! No orchestrator tool approves anything: `PlanEdit` has no approve or override
//! operation, and nothing here reaches `Approve`, `Override`, `Accept` or a hold's
//! verdict, which only the user's requests carry (`OrchEvent::{ApproveHold,
//! RejectHold}`, from `anthrex run approve|reject --hold`).

use proto::{AgentRole, FindingAnswer, MessageTarget, PlanEdit, RunState, Runtime, ToolCall};
use serde_json::json;

use super::batch::{Applied, Refused, apply_batch, record_rejected};
use super::requests::log;
use super::{
    Effect, EngineState, ReplyId, gate_holds, goal_rounds, kinds, planners, run_scouts, wake,
};
use crate::run::edits_orch::{MessageOutcome, one_edit_rule};
use crate::run::model::Run;
use crate::run::orch::tools::{OrchCall, parse_call};
use crate::run::orch::{EditSource, EpicRecord, digest};
use crate::run::validate::EditScope;

#[path = "orch_event.rs"]
mod event;
pub use event::{OrchEvent, ScoutEnd};
#[path = "orch_submit.rs"]
mod submit;
pub(super) use submit::{submit_plan, submit_refusal};

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

/// A refusal: `ToolResult { ok: false }` holding `{"error": "<text>"}` (decision 15).
pub(super) fn refuse(fx: &mut Vec<Effect>, reply: ReplyId, text: impl Into<String>) {
    let error = json!({ "error": text.into() }).to_string();
    fx.push(Effect::Reply {
        reply,
        result: Err(error),
    });
}

/// Decision 15, in order: the run, its state (the orchestrator's and sub-planners' own
/// gate, ahead of M8a's `running`-only one), the caller, the tool and its arguments. A
/// sub-planner's call goes to `planners.rs` after the state gate.
pub(super) fn tool(
    state: &mut EngineState,
    reply: ReplyId,
    call: &ToolCall,
    refusals: &[(Runtime, String)],
    now: u64,
    fx: &mut Vec<Effect>,
) {
    let EngineState {
        runs, quiet_base, ..
    } = state;
    // Milestone 9.6 ruling T8-2: the pack's previous spec, frozen at the brainstorm's start.
    let earlier = (call.tool == "start_brainstorm")
        .then(|| crate::run::design::pack::previous_spec(runs.values(), &call.run_id))
        .flatten();
    let Some(run) = runs.get_mut(&call.run_id) else {
        return refuse(fx, reply, format!("unknown run {}", call.run_id));
    };
    use AgentRole::{Brainstormer, DocReviewer, Orchestrator, Planner};
    if !matches!(
        call.role,
        Orchestrator | Planner | Brainstormer | DocReviewer
    ) {
        let text = format!("tool {} is not available to this role", call.tool);
        return refuse(fx, reply, text);
    }
    match run.state {
        RunState::Brainstorming
        | RunState::Specifying
        | RunState::Planning
        | RunState::AwaitingApproval
        | RunState::Running
        | RunState::Complete => {}
        // Ruling WB-A-W1: a design agent's one write is held, not refused.
        RunState::Halted | RunState::Paused
            if super::design_agents::held::takes(run, call.role) => {}
        // Milestone 9.9 decision 6: a halted run takes the orchestrator's lone `resume_run`
        // (and `ask_user`, here and on a paused run).
        RunState::Halted | RunState::Paused if super::orch_ops::admitted(run.state, call) => {}
        RunState::Paused => {
            let text = format!("run {} is paused; the user must resume it", run.id);
            return refuse(fx, reply, text);
        }
        other => return refuse(fx, reply, format!("run {} is {}", run.id, other.label())),
    }
    // M9.9 second review, C-1: a run being ended takes no new work; the reads, and
    // the orchestrator's summary, still answer.
    let ending = ending(run);
    if call.role == AgentRole::Planner {
        if let Some(text) = ending.filter(|_| call.tool == "submit_epic") {
            return refuse(fx, reply, text);
        }
        return planners::tool(run, reply, call, refusals, now, fx);
    }
    // Milestone 9.6: a design agent's write (task M9.6.8).
    if matches!(call.role, Brainstormer | DocReviewer) {
        return super::design_agents::tool(run, reply, call, (ending, now), fx);
    }
    let window = run.orch.orchestrator.as_ref().and_then(|o| o.window_id);
    if window != Some(call.window_id) {
        let text = format!("this window is not the orchestrator of run {}", run.id);
        return refuse(fx, reply, text);
    }
    // Milestone 9.9 decision 19: the orchestrator's own window called a tool: it acts.
    if call.role == Orchestrator {
        super::orch_stall::acted(run, now);
    }
    let parsed = match parse_call(call.role, &call.tool, &call.args) {
        Ok(parsed) => parsed,
        Err(text) => return refuse(fx, reply, text),
    };
    let Some(parsed) = super::design::tool(run, reply, call, (parsed, earlier), now, fx) else {
        return;
    };
    match parsed {
        OrchCall::EditPlan { edits, submit, .. }
            if ending.is_some()
                && (submit || !edits.is_empty())
                && !earlier_rounds_only(run, &edits, submit) =>
        {
            refuse(fx, reply, ending.unwrap_or_default())
        }
        OrchCall::SpawnSubplanner { .. } | OrchCall::SpawnScout { .. } if ending.is_some() => {
            refuse(fx, reply, ending.unwrap_or_default())
        }
        // Milestone 9.3 decision 30: a round, alone in its call.
        OrchCall::EditPlan {
            iterate: Some(goal),
            edits,
            submit,
            summary,
            responses,
        } => {
            let alone = edits.is_empty() && !submit && summary.is_none() && responses.is_empty();
            goal_rounds::edit(run, reply, (&goal, alone), (now, quiet_base), fx)
        }
        OrchCall::EditPlan {
            edits,
            submit,
            summary,
            iterate: None,
            responses,
        } => {
            let call = (&edits[..], submit, summary, &responses[..]);
            edit_plan(run, reply, call, refusals, (now, quiet_base), fx)
        }
        // Milestone 9.9 ruling R9: a complete run may still ask (decision 15).
        OrchCall::AskUser {
            question,
            options,
            context,
        } => super::asks::ask(run, reply, (question, options, context), now, fx),
        _ if run.state == RunState::Complete => {
            let text = format!("run {} is {}", run.id, run.state.label());
            refuse(fx, reply, text)
        }
        OrchCall::SpawnSubplanner {
            epic,
            title,
            area,
            brief,
            scout_refs,
            covers,
        } => {
            let spec = EpicRecord::requested(&epic, &title, area, &brief, scout_refs);
            let spec = EpicRecord { covers, ..spec };
            planners::spawn_subplanner(run, reply, spec, (now, quiet_base), fx)
        }
        OrchCall::SpawnScout {
            id,
            question,
            area,
            web,
        } => {
            let args = (&id[..], question, area, web);
            run_scouts::spawn(run, reply, args, (now, quiet_base), fx)
        }
        // The reads are answered by the driver (M9.11).
        _ => refuse(
            fx,
            reply,
            format!("tool {} is not available yet", call.tool),
        ),
    }
}

/// Decision 19's `edit_plan`: one batch, then `submit` (decision 27) and `summary`, all
/// or nothing. On a `complete` run only a summary is accepted.
fn edit_plan(
    run: &mut Run,
    reply: ReplyId,
    (edits, submit, summary, responses): (&[PlanEdit], bool, Option<String>, &[FindingAnswer]),
    refusals: &[(Runtime, String)],
    (now, base): (u64, &mut Option<Run>),
    fx: &mut Vec<Effect>,
) {
    if run.state == RunState::Complete {
        let Some(summary) = summary.filter(|_| edits.is_empty()) else {
            let text = "edits are not accepted on a complete run; only a summary is";
            return refuse(fx, reply, text);
        };
        write_summary(run, summary, now, fx);
        return accepted(run, reply, (Vec::new(), None, None), (now, base), fx);
    }
    // Milestone 9.9 decision 11: an op is alone in its call and answered there.
    let others = summary.is_some() || !responses.is_empty();
    if super::orch_ops::intercept(run, reply, (edits, submit, others), (now, base), fx) {
        return;
    }
    // Decision 42: a `message` or `refresh` is alone in its call, before any effect.
    if let Err(error) = one_edit_rule(edits, submit, summary.is_some()) {
        return refuse(fx, reply, error.to_string());
    }
    let source = EditSource::Orchestrator;
    // Decision 37: the engine owns its integration reviews.
    if let Some(text) = kinds::engine_owned(run, edits) {
        record_rejected(run, edits, &source, text.clone(), now);
        return refuse(fx, reply, text);
    }
    // Milestone 9.6 ruling T7-8: the plan at its open gate is the user's to change.
    let changes = super::design::plan::changes_tasks(edits);
    if let Some(text) = super::design_gate::plan_locked(run).filter(|_| changes) {
        record_rejected(run, edits, &source, text.clone(), now);
        return refuse(fx, reply, text);
    }
    let mut edited = run.clone();
    let mut effects = Vec::new();
    let mut added = Vec::new();
    let mut message = None;
    if !edits.is_empty() {
        let batch = (edits, &EditScope::Run, refusals);
        match apply_batch(&mut edited, batch, &source, now, &mut effects) {
            Ok(Applied {
                added: new,
                message: outcome,
                ..
            }) => (added, message) = (new, outcome),
            // Decision 40: a rejected batch is logged too.
            Err(Refused::Text(text)) => {
                record_rejected(run, edits, &source, text.clone(), now);
                return refuse(fx, reply, text);
            }
            Err(Refused::Plan(errors)) => {
                let first = errors.first().map(ToString::to_string).unwrap_or_default();
                record_rejected(run, edits, &source, first, now);
                return fx.push(Effect::Reply {
                    reply,
                    result: Err(rejected(&errors)),
                });
            }
        }
    }
    let held = gate_holds::assign(&mut edited, edits, &added, now);
    // Milestone 9.2 decision 31: a review fix outside its stage waits for the user,
    // whatever other task of the call an epic or the promotion holds (fix round, I2).
    let review = super::delivery::review_holds(&mut edited, &added, now);
    let held = held.or(review);
    // Milestone 9.6 decisions 18 to 21: a design run's checks, plan review and gate.
    let author = submit.then_some(proto::DocAuthor::Orchestrator);
    let submitted = super::design::plan::submit(&mut edited, author, responses, now, &mut effects);
    if let Err(text) = submitted {
        super::design_agents::reviewer::refused(run, &text, now);
        record_rejected(run, edits, &source, text.clone(), now);
        return refuse(fx, reply, text);
    }
    if let Some(summary) = summary {
        write_summary(&mut edited, summary, now, &mut effects);
    }
    let notes = new_notes(run, &edited);
    *run = edited;
    fx.extend(effects);
    accepted(run, reply, (notes, held, message), (now, base), fx)
}

/// Decision 19's rejected batch: `{"accepted": false, "errors": [...]}`.
pub(super) fn rejected(errors: &[crate::run::plan::PlanError]) -> String {
    let errors: Vec<_> = errors
        .iter()
        .map(|e| json!({"task": e.task, "field": e.field, "rule": e.rule, "message": e.message}))
        .collect();
    json!({"accepted": false, "errors": errors}).to_string()
}

/// Decision 19's accepted reply. `revision` is the digest's after this batch and the
/// scheduler's pass over it (run here first, as a tick at the same time would), so it
/// is the revision `run_status` then reports.
pub(super) fn accepted(
    run: &mut Run,
    reply: ReplyId,
    (notes, held, message): (Vec<String>, Option<String>, Option<MessageOutcome>),
    (now, base): (u64, &mut Option<Run>),
    fx: &mut Vec<Effect>,
) {
    settle_quiet(run, base, now, fx);
    let hold_awaits = held.as_ref().is_some_and(|id| {
        run.orch
            .gate_holds
            .iter()
            .any(|h| &h.id == id && h.state == proto::HoldState::Awaiting)
    });
    let mut value = json!({
        "accepted": true,
        "revision": run.orch.digest_rev,
        "awaiting_approval": run.state == RunState::AwaitingApproval || hold_awaits,
        "notes": notes,
        "held": held,
    });
    // Milestone 9.6 decision 20: the plan went to its review, not to the gate.
    if super::design::plan::awaiting_review(run) {
        value["awaiting_review"] = json!(true);
    }
    // Decision 42b: a message's recipients.
    if let Some(outcome) = message {
        let refused: Vec<_> = outcome
            .refused
            .iter()
            .map(|(task, reason)| json!({"task": task, "reason": reason}))
            .collect();
        value["delivered"] = json!(outcome.delivered);
        value["refused"] = json!(refused);
    }
    let text = value.to_string();
    fx.push(Effect::Reply {
        reply,
        result: Ok(text),
    });
}

/// M9.9 second review, M-b: the orchestrator's own call keeps the run as its edit left
/// it, before this pass, in `base`: `engine::step` quiets only the blocks up to there,
/// so a stall this pass finds is still noted.
pub(super) fn settle_quiet(run: &mut Run, base: &mut Option<Run>, now: u64, fx: &mut Vec<Effect>) {
    *base = Some(run.clone());
    settle(run, now, fx);
}

/// W1 fix round 2: while a round is being cancelled, the orchestrator still answers
/// and messages the earlier rounds' live tasks (the final fix wave's A-I2); nothing
/// else, and nothing of the round itself.
fn earlier_rounds_only(run: &Run, edits: &[PlanEdit], submit: bool) -> bool {
    let Some(n) = super::goal_rounds_end::cancelling_round(run) else {
        return false;
    };
    let earlier = |id: &str| run.task(id).is_some_and(|t| t.round < n);
    !submit
        && edits.iter().all(|edit| match edit {
            PlanEdit::Answer { task_id, .. } => earlier(task_id),
            PlanEdit::Message {
                to: MessageTarget::Tasks(ids),
                ..
            } => ids.iter().all(|id| earlier(id)),
            _ => false,
        })
}

/// `run <id> was cancelled` or `run <id> is finishing` while the run is being ended
/// (M9.9 second review, C-1).
pub(super) fn ending(run: &Run) -> Option<String> {
    if run.cancelled {
        Some(format!("run {} was cancelled", run.id))
    } else if run.finish_edit {
        Some(format!("run {} is finishing", run.id))
    } else {
        None
    }
}

/// The scheduler's pass, then the digest's revision (decision 16).
pub(super) fn settle(run: &mut Run, now: u64, fx: &mut Vec<Effect>) {
    super::dispatch::schedule(run, now, fx);
    digest::note_change(run);
}

/// The validation notes the batch added, as `<task>: <note>`.
fn new_notes(before: &Run, after: &Run) -> Vec<String> {
    let mut out = Vec::new();
    for task in &after.tasks {
        let old = before.task(task.id()).map_or(&[][..], |t| &t.notes[..]);
        for note in task.notes.iter().filter(|n| !old.contains(n)) {
            out.push(format!("{}: {note}", task.id()));
        }
    }
    out
}

/// The report is rewritten at once (M9.16), so a `complete` run's opens with the summary
/// before the user accepts or discards it.
fn write_summary(run: &mut Run, summary: String, now: u64, fx: &mut Vec<Effect>) {
    // Milestone 9.3 decision 17: the round's summary too. While a later round is still
    // open, a summary is the previous round's, whose completion asked for it (task 4b
    // fix round 1, m2).
    let last = run.rounds.len().saturating_sub(1);
    let k = last.saturating_sub(usize::from(super::goal_rounds_end::open_round(run)));
    // The final fix wave (review A, I4): a `pr` round's summary is asked for at its own
    // end (decision 17); the completion's summary is the run's only.
    let completed_pr = super::delivery::pr(run) && run.state == RunState::Complete;
    let keep = |r: &crate::run::model::Round| completed_pr && r.n > 1 && r.summary.is_some();
    if let Some(o) = run.orch.orchestrator.as_mut() {
        if let Some(round) = run.rounds.get_mut(k).filter(|r| !keep(r)) {
            round.summary = Some(summary.clone());
        }
        o.summary = Some(summary);
        log(run, now, "the orchestrator wrote its summary");
        fx.push(Effect::WriteReport {
            run_id: run.id.clone(),
        });
    }
}
