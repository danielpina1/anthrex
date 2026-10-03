//! What the engine launches for a run's planning agents (decision 1): the
//! orchestrator's route (decision 6), its role and its window (decisions 5, 11), a
//! sub-planner's route and session (decision 31), and a run scout's (decision 20). Pure.

use proto::{
    AgentRole, ModelEntry, OrchestratorChoice, Route, RoutingCandidate, RunRef, Runtime, Strength,
    WindowSpec,
};

use std::path::Path;

use proto::ScoutKind;

use super::contract::{
    ORCHESTRATOR_CONTRACT, PLANNER_CONTRACT, planner_extract_at, planner_prompt, replan_prompt,
    scout_first_turn,
};
use super::extract::ExtractSlot;
use super::{EpicRecord, RunScout};
use crate::headless::{ClaudeSandbox, HeadlessSpec, McpTarget};
use crate::launch::role::{ORCHESTRATOR_ALLOWED_TOOLS, ORCHESTRATOR_DISALLOWED_TOOLS, RoleLaunch};
use crate::run::model::{Run, Task};
use crate::run::role_launch::{
    REVIEWER_CODEX_SANDBOX, REVIEWER_DISALLOWED_TOOLS, REVIEWER_PERMISSION_MODE,
    codex_config_guard, protected_write_denials,
};
use crate::scout::contract::SCOUT_CONTRACT;
use crate::scout::planner::PlannerSpec;
use crate::scout::spec::ScoutSpec;

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
            lane: None,
        },
        mcp: McpTarget {
            role: AgentRole::Orchestrator,
            run_id: run.id.clone(),
            task_id: None,
            scout_id: None,
            epic: None,
            chain: run.chain.clone(),
            lane: None,
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

/// The run scouts' route keys: the ones the run froze at its start (whole-branch
/// review, item 1), else, for a run recorded on this branch before they were frozen,
/// the default scout keys on the run's own frozen `default_runtime` (fix round 2, item
/// 4). Never the daemon's live config: what the start checked is what launches.
pub fn scout_routing(run: &Run) -> crate::scout::spec::ScoutRouting {
    run.limits.orch.scouts.clone().unwrap_or_else(|| {
        let defaults = config::Scouts::default();
        crate::scout::spec::ScoutRouting {
            runtime: None,
            default_runtime: run.limits.default_runtime,
            strength: defaults.strength,
            effort: defaults.effort,
        }
    })
}

/// A run scout's route: [`crate::scout::spec::run_scout_route`] on [`scout_routing`]
/// and the run's roster, over its installed runtimes.
pub fn scout_route_of(run: &Run) -> Route {
    crate::scout::spec::run_scout_route(&run.roster, &scout_routing(run), &run.orch.installed)
}

/// The run scouts' route as `reach::reachable_runtimes` counts it: [`scout_route_of`].
pub fn frozen_scout_route(run: &Run) -> Route {
    scout_route_of(run)
}

/// Decision 31: a sub-planner's route, `[orchestrator.planners]` as the run was built
/// with it, on its runtime or else the orchestrator's (M8b's `scout::spec::route`). The
/// route steps to the peer runtime only when the run's start check did not find the peer
/// missing (`run.orch.installed`; M9.17 fix round 2), so a Codex-only user's frontier
/// planner stays on Codex instead of naming an uninstalled Claude model.
pub fn planner_route(run: &Run) -> Option<Route> {
    let orchestrator = run.orch.orchestrator.as_ref()?;
    let p = &run.limits.orch.planners;
    let runtime = p.runtime.unwrap_or(orchestrator.route.runtime);
    let peer = crate::run::roster::peer(runtime);
    let peer_allowed = run
        .orch
        .installed
        .get(peer.label())
        .copied()
        .unwrap_or(true);
    Some(crate::scout::spec::route_within(
        &run.roster,
        runtime,
        p.strength,
        p.effort,
        peer_allowed,
    ))
}

/// The Claude tools a sub-planner may use (decision 31): its two anthrex tools and the
/// read-only file tools.
pub const PLANNER_ALLOWED_TOOLS: &[&str] = &[
    "mcp__anthrex__get_context",
    "mcp__anthrex__submit_epic",
    "Read",
    "Glob",
    "Grep",
];

/// Decision 31: session `session` of `epic`'s sub-planner, launched read-only as M8b's
/// area scouts are (`scout::spec::headless_spec`): the planner contract, its tools, an
/// explicit `--permission-mode` with the reviewers' denials, an empty-root Claude
/// sandbox denying the checkout and every repository path, Codex `read-only` with the
/// run's config guard, no output filter. It reads the user's checkout (decision 20a).
/// Its first turn is the planner prompt, or the re-plan prompt once the epic was
/// re-planned, with the slot the driver puts the epic's scout extract in (decision 34).
pub fn planner_spec(run: &Run, epic: &EpicRecord, session: u32) -> PlannerSpec {
    let route = epic.route.clone();
    let replan = !epic.replans.is_empty();
    let first_turn = match replan {
        false => planner_prompt(run, epic, ""),
        true => replan_prompt(run, epic, ""),
    };
    let at = planner_extract_at(run, epic, replan);
    let mcp = McpTarget {
        role: AgentRole::Planner,
        run_id: run.id.clone(),
        task_id: None,
        scout_id: None,
        epic: Some(epic.epic.clone()),
        chain: None,
        lane: None,
    };
    let run_ref = RunRef {
        run_id: run.id.clone(),
        task_id: None,
        role: AgentRole::Planner,
        session,
        lane: None,
    };
    let headless = read_only(
        run,
        &route,
        (PLANNER_CONTRACT, PLANNER_ALLOWED_TOOLS),
        mcp,
        run_ref,
    );
    let limits = &run.limits.orch.planners;
    PlannerSpec {
        run_id: run.id.clone(),
        epic: epic.epic.clone(),
        session,
        headless,
        first_turn,
        project: run.project.clone(),
        cwd: run.root.clone(),
        route,
        max_tool_calls: limits.max_tool_calls,
        timeout_secs: limits.timeout_secs,
        extract: ExtractSlot::new(&epic.scout_refs, run.onboarding_report.as_deref(), at, "\n"),
    }
}

/// Decision 20: run scout `scout` as M8b decision 12's area scout, reading the user's
/// checkout `root` (decision 20a), with the run's Codex guard entries, base and
/// repository paths.
pub fn scout_spec(run: &Run, scout: &RunScout, root: &Path, project: &Path) -> ScoutSpec {
    ScoutSpec {
        id: scout.id.clone(),
        kind: ScoutKind::Area,
        run_id: Some(run.id.clone()),
        question: scout.question.clone(),
        first_turn: scout_first_turn(run, &scout.id, &scout.area, &scout.question),
        cwd: root.to_path_buf(),
        project: project.to_path_buf(),
        web: scout.web,
        codex_config: run.codex_config_base.clone(),
        base_sha: run.base_sha.clone(),
        repo_paths: vec![run.root.clone(), run.git_common_dir.clone()],
    }
}

/// The Claude tools a research session may use (decision 35): M8b's area scout's, and
/// the web.
pub const RESEARCH_ALLOWED_TOOLS: &[&str] = &[
    crate::scout::spec::SUBMIT_TOOL,
    "Read",
    "Glob",
    "Grep",
    "WebFetch",
    "WebSearch",
];

/// Decision 35: research task `task`'s session `task.session`, launched as M8b's area
/// scout is (decision 31's read-only launch): the scout contract, its tools with the
/// web ones, the task's resolved route, the user's checkout (decision 20a), and MCP
/// bound to the task, so its `submit_scout_report` reaches the engine.
pub fn research_spec(run: &Run, task: &Task) -> HeadlessSpec {
    let run_ref = RunRef {
        run_id: run.id.clone(),
        task_id: Some(task.id().to_string()),
        role: AgentRole::Scout,
        session: task.session,
        lane: None,
    };
    let mcp = McpTarget {
        role: AgentRole::Scout,
        run_id: run.id.clone(),
        task_id: Some(task.id().to_string()),
        scout_id: None,
        epic: None,
        chain: None,
        lane: None,
    };
    read_only(
        run,
        &task.route,
        (SCOUT_CONTRACT, RESEARCH_ALLOWED_TOOLS),
        mcp,
        run_ref,
    )
}

/// Decisions 36 and 37: a review task's reviewer, launched as M8a's reviewer of a
/// task (`role_launch::reviewer_spec`): read-only in the task's review worktree.
pub fn review_task_spec(run: &Run, task: &Task, route: &Route) -> HeadlessSpec {
    crate::run::role_launch::reviewer_spec(run, task, route)
}

/// A read-only session in the user's checkout (decisions 20a, 31): an explicit
/// `--permission-mode` with the reviewers' denials, an empty-root Claude sandbox
/// denying the checkout and every repository path, Codex `read-only` with the run's
/// config guard, no output filter.
fn read_only(
    run: &Run,
    route: &Route,
    (instructions, tools): (&str, &[&str]),
    mcp: McpTarget,
    run_ref: RunRef,
) -> HeadlessSpec {
    let claude = route.runtime == Runtime::Claude;
    let mut deny = protected_write_denials(&run.root, &[]);
    for path in [&run.root, &run.project, &run.git_common_dir] {
        if !deny.contains(path) {
            deny.push(path.clone());
        }
    }
    HeadlessSpec {
        runtime: route.runtime,
        model: route.model.clone(),
        effort: route.effort,
        cwd: run.root.clone(),
        instructions: instructions.to_string(),
        mcp: Some(mcp),
        allowed_tools: strings(tools),
        claude_permission_mode: claude.then(|| REVIEWER_PERMISSION_MODE.to_string()),
        claude_disallowed_tools: match claude {
            true => strings(&REVIEWER_DISALLOWED_TOOLS),
            false => Vec::new(),
        },
        claude_sandbox: claude.then(|| ClaudeSandbox {
            writable_roots: Vec::new(),
            deny_write: deny,
        }),
        codex_sandbox: REVIEWER_CODEX_SANDBOX.to_string(),
        codex_writable_roots: Vec::new(),
        env: Vec::new(),
        claude_auth: run.limits.claude_auth.into(),
        api_key_helper: run.limits.api_key_helper.clone(),
        run_ref: Some(run_ref),
        codex_config_guard: codex_config_guard(run, route.runtime),
        output_filter: None,
    }
}

fn strings(list: &[&str]) -> Vec<String> {
    list.iter().map(|s| s.to_string()).collect()
}

#[cfg(test)]
#[path = "launch_tests.rs"]
mod tests;
