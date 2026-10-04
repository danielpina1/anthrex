//! Milestone 9.6 decisions 9 and 10 (DF §3), engine side: the design agents, headless
//! read-only sessions on M8b's scout machine, which follow the sub-planner pattern
//! (`planners.rs`). Task M9.6.8 runs the two brainstormers; task M9.6.10 adds the
//! document reviewer.
//!
//! - **Launch.** `start_brainstorm` queues both brainstormers ([`queue_brainstormers`],
//!   decision 10's picks). Each takes a reader slot ([`readers`], counted by
//!   `schedule::readers_busy`) and starts from `planners::dispatch`'s loop, after the
//!   sub-planners and before the run scouts ([`start_next`]), as
//!   `OpKind::StartDesignAgent`; its result binds its window ([`started`]).
//! - **The draft.** A brainstormer's one write, `submit_doc` kind `brainstorm_draft`, is
//!   accepted only from its live window ([`tool`], task 6's review c; a call that comes
//!   before its launch's result is held, `early.rs`), checked against its template and
//!   held in memory, neither stored nor written while the other brainstormer runs
//!   (ruling T8-1); its session is then retired (`Effect::PlannerAccepted`, which the
//!   scout service applies to any tagged session).
//! - **The end.** Each brainstormer ends `Submitted` (its draft held) or `Failed` (a
//!   launch failure, a crash, over budget; [`ended`]). When both have ended, the held
//!   drafts are stored and written (`Done`), and one draft or two wake the orchestrator
//!   (`design::drafts_in`); two failures halt the run, retryably; `run resume`
//!   relaunches them ([`relaunch_failed`], DF §3.5).
//! - **Restart.** A brainstormer running, or holding its draft, at a daemon restart is
//!   queued again and relaunched fresh, as a new session, with the same pack once the
//!   run resumes ([`restore`], DF §8.4).
//!
//! Pure (design decision 2).

use proto::{AgentRole, RoleOutcome, RunState, TokenUsage};

use super::requests::log;
use super::{
    Effect, EngineState, OpKind, OpResult, OrchEvent, ScoutEnd, design, emit_op, history, next_op,
};
use crate::decider::DeciderCaps;
use crate::run::design::pack::{EarlierSpec, freeze};
use crate::run::design::state::{DesignAgent, DesignAgentState};
use crate::run::model::Run;
use crate::run::orch::roles;
use crate::run::orch::roles::lists::{brainstorm_picks_with, unsaved_missing};
use crate::scout::design_spec::{DesignAgentSpec, brainstormer_spec};

/// A brainstormer's session that ended with no accepted draft and no failure of its own.
pub const NO_DRAFT: &str = "the brainstormer ended without an accepted draft";
/// A brainstormer stopped because the user rejected its run.
pub const RUN_REJECTED: &str = "the run was rejected";
/// A brainstormer's record's result once its draft was accepted.
const DRAFT_ACCEPTED: &str = "draft accepted";

/// The design agents holding reader slots: started and not ended.
pub(crate) fn readers(run: &Run) -> usize {
    let Some(design) = run.orch.design.as_ref() else {
        return 0;
    };
    (design.brainstormers.iter().chain(&design.reviewer))
        .filter(|a| a.state == DesignAgentState::Running)
        .count()
}

/// `start_brainstorm` was accepted: decision 10's two brainstormers, queued for reader
/// slots, and their pack's inputs frozen with `earlier` (ruling T8-2).
pub(super) fn queue_brainstormers(
    run: &mut Run,
    earlier: Option<EarlierSpec>,
    caps: &DeciderCaps,
    now: u64,
) {
    let picks = brainstorm_picks_with(run, caps);
    for runtime in unsaved_missing(run, caps) {
        let label = runtime.label();
        let text = format!(
            "brainstormer on {label} skipped: its CLI cannot run without saving the session"
        );
        log(run, now, text);
    }
    let pack = freeze(run, earlier);
    let Some(design) = run.orch.design.as_mut() else {
        return;
    };
    design.pack = Some(pack);
    design.drafts_settled = false;
    design.brainstormers = (picks.iter())
        .map(|p| DesignAgent {
            label: p.label.clone(),
            role: AgentRole::Brainstormer,
            route: p.route.clone(),
            session: 0,
            window_id: None,
            state: DesignAgentState::Queued,
            calls: 0,
            tokens: 0,
            started: None,
            listed: p.listed,
        })
        .collect();
    let named: Vec<String> = (picks.iter())
        .map(|p| format!("{} on {}", p.label, show_route(&p.route)))
        .collect();
    log(
        run,
        now,
        format!("brainstormers queued: {}", named.join(", ")),
    );
}

fn show_route(route: &proto::Route) -> String {
    match route.model.is_empty() {
        true => route.runtime.label().to_string(),
        false => format!("{} {}", route.runtime.label(), route.model),
    }
}

/// One queued brainstormer starts in a free reader slot, while the run brainstorms;
/// false when none is queued (`planners::dispatch`'s loop then tries the run scouts).
pub(super) fn start_next(run: &mut Run, now: u64, fx: &mut Vec<Effect>) -> bool {
    if run.state != RunState::Brainstorming {
        return false;
    }
    let queued = (run.orch.design.as_ref())
        .and_then(|d| (d.brainstormers.iter()).position(|a| a.state == DesignAgentState::Queued));
    let Some(k) = queued else {
        return false;
    };
    start(run, k, now, fx);
    true
}

/// Brainstormer `k`'s next session, its record opened before its start.
fn start(run: &mut Run, k: usize, now: u64, fx: &mut Vec<Effect>) {
    let op = next_op(run);
    let Some(agent) = run.orch.design.as_mut().map(|d| &mut d.brainstormers[k]) else {
        return;
    };
    agent.session += 1;
    agent.state = DesignAgentState::Running;
    agent.window_id = None;
    agent.started = Some(now);
    let agent = agent.clone();
    let spec = brainstormer_spec(run, &agent);
    let text = format!(
        "brainstormer {} session {} starting",
        agent.label, agent.session
    );
    log(run, now, text);
    history::open(run, roles::design_agent_record(run, &agent, now));
    let kind = OpKind::StartDesignAgent {
        spec: Box::new(spec),
    };
    emit_op(run, op, None, kind, fx);
}

/// The brainstormer a spec launched: its index, when that launch is its latest session.
fn latest(run: &Run, spec: &DesignAgentSpec) -> Option<usize> {
    let design = run.orch.design.as_ref()?;
    (design.brainstormers.iter())
        .position(|a| a.label == spec.kind.label() && a.session == spec.session)
}

/// `StartDesignAgent`'s result: the session's window, or its failure. Returns the
/// window a session was bound to, for the calls held before it (`early.rs`). A window
/// whose agent is no longer running (halted while its launch was in flight) is stopped.
pub(super) fn started(
    run: &mut Run,
    kind: &OpKind,
    result: OpResult,
    now: u64,
    fx: &mut Vec<Effect>,
) -> Option<u32> {
    let OpKind::StartDesignAgent { spec } = kind else {
        return None;
    };
    let k = latest(run, spec);
    let live = |run: &Run, k: usize| {
        (run.orch.design.as_ref())
            .is_some_and(|d| d.brainstormers[k].state == DesignAgentState::Running)
    };
    if let OpResult::DesignAgentStarted {
        pack: Some(file), ..
    } = &result
    {
        pack::record(run, file.clone());
    }
    match (result, k) {
        (OpResult::DesignAgentStarted { window_id, .. }, Some(k)) if live(run, k) => {
            if let Some(design) = run.orch.design.as_mut() {
                design.brainstormers[k].window_id = Some(window_id);
            }
            Some(window_id)
        }
        (OpResult::DesignAgentStarted { window_id, .. }, _) => {
            let reason = "the brainstormer is no longer running".to_string();
            fx.push(Effect::StopPlanner { window_id, reason });
            None
        }
        (OpResult::DesignPackUnreadable { reason }, k) => {
            let session = format!("{}/{}", spec.kind.label(), spec.session);
            let k = k.filter(|&k| live(run, k));
            pack::unreadable(run, (&session, k), reason, now, fx);
            None
        }
        (OpResult::Failed { message }, k) => {
            let why = format!("the brainstormer could not start: {message}");
            let session = format!("{}/{}", spec.kind.label(), spec.session);
            let failed = (RoleOutcome::Failed, Some(why.clone()));
            history::close_session(run, (AgentRole::Brainstormer, &session), failed, fx);
            if let Some(k) = k.filter(|&k| live(run, k)) {
                fail(run, k, why, now, fx);
            }
            None
        }
        _ => None,
    }
}

/// `OrchEvent::DesignAgentEnded`, to its run's [`ended`].
pub(super) fn on_ended(state: &mut EngineState, event: OrchEvent, now: u64, fx: &mut Vec<Effect>) {
    let OrchEvent::DesignAgentEnded {
        run_id,
        role,
        label,
        session,
        outcome,
        usage,
        calls,
    } = event
    else {
        return;
    };
    if let Some(run) = state.runs.get_mut(&run_id) {
        ended(
            run,
            (role, &label, session),
            (outcome, usage, calls),
            now,
            fx,
        );
    }
}

/// A design agent's session ended: its usage and calls are counted; a brainstormer
/// still running fails (its draft never came: a crash, over budget, or a quiet end),
/// and its record is finished.
pub(super) fn ended(
    run: &mut Run,
    (role, label, session): (AgentRole, &str, u32),
    (outcome, usage, calls): (ScoutEnd, TokenUsage, u32),
    now: u64,
    fx: &mut Vec<Effect>,
) {
    if role != AgentRole::Brainstormer {
        return;
    }
    let Some(design) = run.orch.design.as_mut() else {
        return;
    };
    let Some(k) = (design.brainstormers.iter()).position(|a| a.label == label) else {
        return;
    };
    let agent = &mut design.brainstormers[k];
    if agent.session != session {
        return;
    }
    agent.tokens += usage.input + usage.output + usage.cache_read + usage.cache_write;
    agent.calls = calls;
    let done = matches!(
        agent.state,
        DesignAgentState::Submitted | DesignAgentState::Done
    );
    let reason = match outcome {
        ScoutEnd::Failed { reason } => reason,
        ScoutEnd::Reported => NO_DRAFT.to_string(),
    };
    let record = format!("{label}/{session}");
    let closed = match done {
        true => (RoleOutcome::Completed, Some(DRAFT_ACCEPTED.to_string())),
        false => (RoleOutcome::Failed, Some(reason.clone())),
    };
    history::close_session(run, (AgentRole::Brainstormer, &record), closed, fx);
    let running = (run.orch.design.as_ref())
        .is_some_and(|d| d.brainstormers[k].state == DesignAgentState::Running);
    if running {
        fail(run, k, reason, now, fx);
    }
}

/// Brainstormer `k` fails with `reason` (DF §3.5), and the brainstorm settles.
fn fail(run: &mut Run, k: usize, reason: String, now: u64, fx: &mut Vec<Effect>) {
    let Some(design) = run.orch.design.as_mut() else {
        return;
    };
    let agent = &mut design.brainstormers[k];
    agent.state = DesignAgentState::Failed(reason.clone());
    let text = format!("brainstormer {} failed: {reason}", agent.label);
    log(run, now, text);
    settle(run, now, fx);
}

/// DF §3.4 and §3.5, once every brainstormer has ended, in brainstorming (a paused
/// run's brainstorm settles when it resumes, `restore::unpause`, fix round 1's m1): the
/// held drafts are stored and written (ruling T8-1); a draft or two wake the
/// orchestrator, with the failure if one failed, once per brainstorm round
/// (`drafts_settled`, ruling T8-5, cleared when brainstormers are queued anew); two
/// failures halt the run, retryably (`run resume` relaunches them, [`relaunch_failed`]).
pub(super) fn settle(run: &mut Run, now: u64, fx: &mut Vec<Effect>) {
    if run.state != RunState::Brainstorming {
        return;
    }
    let Some(design) = run.orch.design.as_ref() else {
        return;
    };
    // Ruling T8-5: once per brainstorm round.
    if design.drafts_settled {
        return;
    }
    let ended = |a: &DesignAgent| {
        matches!(
            a.state,
            DesignAgentState::Submitted | DesignAgentState::Done | DesignAgentState::Failed(_)
        )
    };
    if design.brainstormers.is_empty() || !design.brainstormers.iter().all(ended) {
        return;
    }
    drafts::flush(run, now, fx);
    let Some(design) = run.orch.design.as_ref() else {
        return;
    };
    let failed: Vec<(String, String)> = (design.brainstormers.iter())
        .filter_map(|a| match &a.state {
            DesignAgentState::Failed(reason) => Some((a.label.clone(), reason.clone())),
            _ => None,
        })
        .collect();
    if failed.len() < design.brainstormers.len() {
        if let Some(design) = run.orch.design.as_mut() {
            design.drafts_settled = true;
        }
        let one = failed.first().map(|(l, r)| (l.as_str(), r.as_str()));
        return design::drafts_in(run, one, now);
    }
    let reasons: Vec<&str> = failed.iter().map(|(_, r)| r.as_str()).collect();
    let text = format!(
        "design flow: both brainstormers failed: {}",
        reasons.join("; ")
    );
    if let Some(design) = run.orch.design.as_mut() {
        design.halted_from = Some(RunState::Brainstorming);
        design.phase_started = None;
    }
    // Task 7's concern 5: halted as every halt is (its log line and wake note).
    super::merge::halt(run, text, now);
    run.halt_retryable = true;
}

/// DF §3.5: `run resume` of a run both brainstormers' failures halted queues them both
/// again (a fresh session each, with the same pack). True when it did; the
/// brainstorming clock then waits for their drafts again.
pub(super) fn relaunch_failed(run: &mut Run, now: u64) -> bool {
    let Some(design) = run.orch.design.as_mut() else {
        return false;
    };
    let all_failed = !design.brainstormers.is_empty()
        && (design.brainstormers.iter()).all(|a| matches!(a.state, DesignAgentState::Failed(_)));
    if !all_failed {
        return false;
    }
    for agent in design.brainstormers.iter_mut() {
        agent.state = DesignAgentState::Queued;
        agent.window_id = None;
    }
    design.phase_started = None;
    log(run, now, "the brainstormers relaunch");
    true
}

/// Decision 9 after a daemon restart (DF §8.4): a brainstormer that was running, or
/// whose draft was held (ruling T8-1: lost with the old daemon's memory), is queued
/// again, so it is relaunched fresh, as a new session with the same pack, once the run
/// resumes; nothing is killed (its `StartDesignAgent` reconciles as `NotStarted`, and
/// its record is finished `interrupted` by the restore).
pub(super) fn restore(run: &mut Run, now: u64) {
    let Some(design) = run.orch.design.as_mut() else {
        return;
    };
    let mut again = Vec::new();
    for agent in design.brainstormers.iter_mut() {
        if matches!(
            agent.state,
            DesignAgentState::Running | DesignAgentState::Submitted
        ) {
            agent.state = DesignAgentState::Queued;
            agent.window_id = None;
            again.push(agent.label.clone());
        }
    }
    design.held.clear();
    for label in again {
        let text = format!("brainstormer {label} relaunches after a daemon restart");
        log(run, now, text);
    }
}

/// `planners::halt_all`: `run cancel`, the `finish` edit or a rejected round ends the
/// brainstormers too. A live one is stopped, a queued one never starts; each fails
/// with `reason`, and the brainstorm does not settle (the run is ending).
pub(super) fn halt_all(run: &mut Run, reason: &str, fx: &mut Vec<Effect>) {
    let Some(design) = run.orch.design.as_mut() else {
        return;
    };
    let mut stopped = Vec::new();
    for agent in design.brainstormers.iter_mut() {
        if !matches!(
            agent.state,
            DesignAgentState::Running | DesignAgentState::Queued | DesignAgentState::Submitted
        ) {
            continue;
        }
        if agent.state == DesignAgentState::Running {
            stopped.push(format!("{}/{}", agent.label, agent.session));
            if let Some(window_id) = agent.window_id {
                let reason = reason.to_string();
                fx.push(Effect::StopPlanner { window_id, reason });
            }
        }
        agent.state = DesignAgentState::Failed(reason.to_string());
    }
    design.held.clear();
    for session in stopped {
        history::session_stopped(run, (AgentRole::Brainstormer, &session), fx);
    }
}

#[path = "design_drafts.rs"]
mod drafts;
pub(super) use drafts::tool;
#[path = "design_pack.rs"]
mod pack;
pub(super) use pack::awaiting_drafts;
