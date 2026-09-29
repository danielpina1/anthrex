//! Milestone 9 decision 43: the role-routing records of the sessions that belong to no
//! task — the orchestrator, a sub-planner, a run scout and a decider (pre-run triage
//! included). Each is made when its session is dispatched, with the full ordered
//! candidate snapshot it was chosen from, and finished once with the session's factual
//! outcome. Pure: the engine keeps a run's records in `Run.role_routing_decisions` and
//! appends each finished one to `history.jsonl` as `OpKind::AppendHistory`; the driver
//! appends pre-run triage's itself (`driver/adapt_goal.rs`).
//!
//! **Record ids.** `<run id>/<role>/<session id>` for a run's session, and
//! `triage/<request>/<n>` for pre-run triage, which has no run. A task's record is
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

use proto::{
    AgentRole, Effort, HISTORY_VERSION, ModelEntry, RoleOutcome, RoleRoutingDecision,
    RoleRoutingInput, Route, RoutingCandidate, Runtime, Strength,
};
use serde::{Deserialize, Serialize};

use crate::run::model::Run;
use crate::run::roster::peer;

pub const NOT_INSTALLED: &str = "not installed";
pub const NOT_CONFIGURED: &str = "not in the configured list";
pub const EARLIER_TAKEN: &str = "an earlier candidate was taken";

pub const ORCHESTRATOR_POLICY: &str = "m9-orchestrator-v1";
pub const PLANNER_POLICY: &str = "m9-planner-v1";
pub const SCOUT_POLICY: &str = "m9-scout-v1";
pub const DECIDER_POLICY: &str = "m9-decider-v1";

/// A run record's goal cap (`history::GOAL_CHARS`), which the input's goal shares.
const GOAL_CHARS: usize = 200;

/// Decision 6's resolution as the orchestrator's record keeps it
/// (`OrchestratorRecord.routing`): every launch and restart of the run's orchestrator
/// is recorded with the snapshot taken when its route was resolved, whatever the
/// configuration says later.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RoleSnapshot {
    /// `explicit_choice`, `agent_config` or `roster_default`; empty on a record made
    /// before milestone 9's task M9.13b.
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
        None => format!("triage/{session_id}"),
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

/// M8b decision 12's strength ladder (`scout::spec::route`, and the deciders' route) as
/// an ordered candidate list: `runtime`'s roster entries at or above `strength`, lowest
/// strength first and roster order among ties, then its peer's the same way.
pub fn ladder_candidates(
    roster: &[ModelEntry],
    runtime: Runtime,
    strength: Strength,
    effort: Effort,
) -> Vec<RoutingCandidate> {
    let mut out = runtime_ladder(roster, runtime, strength, effort);
    out.extend(runtime_ladder(roster, peer(runtime), strength, effort));
    out
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

/// `runtime`'s roster entries at or above `strength`, lowest strength first and roster
/// order among ties, at `effort`.
fn runtime_ladder(
    roster: &[ModelEntry],
    runtime: Runtime,
    strength: Strength,
    effort: Effort,
) -> Vec<RoutingCandidate> {
    let mut entries: Vec<&ModelEntry> = roster
        .iter()
        .filter(|e| e.runtime == runtime && e.strength >= strength)
        .collect();
    entries.sort_by_key(|e| e.strength);
    entries
        .into_iter()
        .map(|e| RoutingCandidate {
            route: Route {
                runtime: e.runtime,
                model: e.model.clone(),
                strength: e.strength,
                effort,
            },
            skipped_reason: None,
        })
        .collect()
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
        ORCHESTRATOR_POLICY,
        input_of(run),
        o.routing.candidates.clone(),
        &o.route,
        now,
    ))
}

/// The record of session `session` of epic `k`'s sub-planner: `[orchestrator.planners]`
/// as the run was frozen with it, on its runtime or else the orchestrator's.
pub fn planner_record(run: &Run, k: usize, session: u32, now: u64) -> RoleRoutingDecision {
    let epic = &run.orch.epics[k];
    let p = &run.limits.orch.planners;
    let orchestrator = run.orch.orchestrator.as_ref().map(|o| o.route.runtime);
    let runtime = p.runtime.or(orchestrator).unwrap_or(epic.route.runtime);
    let ladder = ladder_candidates(&run.roster, runtime, p.strength, p.effort);
    let candidates = mark_not_installed(ladder, &run.orch.installed);
    let mut input = input_of(run);
    input.epic = Some(epic.epic.clone());
    input.area = epic.area.clone();
    let trigger = if session == 1 { "start" } else { "replan" };
    record(
        Some(run),
        AgentRole::Planner,
        &format!("{}/{session}", epic.epic),
        trigger,
        "planner_config",
        PLANNER_POLICY,
        input,
        candidates,
        &epic.route,
        now,
    )
}

/// The record of run scout `scout_id`'s session, on the route the driver starts it on
/// (`launch::scout_route_of`: `[orchestrator.scouts]` as the run froze it, over the
/// run's installed runtimes).
pub fn scout_record(
    run: &Run,
    scout_id: &str,
    ctx: &crate::scout::spec::ScoutContext,
    now: u64,
) -> RoleRoutingDecision {
    let chosen = crate::run::orch::launch::scout_route_of(run, ctx);
    let (roster, routing) = match &run.limits.orch.scouts {
        Some(routing) => (&run.roster, routing.clone()),
        None => (&ctx.roster, crate::scout::spec::ScoutRouting::of(ctx)),
    };
    let runtime = routing.runtime.unwrap_or(routing.default_runtime);
    let ladder = ladder_candidates(roster, runtime, routing.strength, chosen.effort);
    let candidates = mark_not_installed(ladder, &run.orch.installed);
    let mut input = input_of(run);
    if let Some(scout) = run.orch.run_scouts.iter().find(|s| s.id == scout_id) {
        input.area = scout.area.clone();
    }
    record(
        Some(run),
        AgentRole::Scout,
        scout_id,
        "start",
        "scout_config",
        SCOUT_POLICY,
        input,
        candidates,
        &chosen,
        now,
    )
}

/// A decider's candidates (review I-2): `[orchestrator.deciders]`'s ladder on the
/// mode's runtime only (the route's), its roster entries at or above `strength`, lowest
/// first. `DeciderContext::new` takes the first of them, else the runtime's first entry,
/// which [`record`] then appends.
pub fn decider_candidates(
    roster: &[ModelEntry],
    route: &Route,
    strength: Strength,
) -> Vec<RoutingCandidate> {
    runtime_ladder(roster, route.runtime, strength, route.effort)
}

/// A decider's record (`run`: a run-bound decider; `None`: pre-run triage, whose
/// `session_id` is `<request>/<n>`). Its trigger and question kind are the request's
/// kind; `candidates` are [`decider_candidates`]. `task_id` is the task when the call
/// is about exactly one; a call about several (a size check of a batch) is run-level
/// and names none (review M-4).
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
        "decider_config",
        DECIDER_POLICY,
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

/// A decider call's outcome: `completed` with `answered`, or `fallback` with its
/// reason.
pub fn decider_outcome(decision: &crate::decider::Decision) -> (RoleOutcome, Option<String>) {
    match decision.source {
        proto::DeciderSource::Decider => (RoleOutcome::Completed, Some("answered".to_string())),
        proto::DeciderSource::Fallback => (RoleOutcome::Fallback, decision.fallback_reason.clone()),
    }
}

#[cfg(test)]
#[path = "roles_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "roles_installed_tests.rs"]
mod installed_tests;
