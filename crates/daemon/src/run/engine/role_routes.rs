//! Milestone 9 decision 43, engine side: a run's role-routing records
//! (`Run.role_routing_decisions`). A record is opened when its session is dispatched,
//! in the step that emits the session's start (so the step's `Persist` keeps it first);
//! it is finished once, when the session ends or a restore finds it open
//! (`interrupted`), and that finished line is appended to `history.jsonl` with M8b's
//! `OpKind::AppendHistory`, whose reconcile row makes the append idempotent by record
//! id. A run whose history is off keeps its records and appends none. Pure (design
//! decision 2).

use proto::{AgentRole, HistoryLine, RoleOutcome, RoleRoutingDecision};

use super::super::ScoutEnd;
use super::{Effect, append};
use crate::run::history::enabled;
use crate::run::model::Run;
use crate::run::orch::{RunScoutState, roles};

/// The session of an open record ended with the daemon that ran it.
pub const RESTARTED: &str = "the daemon restarted during this session";
/// A sub-planner's or run scout's session anthrex stopped because the run was ending
/// (`run cancel`, the `finish` edit): review I-1.
pub const STOPPED: &str = "stopped when the run ended";

/// Keeps `decision` open, unless a record of its id is kept already.
pub(in crate::run::engine) fn open(run: &mut Run, decision: RoleRoutingDecision) {
    let known = run
        .role_routing_decisions
        .iter()
        .any(|d| d.record_id == decision.record_id);
    if !known {
        run.role_routing_decisions.push(decision);
    }
}

/// Finishes the open record `record_id` and appends it. False when no record of that
/// id is open (unknown, or finished already).
pub(in crate::run::engine) fn close(
    run: &mut Run,
    record_id: &str,
    (outcome, result): (RoleOutcome, Option<String>),
    fx: &mut Vec<Effect>,
) -> bool {
    let Some(d) = run
        .role_routing_decisions
        .iter_mut()
        .find(|d| d.record_id == record_id && d.outcome.is_none())
    else {
        return false;
    };
    roles::finish(d, outcome, result);
    let line = HistoryLine::RoleRoute(d.clone());
    if enabled(run) {
        append(run, None, line, fx);
    }
    true
}

/// Closes the record of run-bound `role` session `session_id`.
pub(in crate::run::engine) fn close_session(
    run: &mut Run,
    (role, session_id): (AgentRole, &str),
    ended: (RoleOutcome, Option<String>),
    fx: &mut Vec<Effect>,
) {
    let id = roles::record_id(&run.id, role, session_id);
    close(run, &id, ended, fx);
}

/// The orchestrator's open record, if any (at most one is open at a time).
fn open_orchestrator(run: &Run) -> Option<String> {
    run.role_routing_decisions
        .iter()
        .rev()
        .find(|d| d.role == AgentRole::Orchestrator && d.outcome.is_none())
        .map(|d| d.record_id.clone())
}

/// The orchestrator's status for its record: how the session ended, whether the run's
/// plan had been submitted by then, and whether a summary was written. Never the run's
/// outcome.
fn orchestrator_result(run: &Run, ended: &str) -> String {
    let Some(o) = run.orch.orchestrator.as_ref() else {
        return ended.to_string();
    };
    // Re-review 4: `plan_submitted` is the run's; this session may not be the one
    // that submitted it.
    let plan = match o.plan_submitted {
        true => "the run's plan had been submitted",
        false => "no plan had been submitted",
    };
    let summary = match o.summary {
        Some(_) => "; summary written",
        None => "",
    };
    format!("{ended}; {plan}{summary}")
}

/// Decision 43: the orchestrator's session was dispatched with `trigger`.
pub(in crate::run::engine) fn orchestrator_dispatched(run: &mut Run, trigger: &str, now: u64) {
    if let Some(decision) = roles::orchestrator_record(run, trigger, now) {
        open(run, decision);
    }
}

/// The orchestrator's open session ended while the run goes on: `failed` with `why` (a
/// launch or restart that failed); or its window exited, which is `completed` when the
/// run's plan had been submitted by then (what an orchestrator session is for) and
/// `failed` when it had not (review M-1; re-review 4: the result says what is known of
/// the run's plan, not which session submitted it). The Interfaces' outcomes have no "exited early"; the result says it.
pub(in crate::run::engine) fn orchestrator_ended(
    run: &mut Run,
    failed: Option<&str>,
    fx: &mut Vec<Effect>,
) {
    let Some(id) = open_orchestrator(run) else {
        return;
    };
    let submitted = run
        .orch
        .orchestrator
        .as_ref()
        .is_some_and(|o| o.plan_submitted);
    let ended = match (failed, submitted) {
        (Some(why), _) => (RoleOutcome::Failed, Some(why.to_string())),
        (None, true) => (
            RoleOutcome::Completed,
            Some(
                "its window exited after the run's plan had been submitted, before the run ended"
                    .into(),
            ),
        ),
        (None, false) => (
            RoleOutcome::Failed,
            Some("its window exited before the run's plan was submitted".into()),
        ),
    };
    close(run, &id, ended, fx);
}

/// Every step: a run that ended ends its orchestrator's session, `completed` when its
/// window had started and `interrupted` when its launch had not answered yet.
pub(in crate::run::engine) fn pass(run: &mut Run, fx: &mut Vec<Effect>) {
    if !run.state.is_terminal() {
        return;
    }
    let Some(id) = open_orchestrator(run) else {
        return;
    };
    let started = run
        .orch
        .orchestrator
        .as_ref()
        .is_some_and(|o| o.window_id.is_some());
    let ended = match started {
        true => (
            RoleOutcome::Completed,
            Some(orchestrator_result(run, "live until the run ended")),
        ),
        false => (
            RoleOutcome::Interrupted,
            Some("the run ended before its window started".to_string()),
        ),
    };
    close(run, &id, ended, fx);
}

/// Decision 43 on `Restore`: every session the old daemon ran is over, so each open
/// record is finished `interrupted` and appended, once (a finished record is never
/// finished again).
pub(in crate::run::engine) fn interrupt_open(run: &mut Run, fx: &mut Vec<Effect>) {
    let open: Vec<(String, String)> = run
        .role_routing_decisions
        .iter()
        .filter(|d| d.outcome.is_none())
        .map(|d| {
            let text = match &d.result {
                Some(status) => format!("{status}; {RESTARTED}"),
                None => RESTARTED.to_string(),
            };
            (d.record_id.clone(), text)
        })
        .collect();
    for (id, text) in open {
        close(run, &id, (RoleOutcome::Interrupted, Some(text)), fx);
    }
}

/// Epic `epic`'s latest planner session's record notes that its `submit_epic` was
/// accepted; the session's end finishes it `completed`.
pub(in crate::run::engine) fn planner_accepted(run: &mut Run, epic: &str) {
    let Some(session) = run
        .orch
        .epics
        .iter()
        .find(|e| e.epic == epic)
        .and_then(|e| e.sessions.last())
        .map(|s| s.session)
    else {
        return;
    };
    let id = roles::record_id(&run.id, AgentRole::Planner, &format!("{epic}/{session}"));
    if let Some(d) = run
        .role_routing_decisions
        .iter_mut()
        .find(|d| d.record_id == id && d.outcome.is_none())
    {
        d.result = Some(ACCEPTED.to_string());
    }
}

/// A planner record's result once its epic was accepted.
const ACCEPTED: &str = "epic accepted";

/// Session `session` of epic `epic`'s sub-planner ended (`Ok`: the machine's outcome)
/// or could not start (`Err`). `completed` when its `submit_epic` was accepted, else
/// `failed`; the result says which, with its rejected submissions.
pub(in crate::run::engine) fn planner_ended(
    run: &mut Run,
    (epic, session): (&str, u32),
    outcome: Result<&ScoutEnd, String>,
    fx: &mut Vec<Effect>,
) {
    let id = roles::record_id(&run.id, AgentRole::Planner, &format!("{epic}/{session}"));
    let accepted = run
        .role_routing_decisions
        .iter()
        .any(|d| d.record_id == id && d.result.as_deref() == Some(ACCEPTED));
    let rejections = run
        .orch
        .epics
        .iter()
        .find(|e| e.epic == epic)
        .and_then(|e| e.sessions.iter().find(|s| s.session == session))
        .map_or(0, |s| s.rejections);
    let rejected = match rejections {
        0 => String::new(),
        n => format!("; {n} submissions rejected"),
    };
    let ended = match (outcome, accepted) {
        (Ok(ScoutEnd::Reported), true) => (RoleOutcome::Completed, ACCEPTED.to_string()),
        (Ok(ScoutEnd::Reported), false) => {
            (RoleOutcome::Failed, "ended without an accepted epic".into())
        }
        (Ok(ScoutEnd::Failed { reason }), true) => {
            (RoleOutcome::Failed, format!("{ACCEPTED}; then {reason}"))
        }
        (Ok(ScoutEnd::Failed { reason }), false) => (RoleOutcome::Failed, reason.clone()),
        (Err(why), _) => (RoleOutcome::Failed, why),
    };
    let (outcome, text) = ended;
    close(run, &id, (outcome, Some(format!("{text}{rejected}"))), fx);
}

/// Review I-1 (narrowed by the re-review, 1): `run cancel` or the `finish` edit
/// stopped this session (`planners::halt_all`: a planning epic's latest session, a
/// running scout). It did not fail: its open record is finished `interrupted`,
/// [`STOPPED`], before the stopped session reports its end. A session `halt_all` does
/// not stop (an accepted planner still ending, one the engine stopped at
/// `max_rejections`) keeps its own end.
pub(in crate::run::engine) fn session_stopped(
    run: &mut Run,
    (role, session_id): (AgentRole, &str),
    fx: &mut Vec<Effect>,
) {
    let ended = (RoleOutcome::Interrupted, Some(STOPPED.to_string()));
    close_session(run, (role, session_id), ended, fx);
}

/// `OrchEvent::RoleRoute`: the record is kept, unless it is a run scout's whose scout
/// is no longer running (stopped when the run ended, re-review 2): that session will
/// not start, so nothing is recorded, and the refusal tells the driver not to start it.
/// A kept record's `log` line goes to the run log (ruling T10b-1).
pub(in crate::run::engine) fn keep(
    run: &mut Run,
    decision: RoleRoutingDecision,
    (log, now): (Option<String>, u64),
) -> Result<String, String> {
    if decision.role == AgentRole::Scout {
        let id = decision.session_id.as_str();
        let state = run
            .orch
            .run_scouts
            .iter()
            .find(|s| s.id == id)
            .map(|s| &s.state);
        match state {
            Some(RunScoutState::Running) => {}
            Some(RunScoutState::Failed { .. }) => {
                return Err(format!("scout {id} was {STOPPED}"));
            }
            _ => return Err(format!("scout {id} is not running")),
        }
    }
    open(run, decision);
    if let Some(line) = log {
        crate::run::engine::requests::log(run, now, line);
    }
    Ok("recorded".to_string())
}
