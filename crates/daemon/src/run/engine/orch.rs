//! Milestone 9, engine side (decision 1): the orchestrator's events, its tools' writes
//! (decisions 15, 19, 21 and 27: `edit_plan` with `submit` and `summary`, and the
//! engine's half of `spawn_subplanner`). Its window's lifecycle is `orch_window.rs`,
//! the holds `gate_holds.rs`, the promotion `promote.rs`. Pure (design decision 2).
//!
//! No orchestrator tool approves anything: `PlanEdit` has no approve or override
//! operation, and nothing here reaches `Approve`, `Override`, `Accept` or a hold's
//! verdict, which only the user's requests carry (`OrchEvent::{ApproveHold,
//! RejectHold}`, from `anthrex run approve|reject --hold`).

use proto::{AgentRole, RoleOutcome, RoleRoutingDecision, RunState, Runtime, TokenUsage, ToolCall};
use serde_json::json;

use super::batch::{Applied, Refused, apply_batch, record_rejected};
use super::requests::log;
use super::{Effect, EngineState, ReplyId, gate_holds, kinds, planners, run_scouts, wake};
use crate::run::edits_orch::{MessageOutcome, one_edit_rule};
use crate::run::model::Run;
use crate::run::orch::tools::{OrchCall, parse_call};
use crate::run::orch::{EditSource, EpicRecord, digest};
use crate::run::validate::EditScope;

/// The events of milestone 9's agents. Later tasks add the scouts', sub-planners' and
/// wake-ups' own.
#[derive(Debug, Clone, PartialEq)]
pub enum OrchEvent {
    /// Decision 15: an orchestrator's or sub-planner's write tool, or a worker's
    /// `task_note`, with the runtime refusals the driver computed (ruling T22-I1b).
    Tool {
        reply: ReplyId,
        call: ToolCall,
        refusals: Vec<(Runtime, String)>,
    },
    /// Decision 28's verdicts, from the user's `run approve|reject --hold` only.
    ApproveHold {
        reply: ReplyId,
        run_id: String,
        hold: String,
    },
    RejectHold {
        reply: ReplyId,
        run_id: String,
        hold: String,
    },
    /// Decision 20: a run scout's session ended (the driver awaits its handle).
    ScoutEnded {
        run_id: String,
        scout_id: String,
        outcome: ScoutEnd,
        usage: TokenUsage,
    },
    /// Decision 32: a sub-planner's session ended.
    PlannerEnded {
        run_id: String,
        epic: String,
        session: u32,
        outcome: ScoutEnd,
        usage: TokenUsage,
    },
    /// Decision 39: the driver pasted the wake-up of `digest_revision` into the
    /// orchestrator's window; `notes_seq` is its `Effect::WakeOrchestrator`'s.
    OrchestratorWoken {
        run_id: String,
        digest_revision: u64,
        notes_seq: u64,
    },
    /// Decisions 16 and 39: the orchestrator read the digest at `digest_revision`,
    /// whose answer included the wake notes up to `notes_seq` (`wake::notes_seq` of
    /// the run the answer was built from; M9.9 review fixes, M6).
    DigestRead {
        run_id: String,
        digest_revision: u64,
        notes_seq: u64,
    },
    /// Decision 13: the driver saw the orchestrator's window exit (`live: false`), or
    /// come back after an exit (`live: true`, the user's `anthrex restart`).
    OrchestratorWindow {
        run_id: String,
        window_id: u32,
        live: bool,
        /// The record's `launches` when the driver looked (M9.13 review).
        launch: u64,
    },
    /// Decision 14a: the run's OTLP token, drawn by the driver from the OS random
    /// source when it launches an orchestrator whose record has none.
    OtlpToken { run_id: String, token: String },
    /// Decision 43: the record of a session the driver dispatches (a run-bound decider,
    /// a run scout), sent before the session starts.
    /// Answered once the record is kept (review M-2): the driver starts the session
    /// only after the step that keeps it was saved.
    RoleRoute {
        reply: ReplyId,
        run_id: String,
        decision: Box<RoleRoutingDecision>,
    },
    /// Decision 43: that session ended.
    RoleRouteEnded {
        run_id: String,
        record_id: String,
        outcome: RoleOutcome,
        result: Option<String>,
    },
}

/// How a run scout's or sub-planner's session ended: its report or epic accepted, or
/// the machine's failure.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ScoutEnd {
    Reported,
    Failed { reason: String },
}

impl OrchEvent {
    /// The request the event answers, if any.
    pub fn reply(&self) -> Option<ReplyId> {
        match self {
            OrchEvent::Tool { reply, .. }
            | OrchEvent::ApproveHold { reply, .. }
            | OrchEvent::RejectHold { reply, .. }
            | OrchEvent::RoleRoute { reply, .. } => Some(*reply),
            OrchEvent::ScoutEnded { .. }
            | OrchEvent::PlannerEnded { .. }
            | OrchEvent::OrchestratorWoken { .. }
            | OrchEvent::DigestRead { .. }
            | OrchEvent::OrchestratorWindow { .. }
            | OrchEvent::OtlpToken { .. }
            | OrchEvent::RoleRouteEnded { .. } => None,
        }
    }
}

pub(super) fn on_orch_event(
    state: &mut EngineState,
    event: OrchEvent,
    now: u64,
    fx: &mut Vec<Effect>,
) {
    match event {
        // Milestone 9 decision 42f: a worker's `task_note` passes M8a's run gate and
        // early hold like its other tools (`done.rs`).
        OrchEvent::Tool { reply, call, .. } if call.role == AgentRole::Worker => {
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
        OrchEvent::OrchestratorWoken {
            run_id,
            digest_revision,
            notes_seq,
        } => {
            if let Some(run) = state.runs.get_mut(&run_id) {
                wake::woken(run, digest_revision, notes_seq);
            }
        }
        OrchEvent::DigestRead {
            run_id, notes_seq, ..
        } => {
            if let Some(run) = state.runs.get_mut(&run_id) {
                wake::digest_read(run, notes_seq, now);
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
        OrchEvent::RoleRoute {
            reply,
            run_id,
            decision,
        } => {
            let result = match state.runs.get_mut(&run_id) {
                Some(run) => {
                    super::history::open(run, *decision);
                    Ok("recorded".to_string())
                }
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
    let Some(run) = runs.get_mut(&call.run_id) else {
        return refuse(fx, reply, format!("unknown run {}", call.run_id));
    };
    if !matches!(call.role, AgentRole::Orchestrator | AgentRole::Planner) {
        let text = format!("tool {} is not available to this role", call.tool);
        return refuse(fx, reply, text);
    }
    match run.state {
        RunState::Planning
        | RunState::AwaitingApproval
        | RunState::Running
        | RunState::Complete => {}
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
    let window = run.orch.orchestrator.as_ref().and_then(|o| o.window_id);
    if window != Some(call.window_id) {
        let text = format!("this window is not the orchestrator of run {}", run.id);
        return refuse(fx, reply, text);
    }
    let parsed = match parse_call(call.role, &call.tool, &call.args) {
        Ok(parsed) => parsed,
        Err(text) => return refuse(fx, reply, text),
    };
    match parsed {
        OrchCall::EditPlan { edits, submit, .. }
            if ending.is_some() && (submit || !edits.is_empty()) =>
        {
            refuse(fx, reply, ending.unwrap_or_default())
        }
        OrchCall::SpawnSubplanner { .. } | OrchCall::SpawnScout { .. } if ending.is_some() => {
            refuse(fx, reply, ending.unwrap_or_default())
        }
        OrchCall::EditPlan {
            edits,
            submit,
            summary,
        } => {
            let call = (&edits[..], submit, summary);
            edit_plan(run, reply, call, refusals, (now, quiet_base), fx)
        }
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
        } => {
            let spec = EpicRecord::requested(&epic, &title, area, &brief, scout_refs);
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
    (edits, submit, summary): (&[proto::PlanEdit], bool, Option<String>),
    refusals: &[(Runtime, String)],
    (now, base): (u64, &mut Option<Run>),
    fx: &mut Vec<Effect>,
) {
    if run.state == RunState::Complete {
        let Some(summary) = summary.filter(|_| edits.is_empty()) else {
            let text = "edits are not accepted on a complete run; only a summary is";
            return refuse(fx, reply, text);
        };
        write_summary(run, summary, now);
        return accepted(run, reply, (Vec::new(), None, None), (now, base), fx);
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
    if submit && let Err(text) = submit_plan(&mut edited, "the orchestrator", now) {
        record_rejected(run, edits, &source, text.clone(), now);
        return refuse(fx, reply, text);
    }
    if let Some(summary) = summary {
        write_summary(&mut edited, summary, now);
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
fn accepted(
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

fn write_summary(run: &mut Run, summary: String, now: u64) {
    if let Some(o) = run.orch.orchestrator.as_mut() {
        o.summary = Some(summary);
        log(run, now, "the orchestrator wrote its summary");
    }
}

/// Decision 27's `submit`. In `planning` the plan must hold an unfinished task and no
/// sub-planner may be live; the run then waits at the gate, or runs at once when it was
/// started with `--yes`. In `awaiting_approval` nothing changes. On a promoted running
/// run the plan is submitted and hold `promotion` awaits the user, created empty when
/// nothing was added. A run being discarded or accepted takes no submit. Otherwise it is
/// ignored. `who` submits: the orchestrator, or the user's `run edit` (decision 13,
/// `requests::edit`, which admits `planning` only).
pub(super) fn submit_plan(run: &mut Run, who: &str, now: u64) -> Result<(), String> {
    // A discard or accept in flight takes no submit (M9.7 second review, ruling 4).
    if let Some(how) = super::dispatch::finishing_as(run) {
        return Err(format!("run {} is being {how}", run.id));
    }
    match run.state {
        RunState::Planning => {
            if run.tasks.iter().all(|t| t.state.is_finished()) {
                return Err("the plan has no tasks yet; add tasks before submitting".into());
            }
            planners_finished(run)?;
            set_submitted(run);
            if run.orch.yes {
                run.state = RunState::Running;
                run.approved_by = Some("--yes".into());
                run.approved_at = Some(now);
                for q in &mut run.decider_queue {
                    q.queued_at = now;
                }
                log(
                    run,
                    now,
                    format!("{who} submitted the plan; approved by --yes"),
                );
            } else {
                run.state = RunState::AwaitingApproval;
                log(
                    run,
                    now,
                    format!("{who} submitted the plan; awaiting approval"),
                );
            }
        }
        RunState::Running if run.orch.orchestrator.is_some() => {
            // While the promotion window is open, its submit waits for the
            // sub-planners too, so the user sees the promotion's whole plan (M9.7
            // second review, rulings 8 and 9).
            if gate_holds::promotion_open(run) {
                planners_finished(run)?;
            }
            set_submitted(run);
            if gate_holds::submit_promotion(run, now) {
                log(run, now, "the orchestrator submitted its additions");
            }
            // The epic rounds its own additions opened, whose epics no sub-planner is
            // planning (M9.7 second review, items 8 and 10).
            gate_holds::submit_epic_rounds(run, now);
        }
        _ => {}
    }
    Ok(())
}

/// Decision 27: no sub-planner is queued or planning.
pub(super) fn planners_finished(run: &Run) -> Result<(), String> {
    match run.orch.epics.iter().find(|e| e.phase.is_live()) {
        Some(e) => Err(format!(
            "sub-planner {} is still planning; submit when every sub-planner has finished",
            e.epic
        )),
        None => Ok(()),
    }
}

fn set_submitted(run: &mut Run) {
    if let Some(o) = run.orch.orchestrator.as_mut() {
        o.plan_submitted = true;
    }
}
