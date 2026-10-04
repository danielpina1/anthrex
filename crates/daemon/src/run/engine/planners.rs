//! Milestone 9 decisions 21, 22, 31 and 32, engine side: sub-planners. An epic's
//! record keeps its planner's phase, sessions, counts and rejections; the session itself
//! runs on M8b's scout machine (`scout/planner.rs`), which this module starts in a
//! reader slot (`OpKind::StartPlanner`) and never drives turn by turn. Its one write,
//! `submit_epic`, is answered here. Run scouts are `run_scouts.rs`; the reader-slot
//! order both follow is [`dispatch`]'s. Pure (design decision 2).
//!
//! **The binding rule.** A sub-planner's batch goes through the same
//! `gate_holds::assign` as the orchestrator's: on a promoted or gated run its tasks wait
//! under its epic's round, which its accepted epic submits to the user, so nothing it
//! adds runs before the user approves a round that holds it.

use proto::{PlanEdit, RunPath, RunState, Runtime, TaskState, ToolCall};

use super::batch::{Applied, Refused, apply_batch, record_rejected};
use super::orch::{refuse, rejected, settle, settle_quiet};
use super::requests::log;
use super::{
    Effect, OpKind, OpResult, ReplyId, emit_op, gate_holds, history, kinds, next_op, run_scouts,
    wake,
};
use crate::run::edits_orch::planner_confinement;
use crate::run::globs::{intersects, validate_glob};
use crate::run::model::Run;
use crate::run::orch::contract_rounds::ITERATE_BY_PLANNER;
use crate::run::orch::launch::{planner_route, planner_spec};
use crate::run::orch::roles;
use crate::run::orch::tools::{OrchCall, parse_call};
use crate::run::orch::{EditSource, EpicRecord, PlannerPhase, PlannerSession};
use crate::run::plan::PlanError;
use crate::run::validate::EditScope;
use crate::run::validate_graph::is_valid_area_glob;

use super::ScoutEnd;

/// Decision 22's reply to an accepted epic.
pub const EPIC_RECORDED: &str = "Epic recorded. You are done; end your turn now.";

/// M9.8 review, ruling 4: the most re-plans of one epic. Engine constants; M9.5's
/// tuning may make them configurable.
pub const MAX_REPLANS_PER_EPIC: u32 = 3;
/// M9.8 review, ruling 4: the most epics in one run.
pub const MAX_EPICS: usize = 20;

/// The live sessions holding reader slots (decision 31): sub-planners that were
/// started and have not ended, and run scouts running.
pub(crate) fn readers(run: &Run) -> usize {
    let planners = run
        .orch
        .epics
        .iter()
        .filter(|e| e.phase == PlannerPhase::Planning)
        .count();
    planners + run_scouts::running(run)
}

/// Decision 31's reader-slot order, after M8b's deciders and the reviewers (which the
/// scheduler's running pass dispatches first): sub-planners, oldest epic first, then
/// run scouts, in the order they were asked for. Research and review tasks follow
/// (task M9.9). Planners and scouts start while the run is being planned, at the gate
/// and while it runs; nothing starts in any other state.
pub(super) fn dispatch(run: &mut Run, now: u64, fx: &mut Vec<Effect>) {
    if !super::design::readers_start(run.state) {
        return;
    }
    while super::schedule::readers_busy(run) < usize::from(run.limits.max_readers) {
        if let Some(k) = run
            .orch
            .epics
            .iter()
            .position(|e| e.phase == PlannerPhase::Queued)
        {
            start(run, k, now, fx);
        } else if !super::design_agents::start_next(run, now, fx)
            && !run_scouts::start_next(run, now, fx)
        {
            break;
        }
    }
}

/// Session `n + 1` of epic `k`'s sub-planner (decision 32).
fn start(run: &mut Run, k: usize, now: u64, fx: &mut Vec<Effect>) {
    let session = run.orch.epics[k].sessions.len() as u32 + 1;
    let op = next_op(run);
    let spec = planner_spec(run, &run.orch.epics[k], session);
    let epic = &mut run.orch.epics[k];
    epic.phase = PlannerPhase::Planning;
    epic.sessions.push(PlannerSession {
        session,
        window_id: None,
        op: Some(op),
        started_at: now,
        ended_at: None,
        usage: Default::default(),
        rejections: 0,
    });
    let text = format!("sub-planner {} session {session} starting", epic.epic);
    log(run, now, text);
    // Decision 43: the session's record, before its start.
    let record = roles::planner_record(run, k, session, now);
    history::open(run, record);
    let kind = OpKind::StartPlanner {
        spec: Box::new(spec),
    };
    emit_op(run, op, None, kind, fx);
}

/// `StartPlanner`'s result: the session's window, or its failure. Returns the window a
/// session was bound to, for the calls held before it (`early.rs`).
pub(super) fn started(
    run: &mut Run,
    kind: &OpKind,
    result: OpResult,
    now: u64,
    fx: &mut Vec<Effect>,
) -> Option<u32> {
    let OpKind::StartPlanner { spec } = kind else {
        return None;
    };
    if let OpResult::Failed { message } = &result {
        let why = format!("the sub-planner could not start: {message}");
        history::planner_ended(run, (&spec.epic, spec.session), Err(why), fx);
    }
    let k = run.orch.epics.iter().position(|e| e.epic == spec.epic)?;
    let latest = run.orch.epics[k].sessions.len() as u32 == spec.session;
    let epic = &mut run.orch.epics[k];
    let session = epic
        .sessions
        .iter_mut()
        .find(|s| s.session == spec.session)?;
    match result {
        OpResult::PlannerStarted { window_id } => {
            session.window_id = Some(window_id);
            // Halted while its launch was in flight (`halt_all`): stopped at once.
            if let (true, PlannerPhase::Failed { reason }) = (latest, &epic.phase) {
                let reason = reason.clone();
                fx.push(Effect::StopPlanner { window_id, reason });
            }
            Some(window_id)
        }
        OpResult::Failed { message } if latest && epic.phase == PlannerPhase::Planning => {
            session.ended_at = Some(now);
            let reason = format!("the sub-planner could not start: {message}");
            fail(run, k, reason, now);
            None
        }
        _ => None,
    }
}

/// Decision 32: a sub-planner's session ended. Its usage counts whatever the session;
/// the latest session of a planning epic that failed fails the epic, which keeps the
/// tasks it already had and wakes the orchestrator. An accepted epic was finished by
/// its `submit_epic` already.
pub(super) fn ended(
    run: &mut Run,
    (epic, session): (&str, u32),
    (outcome, usage): (ScoutEnd, proto::TokenUsage),
    now: u64,
    fx: &mut Vec<Effect>,
) {
    let Some(k) = run.orch.epics.iter().position(|e| e.epic == epic) else {
        return;
    };
    history::planner_ended(run, (epic, session), Ok(&outcome), fx);
    run.orch.planner_usage += usage;
    let record = &mut run.orch.epics[k];
    let latest = record.sessions.len() as u32 == session;
    if let Some(s) = record.sessions.iter_mut().find(|s| s.session == session) {
        s.usage += usage;
        s.ended_at.get_or_insert(now);
    }
    if !latest || record.phase != PlannerPhase::Planning {
        return;
    }
    let reason = match outcome {
        ScoutEnd::Failed { reason } => reason,
        ScoutEnd::Reported => "the sub-planner ended without an accepted epic".to_string(),
    };
    fail(run, k, reason, now);
}

/// Epic `k`'s sub-planner fails with `reason` (decision 32).
fn fail(run: &mut Run, k: usize, reason: String, now: u64) {
    let epic = &mut run.orch.epics[k];
    epic.phase = PlannerPhase::Failed {
        reason: reason.clone(),
    };
    epic.ended_at = Some(now);
    let name = epic.epic.clone();
    log(run, now, format!("sub-planner {name} failed: {reason}"));
    wake::note(run, format!("sub-planner {name} failed: {reason}"));
}

/// `run cancel`'s reason for the sub-planners and run scouts it halts.
pub const RUN_CANCELLED: &str = "the run was cancelled";
/// The `finish` edit's reason for the sub-planners and run scouts it halts.
pub const RUN_FINISHED: &str = "the finish edit ends the run";

/// M9.9 review fixes: `run cancel` and the `finish` edit end the run's sub-planners
/// and run scouts rather than wait for them. A live session is halted (the machine's
/// `ScoutEvent::Halt`, through `Effect::StopPlanner` or `Effect::StopScout`); a queued
/// one never starts. Each ends `failed` with `reason` (decisions 20 and 32's outcome).
pub(super) fn halt_all(run: &mut Run, reason: &str, now: u64, fx: &mut Vec<Effect>) {
    for k in 0..run.orch.epics.len() {
        if !run.orch.epics[k].phase.is_live() {
            continue;
        }
        let epic = &run.orch.epics[k];
        if let (PlannerPhase::Planning, Some(s)) = (&epic.phase, epic.sessions.last()) {
            let session = format!("{}/{}", epic.epic, s.session);
            history::session_stopped(run, (proto::AgentRole::Planner, &session), fx);
        }
        let window = run.orch.epics[k]
            .sessions
            .last()
            .filter(|s| s.ended_at.is_none())
            .and_then(|s| s.window_id);
        if let Some(window_id) = window {
            fx.push(Effect::StopPlanner {
                window_id,
                reason: reason.to_string(),
            });
        }
        fail(run, k, reason.to_string(), now);
    }
    run_scouts::halt_all(run, reason, now, fx);
    super::design_agents::halt_all(run, reason, fx);
}

/// Decision 32 after a daemon restart: a queued or live sub-planner is not resumed; it
/// fails, and nothing is killed (its `StartPlanner` reconciles as `NotStarted`).
pub(super) fn restore(run: &mut Run, now: u64) {
    for k in 0..run.orch.epics.len() {
        if run.orch.epics[k].phase.is_live() {
            let reason = "the daemon restarted during this sub-planner".to_string();
            fail(run, k, reason, now);
        }
    }
    run_scouts::restore(run, now);
    super::design_agents::restore(run, now);
}

/// Decision 21: `spawn_subplanner`. A new epic's id is checked by the tools' parser;
/// here its area (literal paths or `<literal>/**`, overlapping no other epic's) is
/// checked, the run takes the large path, and the epic is recorded with its sub-planner
/// queued for a reader slot and, past the plan gate, its own hold round (decision 28).
/// An epic whose sub-planner has ended is re-planned: a fresh session with the new
/// brief, which opens a round past the gate (M9.7 second review, ruling 10); its title
/// and area stay the epic's. A live one refuses. At the gate, planning resumes
/// (decision 27).
pub(super) fn spawn_subplanner(
    run: &mut Run,
    reply: ReplyId,
    mut spec: EpicRecord,
    (now, base): (u64, &mut Option<Run>),
    fx: &mut Vec<Effect>,
) {
    let epic = spec.epic.clone();
    let hold;
    if let Some(e) = run.orch.epics.iter_mut().find(|e| e.epic == epic) {
        if e.phase.is_live() {
            let text =
                format!("epic {epic} is being planned by its sub-planner; wait for it to finish");
            return refuse(fx, reply, text);
        }
        if e.replans.len() >= MAX_REPLANS_PER_EPIC as usize {
            let text = format!(
                "epic {epic} was already re-planned {MAX_REPLANS_PER_EPIC} times, the most one epic allows"
            );
            return refuse(fx, reply, text);
        }
        e.phase = PlannerPhase::Queued;
        e.replans.push(spec.brief.chars().take(40).collect());
        e.request = spec.brief;
        e.ended_at = None;
        hold = gate_holds::replan_epic_hold(run, &epic, now);
    } else {
        if run.orch.epics.len() >= MAX_EPICS {
            let text = format!(
                "run {} already has {MAX_EPICS} epics, the most one run allows",
                run.id
            );
            return refuse(fx, reply, text);
        }
        if let Err(text) = check_area(run, &epic, &spec.area) {
            return refuse(fx, reply, text);
        }
        if let Some(route) = planner_route(run) {
            spec.route = route;
        }
        spec.started_at = now;
        run.orch.epics.push(spec);
        run.path = Some(RunPath::Large);
        hold = gate_holds::create_epic_hold(run, &epic, now);
    }
    log(run, now, format!("sub-planner {epic} queued"));
    if run.state == RunState::AwaitingApproval {
        super::design_gate::plan_changed(run, now); // milestone 9.6 ruling T7-4
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
    settle_quiet(run, base, now, fx);
    let state = match run.orch.epics.iter().find(|e| e.epic == epic) {
        Some(e) if e.phase == PlannerPhase::Planning => "planning",
        _ => "queued",
    };
    let text = serde_json::json!({"epic": epic, "state": state, "hold": hold}).to_string();
    fx.push(Effect::Reply {
        reply,
        result: Ok(text),
    });
}

/// Decision 21's area: each glob valid and a literal path or `<literal>/**` (M8a
/// decision 12's area form), and none intersecting another epic's area.
fn check_area(run: &Run, epic: &str, area: &[String]) -> Result<(), String> {
    for glob in area {
        if let Err(problem) = validate_glob(glob) {
            return Err(format!("epic {epic}: area: {glob}: {problem}"));
        }
        if !is_valid_area_glob(glob) {
            return Err(format!(
                "epic {epic}: area: {glob} must be a literal path or end in /**"
            ));
        }
    }
    for other in run.orch.epics.iter().filter(|e| e.epic != epic) {
        for theirs in &other.area {
            if area.iter().any(|mine| intersects(mine, theirs)) {
                return Err(format!(
                    "epic {epic}: area: overlaps epic {}'s area ({theirs})",
                    other.epic
                ));
            }
        }
    }
    Ok(())
}

/// A sub-planner's tool (decisions 15, 22), after the run-state gate: the caller must
/// be its epic's live session; then `submit_epic`. Its reads are the driver's.
pub(super) fn tool(
    run: &mut Run,
    reply: ReplyId,
    call: &ToolCall,
    refusals: &[(Runtime, String)],
    now: u64,
    fx: &mut Vec<Effect>,
) {
    if run.state == RunState::Complete {
        let text = format!("run {} is {}", run.id, run.state.label());
        return refuse(fx, reply, text);
    }
    let epic = call.epic.clone().unwrap_or_default();
    let Some(k) = run.orch.epics.iter().position(|e| e.epic == epic) else {
        return refuse(fx, reply, format!("unknown epic {epic}"));
    };
    let record = &run.orch.epics[k];
    let latest = record.sessions.last();
    let window = latest.and_then(|s| s.window_id);
    if window == Some(call.window_id) && record.phase == PlannerPhase::Finished {
        let text = format!("submit_epic was already accepted for epic {epic}");
        return refuse(fx, reply, text);
    }
    // M9.8 review, ruling 3: only the latest session, live and not ended, may call; a
    // queued re-plan's epic has no such session yet.
    if !record.is_live_caller(call.window_id) {
        let text = format!(
            "this window is not the sub-planner of epic {epic} of run {}",
            run.id
        );
        return refuse(fx, reply, text);
    }
    match parse_call(call.role, &call.tool, &call.args) {
        Ok(OrchCall::SubmitEpic { edits, note }) => submit_epic(
            run,
            reply,
            (k, call.window_id),
            (&edits, note),
            refusals,
            now,
            fx,
        ),
        Ok(_) => refuse(
            fx,
            reply,
            format!("tool {} is not available yet", call.tool),
        ),
        Err(text) => refuse(fx, reply, text),
    }
}

/// M9.8 review, ruling 2, as the second review rules it: a sub-planner changes only
/// its current session's round. It may `amend_task`, `split_task` or `add_dep` (as
/// the dependent) a task of its epic only when the task sits in the `Drafting` round
/// its session's additions join, and `cancel_task` one only when it is neither released
/// nor started, since cancelling unapproved work only reduces it. Before the gate
/// nothing is approved, so any unstarted task of its epic may change. Another epic's
/// task is `planner_confinement`'s; the orchestrator's and the user's rights are
/// decision 19's.
fn outside_its_round(run: &Run, edits: &[PlanEdit], epic: &str) -> Vec<PlanError> {
    let round = gate_holds::session_round(run, epic);
    let may_change = |t: &crate::run::model::Task, cancel: bool| {
        let started = t.session > 0
            || !matches!(
                t.state,
                TaskState::Pending | TaskState::Queued | TaskState::Blocked
            );
        if started {
            false
        } else if cancel {
            !gate_holds::released_past_gate(run, t)
        } else {
            !gate_holds::past_gate(run) || (t.orch.gate_hold.is_some() && t.orch.gate_hold == round)
        }
    };
    edits
        .iter()
        .filter_map(|edit| match edit {
            PlanEdit::AmendTask { task_id, .. }
            | PlanEdit::SplitTask { task_id, .. }
            | PlanEdit::AddDep { task_id, .. } => Some((task_id, false)),
            PlanEdit::CancelTask { task_id } => Some((task_id, true)),
            _ => None,
        })
        .filter(|(id, cancel)| {
            run.task(id)
                .is_some_and(|t| t.spec.epic.as_deref() == Some(epic) && !may_change(t, *cancel))
        })
        .map(|(id, _)| {
            let text = format!("task {id}: a sub-planner changes only its own round's tasks");
            PlanError::new(Some(id), "", "2.epic", text)
        })
        .collect()
}

/// The edit ops a sub-planner may not use (decision 22, TT §12.1), each with its
/// refusal; milestone 9.3 decision 30's `iterate` in its own words.
fn forbidden(edit: &PlanEdit) -> Option<String> {
    let op = match edit {
        PlanEdit::Answer { .. } => "answer",
        PlanEdit::Pause => "pause",
        PlanEdit::Resume => "resume",
        PlanEdit::Finish => "finish",
        PlanEdit::Message { .. } => "message",
        PlanEdit::Refresh { .. } => "refresh",
        PlanEdit::Iterate { .. } => return Some(ITERATE_BY_PLANNER.to_string()),
        _ => return None,
    };
    Some(format!("op {op} is not available to a sub-planner"))
}

/// Decision 22's `submit_epic` of epic `k`, from its live session in `window`: one
/// batch under the epic's area and decision 23's rules, confined to the epic's own
/// tasks, then held as the orchestrator's additions are. Accepted, the planner is
/// finished and its session retired; rejected, the error list, and the planner fails
/// at `max_rejections`.
fn submit_epic(
    run: &mut Run,
    reply: ReplyId,
    (k, window): (usize, u32),
    (edits, note): (&[PlanEdit], Option<String>),
    refusals: &[(Runtime, String)],
    now: u64,
    fx: &mut Vec<Effect>,
) {
    let epic = run.orch.epics[k].epic.clone();
    let source = EditSource::Planner { epic: epic.clone() };
    let refused = edits
        .iter()
        .find_map(forbidden)
        .or_else(|| kinds::engine_owned(run, edits));
    if let Some(text) = refused {
        record_rejected(run, edits, &source, text.clone(), now);
        return refuse(fx, reply, text);
    }
    let mut confined = planner_confinement(run, edits, &epic);
    confined.extend(outside_its_round(run, edits, &epic));
    if !confined.is_empty() {
        let first = confined
            .first()
            .map(ToString::to_string)
            .unwrap_or_default();
        record_rejected(run, edits, &source, first, now);
        return reject(run, reply, (k, window), &confined, now, fx);
    }
    let mut edited = run.clone();
    let mut effects = Vec::new();
    let scope = EditScope::Area {
        globs: run.orch.epics[k].area.clone(),
    };
    let batch = (edits, &scope, refusals);
    let added = match apply_batch(&mut edited, batch, &source, now, &mut effects) {
        Ok(Applied { added, .. }) => added,
        Err(Refused::Text(text)) => {
            record_rejected(run, edits, &source, text.clone(), now);
            return refuse(fx, reply, text);
        }
        Err(Refused::Plan(errors)) => {
            let first = errors.first().map(ToString::to_string).unwrap_or_default();
            record_rejected(run, edits, &source, first, now);
            return reject(run, reply, (k, window), &errors, now, fx);
        }
    };
    gate_holds::assign(&mut edited, edits, &added, now);
    let record = &mut edited.orch.epics[k];
    record.phase = PlannerPhase::Finished;
    record.ended_at = Some(now);
    record.edits_accepted += edits.len() as u32;
    if note.is_some() {
        record.note = note;
    }
    if let Some(session) = record.sessions.last_mut() {
        session.ended_at = Some(now);
    }
    history::planner_accepted(&mut edited, &epic);
    gate_holds::submit_epic_round(&mut edited, &epic, now);
    let n = edited
        .tasks
        .iter()
        .filter(|t| t.spec.epic.as_deref() == Some(epic.as_str()))
        .filter(|t| t.state != proto::TaskState::Cancelled)
        .count();
    let plural = if n == 1 { "" } else { "s" };
    log(
        &mut edited,
        now,
        format!("sub-planner {epic} submitted its epic"),
    );
    wake::note(
        &mut edited,
        format!("sub-planner {epic} finished with {n} task{plural}"),
    );
    *run = edited;
    fx.extend(effects);
    settle(run, now, fx);
    fx.push(Effect::Reply {
        reply,
        result: Ok(EPIC_RECORDED.to_string()),
    });
    fx.push(Effect::PlannerAccepted { window_id: window });
}

/// A rejected `submit_epic` (decision 22): counted, its first error kept, and the
/// planner failed and stopped once its session reached `max_rejections`.
fn reject(
    run: &mut Run,
    reply: ReplyId,
    (k, window): (usize, u32),
    errors: &[crate::run::plan::PlanError],
    now: u64,
    fx: &mut Vec<Effect>,
) {
    let max = run.limits.orch.planners.max_rejections;
    let record = &mut run.orch.epics[k];
    record.edits_rejected += 1;
    record.last_rejection = errors.first().map(|e| e.message.clone());
    let rejections = record.sessions.last_mut().map_or(0, |s| {
        s.rejections += 1;
        s.rejections
    });
    if rejections >= max {
        let reason = format!("the sub-planner's epic was rejected {rejections} times");
        fail(run, k, reason.clone(), now);
        fx.push(Effect::StopPlanner {
            window_id: window,
            reason,
        });
    }
    fx.push(Effect::Reply {
        reply,
        result: Err(rejected(errors)),
    });
}
