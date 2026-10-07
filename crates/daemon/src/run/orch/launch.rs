//! What the engine launches for a run's planning agents (decision 1): the
//! orchestrator's route (decision 6), its role and its window (decisions 5, 11), a
//! sub-planner's route and session (decision 31), and a run scout's (decision 20). Pure.

use proto::models::{ModelRef, Role, RoleChoice};
use proto::{AgentRole, OrchestratorChoice, Route, RoutingCandidate, RunRef, Runtime, WindowSpec};

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
use crate::run::model_roles::RunModels;
use crate::run::role_launch::{
    REVIEWER_CODEX_SANDBOX, REVIEWER_DISALLOWED_TOOLS, REVIEWER_PERMISSION_MODE,
    codex_config_guard, protected_write_denials,
};
use crate::run::routing::ROLE_TABLE;
use crate::scout::contract::SCOUT_CONTRACT;
use crate::scout::planner::PlannerSpec;
use crate::scout::spec::ScoutSpec;

/// The orchestrator's route and decision 43's candidate snapshot: `source` is
/// `explicit_choice` (the goal form, `run promote --orchestrator`, a continued chain) or
/// `role_table` (milestone 9.8); `candidates` is the chosen route alone.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Resolved {
    pub route: Route,
    pub source: String,
    pub candidates: Vec<RoutingCandidate>,
}

/// Milestone 9.8 (MR §3.1): the `orchestrator` row of the run's table, unless there is
/// a `choice` (the goal form's, `--orchestrator`, promote's). A choice naming a model
/// runs it, at the choice's effort, else the row's when it is the row's own model, else
/// the model's default. A choice naming only a runtime (fix round 1, I2, controller
/// ruling) runs [`runtime_default`]: never the CLI's bare default for a runtime the
/// table has an orchestrator on.
pub fn orchestrator_route(choice: Option<&OrchestratorChoice>, models: &RunModels) -> Resolved {
    let row = models.choice(Role::Orchestrator);
    let (route, source) = match choice {
        None => (models.route(Role::Orchestrator), ROLE_TABLE),
        Some(c) => {
            let named = c.model.clone().filter(|m| !m.is_empty());
            let (model, effort) = match named {
                Some(id) => {
                    let model = ModelRef {
                        runtime: c.runtime,
                        id: Some(id),
                    };
                    let own = (model == row.model).then_some(row.effort.clone()).flatten();
                    (model, own)
                }
                None => runtime_default(c.runtime, row),
            };
            let effort = c.effort.clone().or(effort);
            let route = RunModels::route_of(&model, effort.as_deref());
            (route, super::roles::lists::EXPLICIT_SOURCE)
        }
    };
    let candidates = vec![RoutingCandidate {
        route: route.clone(),
        skipped_reason: None,
    }];
    Resolved {
        route,
        source: source.to_string(),
        candidates,
    }
}

/// Fix round 1 (I2, controller ruling): the orchestrator a choice naming only `runtime`
/// runs, and its effort: the `orchestrator` row when the row is on `runtime`, else the
/// built-in table's orchestrator on `runtime` (Opus at high for Claude), else the
/// built-in brainstorm row's model on `runtime` at its effort (the built-in table's
/// only Codex model is `codex:default`), else the runtime's default.
fn runtime_default(runtime: Runtime, row: &RoleChoice) -> (ModelRef, Option<String>) {
    let builtin = config::models::builtin_choice(Role::Orchestrator);
    let pair = config::models::builtin_brainstorm();
    let brainstorm = [&pair.first, &pair.second]
        .into_iter()
        .find(|m| m.runtime == runtime)
        .map(|m| RoleChoice {
            model: m.clone(),
            effort: pair.effort.clone(),
            fallback: None,
        });
    [Some(row.clone()), Some(builtin), brainstorm]
        .into_iter()
        .flatten()
        .find(|r| r.model.runtime == runtime)
        .map_or((ModelRef::default_of(runtime), None), |r| {
            (r.model, r.effort)
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
            agent_label: None,
        },
        instructions: ORCHESTRATOR_CONTRACT.to_string(),
        effort: route.effort.clone(),
        claude_allowed_tools: strings(ORCHESTRATOR_ALLOWED_TOOLS),
        claude_disallowed_tools: strings(ORCHESTRATOR_DISALLOWED_TOOLS),
        env: Vec::new(),
        remove_env: Vec::new(),
    }
}

/// Decision 5: `<h4>/orchestrator` in the user's own checkout, on the route's runtime
/// and model (none for the CLI's default), with its first prompt. Milestone 9.5
/// decision 38 and ruling T5a-2: a Claude orchestrator starts with no prompt, and its
/// first prompt is pasted once the window is ready (`engine/wake.rs`).
pub fn orchestrator_window_spec(run: &Run, route: &Route, first_prompt: &str) -> WindowSpec {
    WindowSpec {
        name: Some(format!("{}/orchestrator", run.short())),
        runtime: route.runtime,
        cwd: run.root.clone(),
        worktree_branch: None,
        model: (!route.model.is_empty()).then(|| route.model.clone()),
        initial_prompt: (!first_turn_pasted(route)).then(|| first_prompt.to_string()),
    }
}

/// Milestone 9.5 ruling T5a-2: whether the orchestrator's first prompt waits to be
/// pasted (decision 38). Only Claude's does: Claude runs no hook before its workspace
/// trust prompt, so a prompt on its command line could start work before the user
/// trusts the folder, and FU-F12 was Claude's. Codex keeps 9.3's command-line prompt.
pub fn first_turn_pasted(route: &Route) -> bool {
    route.runtime == Runtime::Claude
}

/// Milestone 9.8 (MR §3.1): the run scouts' route, the run's `research` row.
pub fn scout_route(run: &Run) -> Route {
    run.limits.models().route(Role::Research)
}

/// Decision 31, milestone 9.8 (MR §3.1): a sub-planner's route, the run's `planner`
/// row, once the run has an orchestrator.
pub fn planner_route(run: &Run) -> Option<Route> {
    run.orch.orchestrator.as_ref()?;
    Some(run.limits.models().route(Role::Planner))
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
    let mut first_turn = match replan {
        false => planner_prompt(run, epic, ""),
        true => replan_prompt(run, epic, ""),
    };
    // Milestone 9.6 decision 22: the approved spec's part for this epic, after the
    // extract's slot.
    first_turn.push_str(&crate::run::design::epic::block(run, epic));
    let at = planner_extract_at(run, epic, replan);
    let mcp = McpTarget {
        role: AgentRole::Planner,
        run_id: run.id.clone(),
        task_id: None,
        scout_id: None,
        epic: Some(epic.epic.clone()),
        chain: None,
        lane: None,
        agent_label: None,
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
        agent_label: None,
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
/// config guard, no output filter. Milestone 9.6: the design agents' too
/// (`scout::design_spec`).
pub(crate) fn read_only(
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
        effort: route.effort.clone(),
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
            deny_read: Vec::new(),
            writable_roots: Vec::new(),
            deny_write: deny,
        }),
        codex_sandbox: REVIEWER_CODEX_SANDBOX.to_string(),
        codex_writable_roots: Vec::new(),
        codex_read_only: Vec::new(),
        codex_grant_dialect: None,
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
