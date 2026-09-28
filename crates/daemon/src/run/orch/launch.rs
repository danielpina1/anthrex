//! What the engine launches for a run's planning agents (decision 1): the
//! orchestrator's route (decision 6), its role and its window (decisions 5, 11), and a
//! sub-planner's route (decision 31). Pure.

use proto::{
    AgentRole, ModelEntry, OrchestratorChoice, Route, RoutingCandidate, RunRef, Runtime, Strength,
    WindowSpec,
};

use super::contract::ORCHESTRATOR_CONTRACT;
use crate::headless::McpTarget;
use crate::launch::role::{ORCHESTRATOR_ALLOWED_TOOLS, ORCHESTRATOR_DISALLOWED_TOOLS, RoleLaunch};
use crate::run::model::Run;

/// Decision 6's resolution, and decision 43's candidate snapshot: `source` is
/// `explicit_choice`, `agent_config` or `roster_default`; `candidates` lists the
/// runtime's roster entries in roster order (the chosen route appended when it is not
/// one of them), every one but the chosen with its skip reason.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Resolved {
    pub route: Route,
    pub source: String,
    pub candidates: Vec<RoutingCandidate>,
}

/// Decision 6: the runtime is the choice's, else `[orchestrator.agent] runtime`, else
/// `default_runtime`; the model is the choice's, else the agent's when non-empty (either
/// must be in the roster), else the runtime's first `frontier` entry, else its
/// strongest; the effort is the agent's.
pub fn resolve_orchestrator(
    choice: Option<&OrchestratorChoice>,
    agent: &config::AgentConfig,
    default_runtime: Runtime,
    roster: &[ModelEntry],
) -> Result<Resolved, String> {
    let runtime = choice
        .map(|c| c.runtime)
        .or(agent.runtime)
        .unwrap_or(default_runtime);
    let named = choice
        .and_then(|c| c.model.clone())
        .or_else(|| (!agent.model.is_empty()).then(|| agent.model.clone()));
    let source = if choice.is_some() {
        "explicit_choice"
    } else if agent.runtime.is_some() || !agent.model.is_empty() {
        "agent_config"
    } else {
        "roster_default"
    };
    let entries: Vec<&ModelEntry> = roster.iter().filter(|e| e.runtime == runtime).collect();
    let chosen = match &named {
        Some(model) => *entries
            .iter()
            .find(|e| &e.model == model)
            .ok_or_else(|| format!("{}:{model} is not in the roster", runtime.label()))?,
        None => match entries
            .iter()
            .find(|e| e.strength == Strength::Frontier)
            .or_else(|| entries.iter().rev().max_by_key(|e| e.strength))
        {
            Some(entry) => *entry,
            None => {
                let route = Route {
                    runtime,
                    model: String::new(),
                    strength: Strength::Standard,
                    effort: agent.effort,
                };
                let candidates = vec![RoutingCandidate {
                    route: route.clone(),
                    skipped_reason: None,
                }];
                return Ok(Resolved {
                    route,
                    source: source.into(),
                    candidates,
                });
            }
        },
    };
    let route_of = |e: &ModelEntry| Route {
        runtime: e.runtime,
        model: e.model.clone(),
        strength: e.strength,
        effort: agent.effort,
    };
    let skipped = if named.is_some() {
        "not in the configured list"
    } else {
        "an earlier candidate was taken"
    };
    let candidates = entries
        .iter()
        .map(|e| RoutingCandidate {
            route: route_of(e),
            skipped_reason: (!std::ptr::eq(*e, chosen)).then(|| skipped.to_string()),
        })
        .collect();
    Ok(Resolved {
        route: route_of(chosen),
        source: source.into(),
        candidates,
    })
}

/// The orchestrator's role (decisions 7–11): session `run.orch.orchestrator`'s, its
/// contract, and its tool lists. `env` and `remove_env` are the driver's (decision 10).
pub fn orchestrator_role(run: &Run, route: &Route) -> RoleLaunch {
    let session = run.orch.orchestrator.as_ref().map_or(1, |o| o.session);
    RoleLaunch {
        run_ref: RunRef {
            run_id: run.id.clone(),
            task_id: None,
            role: AgentRole::Orchestrator,
            session,
        },
        mcp: McpTarget {
            role: AgentRole::Orchestrator,
            run_id: run.id.clone(),
            task_id: None,
            scout_id: None,
        },
        instructions: ORCHESTRATOR_CONTRACT.to_string(),
        effort: route.effort,
        claude_allowed_tools: strings(ORCHESTRATOR_ALLOWED_TOOLS),
        claude_disallowed_tools: strings(ORCHESTRATOR_DISALLOWED_TOOLS),
        env: Vec::new(),
        remove_env: Vec::new(),
    }
}

/// Decision 5: `<h4>/orchestrator` in the user's own checkout, on the route's runtime
/// and model (none for the CLI's default), with its first prompt.
pub fn orchestrator_window_spec(run: &Run, route: &Route, first_prompt: &str) -> WindowSpec {
    WindowSpec {
        name: Some(format!("{}/orchestrator", run.short())),
        runtime: route.runtime,
        cwd: run.root.clone(),
        worktree_branch: None,
        model: (!route.model.is_empty()).then(|| route.model.clone()),
        initial_prompt: Some(first_prompt.to_string()),
    }
}

/// Decision 31: a sub-planner's route, `[orchestrator.planners]` as the run was built
/// with it, on its runtime or else the orchestrator's (M8b's `scout::spec::route`).
pub fn planner_route(run: &Run) -> Option<Route> {
    let orchestrator = run.orch.orchestrator.as_ref()?;
    let p = &run.limits.orch.planners;
    let runtime = p.runtime.unwrap_or(orchestrator.route.runtime);
    Some(crate::scout::spec::route(
        &run.roster,
        runtime,
        p.strength,
        p.effort,
    ))
}

fn strings(list: &[&str]) -> Vec<String> {
    list.iter().map(|s| s.to_string()).collect()
}
