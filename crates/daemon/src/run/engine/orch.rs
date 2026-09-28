//! Milestone 9, engine side (decision 1): the orchestrator's events, its tools' writes
//! (decisions 15, 19, 21 and 27: `edit_plan` with `submit` and `summary`, and the
//! engine's half of `spawn_subplanner`). Its window's lifecycle is `orch_window.rs`,
//! the holds `gate_holds.rs`, the promotion `promote.rs`. Pure (design decision 2).
//!
//! No orchestrator tool approves anything: `PlanEdit` has no approve or override
//! operation, and nothing here reaches `Approve`, `Override`, `Accept` or a hold's
//! verdict, which only the user's requests carry (`OrchEvent::{ApproveHold,
//! RejectHold}`, from `anthrex run approve|reject --hold`).

use proto::{AgentRole, RunState, Runtime, ToolCall};
use serde_json::json;

use super::batch::{Applied, Refused, apply_batch};
use super::requests::log;
use super::{Effect, EngineState, ReplyId, gate_holds};
use crate::run::model::Run;
use crate::run::orch::launch::planner_route;
use crate::run::orch::tools::{OrchCall, parse_call};
use crate::run::orch::{EditSource, EpicRecord, PlannerPhase, digest};
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
}

impl OrchEvent {
    /// The request the event answers, if any.
    pub fn reply(&self) -> Option<ReplyId> {
        match self {
            OrchEvent::Tool { reply, .. }
            | OrchEvent::ApproveHold { reply, .. }
            | OrchEvent::RejectHold { reply, .. } => Some(*reply),
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
    }
}

/// A refusal: `ToolResult { ok: false }` holding `{"error": "<text>"}` (decision 15).
fn refuse(fx: &mut Vec<Effect>, reply: ReplyId, text: impl Into<String>) {
    let error = json!({ "error": text.into() }).to_string();
    fx.push(Effect::Reply {
        reply,
        result: Err(error),
    });
}

/// Decision 15, in order: the run, its state (the orchestrator's own gate, ahead of
/// M8a's `running`-only one), the caller, the tool and its arguments.
fn tool(
    state: &mut EngineState,
    reply: ReplyId,
    call: &ToolCall,
    refusals: &[(Runtime, String)],
    now: u64,
    fx: &mut Vec<Effect>,
) {
    let Some(run) = state.runs.get_mut(&call.run_id) else {
        return refuse(fx, reply, format!("unknown run {}", call.run_id));
    };
    if call.role != AgentRole::Orchestrator {
        // Sub-planners' `submit_epic` (M9.8) and workers' `task_note` (M9.13a).
        return refuse(
            fx,
            reply,
            format!("tool {} is not available yet", call.tool),
        );
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
        OrchCall::EditPlan {
            edits,
            submit,
            summary,
        } => edit_plan(run, reply, (&edits, submit, summary), refusals, now, fx),
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
            spawn_subplanner(run, reply, spec, now, fx)
        }
        // `spawn_scout` is M9.8's; the reads are answered by the driver (M9.11).
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
    now: u64,
    fx: &mut Vec<Effect>,
) {
    if run.state == RunState::Complete {
        let Some(summary) = summary.filter(|_| edits.is_empty()) else {
            let text = "edits are not accepted on a complete run; only a summary is";
            return refuse(fx, reply, text);
        };
        write_summary(run, summary, now);
        return accepted(run, reply, (Vec::new(), None), now, fx);
    }
    let mut edited = run.clone();
    let mut effects = Vec::new();
    let mut added = Vec::new();
    if !edits.is_empty() {
        let batch = (edits, &EditScope::Run, refusals);
        match apply_batch(
            &mut edited,
            batch,
            &EditSource::Orchestrator,
            now,
            &mut effects,
        ) {
            Ok(Applied { added: new, .. }) => added = new,
            Err(Refused::Text(text)) => return refuse(fx, reply, text),
            Err(Refused::Plan(errors)) => {
                let errors: Vec<_> = errors
                    .iter()
                    .map(|e| {
                        json!({"task": e.task, "field": e.field, "rule": e.rule, "message": e.message})
                    })
                    .collect();
                let text = json!({"accepted": false, "errors": errors}).to_string();
                return fx.push(Effect::Reply {
                    reply,
                    result: Err(text),
                });
            }
        }
    }
    let held = gate_holds::assign(&mut edited, edits, &added, now);
    if submit && let Err(text) = submit_plan(&mut edited, "the orchestrator", now) {
        return refuse(fx, reply, text);
    }
    if let Some(summary) = summary {
        write_summary(&mut edited, summary, now);
    }
    let notes = new_notes(run, &edited);
    *run = edited;
    fx.extend(effects);
    accepted(run, reply, (notes, held), now, fx)
}

/// Decision 19's accepted reply. `revision` is the digest's after this batch and the
/// scheduler's pass over it (run here first, as a tick at the same time would), so it
/// is the revision `run_status` then reports.
fn accepted(
    run: &mut Run,
    reply: ReplyId,
    (notes, held): (Vec<String>, Option<String>),
    now: u64,
    fx: &mut Vec<Effect>,
) {
    settle(run, now, fx);
    let hold_awaits = held.as_ref().is_some_and(|id| {
        run.orch
            .gate_holds
            .iter()
            .any(|h| &h.id == id && h.state == proto::HoldState::Awaiting)
    });
    let text = json!({
        "accepted": true,
        "revision": run.orch.digest_rev,
        "awaiting_approval": run.state == RunState::AwaitingApproval || hold_awaits,
        "notes": notes,
        "held": held,
    })
    .to_string();
    fx.push(Effect::Reply {
        reply,
        result: Ok(text),
    });
}

/// The scheduler's pass, then the digest's revision (decision 16).
fn settle(run: &mut Run, now: u64, fx: &mut Vec<Effect>) {
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
            if let Some(e) = run.orch.epics.iter().find(|e| e.phase.is_live()) {
                return Err(format!(
                    "sub-planner {} is still planning; submit when every sub-planner has finished",
                    e.epic
                ));
            }
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
            set_submitted(run);
            if gate_holds::submit_promotion(run, now) {
                log(run, now, "the orchestrator submitted its additions");
            }
        }
        _ => {}
    }
    Ok(())
}

fn set_submitted(run: &mut Run) {
    if let Some(o) = run.orch.orchestrator.as_mut() {
        o.plan_submitted = true;
    }
}

/// Decision 21, the engine's state half (task M9.8 validates the epic and its area,
/// sets the path and starts the session in a reader slot): a new epic is recorded with
/// its sub-planner queued, and held on a running run (decision 28); an epic whose
/// sub-planner has ended is queued again for a fresh one. On a run awaiting approval,
/// planning resumes (decision 27).
fn spawn_subplanner(
    run: &mut Run,
    reply: ReplyId,
    mut spec: EpicRecord,
    now: u64,
    fx: &mut Vec<Effect>,
) {
    let epic = spec.epic.clone();
    let mut hold = None;
    if let Some(e) = run.orch.epics.iter_mut().find(|e| e.epic == epic) {
        if e.phase.is_live() {
            let text =
                format!("epic {epic} is being planned by its sub-planner; wait for it to finish");
            return refuse(fx, reply, text);
        }
        e.phase = PlannerPhase::Queued;
        e.replans.push(spec.brief.chars().take(40).collect());
        e.request = spec.brief;
        e.ended_at = None;
    } else {
        if let Some(route) = planner_route(run) {
            spec.route = route;
        }
        spec.started_at = now;
        run.orch.epics.push(spec);
        hold = gate_holds::create_epic_hold(run, &epic, now);
    }
    log(run, now, format!("sub-planner {epic} queued"));
    if run.state == RunState::AwaitingApproval {
        run.state = RunState::Planning;
        if let Some(o) = run.orch.orchestrator.as_mut() {
            o.plan_submitted = false;
        }
        log(
            run,
            now,
            "planning again: a sub-planner was started at the gate",
        );
    }
    settle(run, now, fx);
    let text = json!({"epic": epic, "state": "queued", "hold": hold}).to_string();
    fx.push(Effect::Reply {
        reply,
        result: Ok(text),
    });
}
