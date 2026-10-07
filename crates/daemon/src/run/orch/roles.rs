//! Milestone 9 decision 43: the role-routing records of the sessions that belong to no
//! task — the orchestrator, a sub-planner, a run scout and a decider (pre-run triage
//! included). Each is made when its session is dispatched, with the full ordered
//! candidate snapshot it was chosen from, and finished once with the session's factual
//! outcome. Pure: the engine keeps a run's records in `Run.role_routing_decisions` and
//! appends each finished one to `history.jsonl` as `OpKind::AppendHistory`; the driver
//! appends pre-run triage's itself (`driver/adapt_goal.rs`).
//!
//! **Record ids.** `<run id>/<role>/<session id>` for a run's session, and
//! `triage/<request>/<n>` for pre-run triage (and `run_name/<request>/<n>` for the run
//! name), which have no run. A task's record is
//! `<run id>/<task id>` (one `/`), a run's and a revert's have their own prefixes, so no
//! role record can take another record's id; `history_io` keeps the last line of each.
//!
//! **Session ids.** The orchestrator's is the n-th orchestrator session dispatched in
//! the run (a launch, a relaunch after a failed launch, an engine restart or the user's
//! own `anthrex restart` each count), a sub-planner's `<epic>/<session>`, a run scout's
//! its scout id (a scout id is used once per run: a retry is a new scout), a run-bound
//! decider's its `Decide` op id.
//!
//! **Skip reasons.** Only [`NOT_INSTALLED`], [`NOT_CONFIGURED`] and [`EARLIER_TAKEN`]:
//! an unchosen candidate is never labelled a failure.

use proto::models::{Role, RoleChoice};
use proto::{
    AgentRole, HISTORY_VERSION, RoleOutcome, RoleRoutingDecision, RoleRoutingInput, Route,
    RoutingCandidate,
};
use serde::{Deserialize, Serialize};

use crate::run::design::state::DesignAgent;
use crate::run::model::Run;
use crate::run::model_roles::RunModels;
use crate::run::routing::{ROLE_TABLE, ROLES_POLICY};

pub const NOT_INSTALLED: &str = "not installed";
pub const NOT_CONFIGURED: &str = "not in the configured list";
pub const EARLIER_TAKEN: &str = "an earlier candidate was taken";

pub const ORCHESTRATOR_POLICY: &str = "m9-orchestrator-v1";
pub const PLANNER_POLICY: &str = "m9-planner-v1";
pub const SCOUT_POLICY: &str = "m9-scout-v1";
pub const DECIDER_POLICY: &str = "m9-decider-v1";
/// Milestone 9.6 decision 10: a brainstormer's.
pub const BRAINSTORMER_POLICY: &str = "m9.6-brainstormer-v1";
/// Milestone 9.6 decision 10 (task M9.6.10): the document reviewer's policy.
pub const DOC_REVIEWER_POLICY: &str = "m9.6-doc-reviewer-v1";

/// A run record's goal cap (`history::GOAL_CHARS`), which the input's goal shares.
const GOAL_CHARS: usize = 200;

/// Decision 6's resolution as the orchestrator's record keeps it
/// (`OrchestratorRecord.routing`): every launch and restart of the run's orchestrator
/// is recorded with the snapshot taken when its route was resolved, whatever the
/// configuration says later.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RoleSnapshot {
    /// `explicit_choice`, `continued_chain`, `configured_list`, `agent_config` or
    /// `roster_default`; empty on a record made before milestone 9's task M9.13b.
    pub source: String,
    pub candidates: Vec<RoutingCandidate>,
}

/// A role's name in a record id.
pub fn role_name(role: AgentRole) -> &'static str {
    match role {
        AgentRole::Orchestrator => "orchestrator",
        AgentRole::Worker => "worker",
        AgentRole::Reviewer => "reviewer",
        AgentRole::Scout => "scout",
        AgentRole::Planner => "planner",
        AgentRole::Decider => "decider",
        AgentRole::Racer => "racer",
        AgentRole::TestWriter => "test_writer",
        AgentRole::Brainstormer => "brainstormer",
        AgentRole::DocReviewer => "doc_reviewer",
    }
}

/// `<run id>/<role>/<session id>`: the id of a run-bound session's record.
pub fn record_id(run_id: &str, role: AgentRole, session_id: &str) -> String {
    format!("{run_id}/{}/{session_id}", role_name(role))
}

/// A goal as a record's input keeps it: capped as a run record's is.
pub fn capped_goal(goal: &str) -> String {
    goal.chars().take(GOAL_CHARS).collect()
}

/// The run's part of every record's input: its path, its goal capped as a run
/// record's is, and its profile languages. Never a transcript or a credential.
pub fn input_of(run: &Run) -> RoleRoutingInput {
    RoleRoutingInput {
        run_path: run.path,
        goal: Some(capped_goal(&run.goal)),
        languages: run.profile_languages.clone(),
        ..RoleRoutingInput::default()
    }
}

/// Decision 43's record of one dispatch, open (no outcome). `chosen` is appended to
/// `candidates` when absent and `selected_index` points at it; the chosen candidate
/// carries no skip reason, and an unchosen one without a reason gets
/// [`EARLIER_TAKEN`] after the chosen one and [`NOT_CONFIGURED`] before it.
#[allow(
    clippy::too_many_arguments,
    reason = "one argument per field of decision 43's record; every caller names each"
)]
pub fn record(
    run: Option<&Run>,
    role: AgentRole,
    session_id: &str,
    trigger: &str,
    source: &str,
    policy: &str,
    input: RoleRoutingInput,
    mut candidates: Vec<RoutingCandidate>,
    chosen: &Route,
    now: u64,
) -> RoleRoutingDecision {
    let selected = match candidates.iter().position(|c| &c.route == chosen) {
        Some(i) => i,
        None => {
            candidates.push(RoutingCandidate {
                route: chosen.clone(),
                skipped_reason: None,
            });
            candidates.len() - 1
        }
    };
    for (i, c) in candidates.iter_mut().enumerate() {
        if i == selected {
            c.skipped_reason = None;
        } else if c.skipped_reason.is_none() {
            let reason = if i > selected {
                EARLIER_TAKEN
            } else {
                NOT_CONFIGURED
            };
            c.skipped_reason = Some(reason.to_string());
        }
    }
    let record_id = match run {
        Some(run) => record_id(&run.id, role, session_id),
        // Pre-run triage's `triage/...`, and the run name's `run_name/...`.
        None => format!("{trigger}/{session_id}"),
    };
    RoleRoutingDecision {
        v: HISTORY_VERSION,
        record_id,
        at: now,
        run_id: run.map(|r| r.id.clone()),
        task_id: None,
        role,
        session_id: session_id.to_string(),
        trigger: trigger.to_string(),
        source: source.to_string(),
        policy_version: policy.to_string(),
        pick_policy: None,
        rotation: None,
        input,
        chosen: chosen.clone(),
        selected_index: selected as u32,
        candidates,
        outcome: None,
        result: None,
    }
}

/// The session ended: its factual outcome and the role's own status. A record is
/// finished once; a later call leaves it as it is.
pub fn finish(decision: &mut RoleRoutingDecision, outcome: RoleOutcome, result: Option<String>) {
    if decision.outcome.is_none() {
        decision.outcome = Some(outcome);
        decision.result = result;
    }
}

/// Milestone 9.8 (ruling F16): a role record's candidates, `choice`'s model at its
/// effort, then its fallback at its default effort.
pub fn row_candidates(choice: &RoleChoice) -> Vec<RoutingCandidate> {
    let own = RunModels::route_of(&choice.model, choice.effort.as_deref());
    let fallback = (choice.fallback.as_ref()).map(|f| RunModels::route_of(f, None));
    (std::iter::once(own).chain(fallback))
        .map(|route| RoutingCandidate {
            route,
            skipped_reason: None,
        })
        .collect()
}

/// M9.17 fix round 3: each candidate on a runtime the run's start found not installed
/// (`run.orch.installed`) is skipped as [`NOT_INSTALLED`], its factual reason.
pub fn mark_not_installed(
    mut candidates: Vec<RoutingCandidate>,
    installed: &std::collections::BTreeMap<String, bool>,
) -> Vec<RoutingCandidate> {
    for c in &mut candidates {
        if installed.get(c.route.runtime.label()) == Some(&false) {
            c.skipped_reason = Some(NOT_INSTALLED.to_string());
        }
    }
    candidates
}

/// The run's next orchestrator session id: one more than its orchestrator records.
fn next_orchestrator_session(run: &Run) -> u32 {
    let n = run
        .role_routing_decisions
        .iter()
        .filter(|d| d.role == AgentRole::Orchestrator)
        .count();
    n as u32 + 1
}

/// The record of the orchestrator session about to be dispatched with `trigger`
/// (`start`, `promote`, `retry` or `restart`), from its route's kept snapshot.
pub fn orchestrator_record(run: &Run, trigger: &str, now: u64) -> Option<RoleRoutingDecision> {
    let o = run.orch.orchestrator.as_ref()?;
    let session = next_orchestrator_session(run).to_string();
    let source = match o.routing.source.as_str() {
        "" => "unrecorded",
        source => source,
    };
    Some(record(
        Some(run),
        AgentRole::Orchestrator,
        &session,
        trigger,
        source,
        ROLES_POLICY,
        input_of(run),
        o.routing.candidates.clone(),
        &o.route,
        now,
    ))
}

/// The record of session `session` of epic `k`'s sub-planner (milestone 9.8, ruling
/// F16): the run's `planner` row, source `role_table`.
pub fn planner_record(run: &Run, k: usize, session: u32, now: u64) -> RoleRoutingDecision {
    let epic = &run.orch.epics[k];
    let row = row_candidates(run.limits.models().choice(Role::Planner));
    let candidates = mark_not_installed(row, &run.orch.installed);
    let mut input = input_of(run);
    input.epic = Some(epic.epic.clone());
    input.area = epic.area.clone();
    let trigger = if session == 1 { "start" } else { "replan" };
    record(
        Some(run),
        AgentRole::Planner,
        &format!("{}/{session}", epic.epic),
        trigger,
        ROLE_TABLE,
        ROLES_POLICY,
        input,
        candidates,
        &epic.route,
        now,
    )
}

/// Milestone 9.6 decision 10: the record of brainstormer `agent`'s current session,
/// `<label>/<session>`, over the run's `brainstorm` row (milestone 9.8, ruling F16:
/// source `role_table`, its two models at its effort, a missing runtime's marked so). A
/// document reviewer (task M9.6.10) is the `reviewer` row's pick against the
/// orchestrator (decision 27), its row's model and fallback the candidates, else the
/// orchestrator's own route (`same_runtime`, its review's record).
///
/// Its trigger is `start` for a first session, `rethink` for the first session of a
/// rethink's round (`rethink`, ruling T13-1), else `relaunch` (ruling T8-7's relaunch,
/// a restart's, a resume's).
pub fn design_agent_record(
    run: &Run,
    agent: &DesignAgent,
    rethink: bool,
    now: u64,
) -> RoleRoutingDecision {
    let trigger = match (agent.session, rethink) {
        (1, _) => "start",
        (_, true) => "rethink",
        _ => "relaunch",
    };
    let models = run.limits.models();
    let (source, candidates) = if agent.role == AgentRole::DocReviewer {
        // WB-C M-2 (the final fix wave's FW-44): its pick's own reason, as its review
        // record (and the version it reviews) says it, never a route compared now.
        let design = run.orch.design.as_ref();
        let review = design.and_then(|d| {
            (d.reviews.iter()).rfind(|r| format!("{}-r{}", r.doc.label(), r.n) == agent.label)
        });
        let same = review.is_some_and(|r| r.same_runtime);
        let source = if same { "same_runtime" } else { ROLE_TABLE };
        (source, row_candidates(models.choice(Role::Reviewer)))
    } else {
        let b = &models.brainstorm;
        let pair = [&b.first, &b.second].map(|m| RoutingCandidate {
            route: RunModels::route_of(m, b.effort.as_deref()),
            skipped_reason: None,
        });
        (ROLE_TABLE, pair.to_vec())
    };
    let candidates = mark_not_installed(candidates, &run.orch.installed);
    record(
        Some(run),
        agent.role,
        &format!("{}/{}", agent.label, agent.session),
        trigger,
        source,
        ROLES_POLICY,
        input_of(run),
        candidates,
        &agent.route,
        now,
    )
}

/// The record of run scout `scout_id`'s session, on the route the driver starts it on
/// (`launch::scout_route`: the run's `research` row, milestone 9.8).
pub fn scout_record(run: &Run, scout_id: &str, now: u64) -> RoleRoutingDecision {
    let chosen = crate::run::orch::launch::scout_route(run);
    let row = row_candidates(run.limits.models().choice(Role::Research));
    let candidates = mark_not_installed(row, &run.orch.installed);
    let mut input = input_of(run);
    if let Some(scout) = run.orch.run_scouts.iter().find(|s| s.id == scout_id) {
        input.area = scout.area.clone();
    }
    record(
        Some(run),
        AgentRole::Scout,
        scout_id,
        "start",
        ROLE_TABLE,
        ROLES_POLICY,
        input,
        candidates,
        &chosen,
        now,
    )
}

/// A decider's record (`run`: a run-bound decider; `None`: pre-run triage, whose
/// `session_id` is `<request>/<n>`), over its helper row (milestone 9.8, ruling F16:
/// source `role_table`). Its trigger and question kind are the request's kind;
/// `candidates` are the row's ([`crate::decider::call::Routed::candidates`]).
/// `task_id` is the task when the call is about exactly one; a call about several (a
/// size check of a batch) is run-level and names none (review M-4).
pub fn decider_record(
    run: Option<&Run>,
    (session_id, kind): (&str, &str),
    task_ids: &[String],
    (route, candidates): (&Route, Vec<RoutingCandidate>),
    input: RoleRoutingInput,
    now: u64,
) -> RoleRoutingDecision {
    let mut input = input;
    input.question_kind = Some(kind.to_string());
    let mut decision = record(
        run,
        AgentRole::Decider,
        session_id,
        kind,
        ROLE_TABLE,
        ROLES_POLICY,
        input,
        candidates,
        route,
        now,
    );
    decision.task_id = match task_ids {
        [one] => Some(one.clone()),
        _ => None,
    };
    decision
}

/// [`decider_record`] for a call [`crate::decider::call::routed`] routed: its route and
/// its row's candidates.
pub fn decider_routed(
    run: Option<&Run>,
    session: (&str, &str),
    task_ids: &[String],
    routed: &crate::decider::call::Routed,
    (input, now): (RoleRoutingInput, u64),
) -> RoleRoutingDecision {
    let chosen = (&routed.ctx.route, routed.candidates());
    decider_record(run, session, task_ids, chosen, input, now)
}

/// A decider call's outcome: `completed` with `answered`, or `fallback` with its
/// reason.
pub fn decider_outcome(decision: &crate::decider::Decision) -> (RoleOutcome, Option<String>) {
    match decision.source {
        proto::DeciderSource::Decider => (RoleOutcome::Completed, Some("answered".to_string())),
        proto::DeciderSource::Fallback => (RoleOutcome::Fallback, decision.fallback_reason.clone()),
    }
}

#[path = "role_lists.rs"]
pub mod lists;

#[cfg(test)]
#[path = "roles_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "roles_installed_tests.rs"]
mod installed_tests;
