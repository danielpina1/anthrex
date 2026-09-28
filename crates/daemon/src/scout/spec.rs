//! A scout's headless session (milestone 8b decision 12): its route and its read-only
//! launch spec. Pure.

use std::path::PathBuf;

use proto::{AgentRole, Effort, ModelEntry, Route, RunRef, Runtime, ScoutKind, Strength};

use super::contract::{ONBOARDING_CONTRACT, SCOUT_CONTRACT};
use crate::headless::argv::{CliCaps, CodexProjectConfig};
use crate::headless::codex_guard::{CodexConfigGuard, GuardEntry, ObjectFormat};
use crate::headless::{ClaudeSandbox, HeadlessSpec, McpTarget};
use crate::run::role_launch::{
    REVIEWER_CODEX_SANDBOX, REVIEWER_DISALLOWED_TOOLS, REVIEWER_PERMISSION_MODE,
    protected_write_denials,
};
use crate::run::roster::{lowest_at_or_above, peer};

/// The scout's own MCP tool, as Claude names it.
pub const SUBMIT_TOOL: &str = "mcp__anthrex__submit_scout_report";

/// One scout to launch. Serializable: milestone 9's `StartScout` op carries it.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ScoutSpec {
    pub id: String,
    pub kind: ScoutKind,
    /// `None`: a repository-level scout (the onboarding scout).
    pub run_id: Option<String>,
    pub question: String,
    pub first_turn: String,
    pub cwd: PathBuf,
    pub project: PathBuf,
    /// An area scout may also use `WebFetch` and `WebSearch`.
    pub web: bool,
    /// The `.codex` entries of the commit the scout reads (`run::git::codex_config_tree`).
    pub codex_config: Vec<GuardEntry>,
    pub base_sha: String,
    /// Ruling R-T1-1: every repository path the scout can see besides `cwd` and
    /// `project` (a standalone checkout's repository and the object directory it
    /// borrows), denied to its Claude sandbox with them.
    pub repo_paths: Vec<PathBuf>,
}

/// What every scout launch reads from the daemon.
#[derive(Debug, Clone)]
pub struct ScoutContext {
    pub roster: Vec<ModelEntry>,
    pub default_runtime: Runtime,
    pub scouts: config::Scouts,
    pub claude: config::ClaudeHeadless,
    pub caps: CliCaps,
    pub data_dir: PathBuf,
}

/// Decision 12's route: the first roster entry of `runtime` at the lowest strength at or
/// above `strength`, else the same on the peer runtime, else the first entry of
/// `runtime`, else `runtime` with no model (the CLI's default).
pub fn route(roster: &[ModelEntry], runtime: Runtime, strength: Strength, effort: Effort) -> Route {
    let entry = lowest_at_or_above(roster, runtime, strength, None)
        .or_else(|| lowest_at_or_above(roster, peer(runtime), strength, None))
        .or_else(|| roster.iter().find(|e| e.runtime == runtime));
    match entry {
        Some(entry) => Route {
            runtime: entry.runtime,
            model: entry.model.clone(),
            strength: entry.strength,
            effort,
        },
        None => Route {
            runtime,
            model: String::new(),
            strength,
            effort,
        },
    }
}

/// The route `ctx` gives every scout: `scouts.runtime`, else the orchestrator's default
/// runtime, at `scouts.strength` and `scouts.effort`.
pub fn scout_route(ctx: &ScoutContext) -> Route {
    let runtime = ctx.scouts.runtime.unwrap_or(ctx.default_runtime);
    route(&ctx.roster, runtime, ctx.scouts.strength, ctx.scouts.effort)
}

/// Decision 12's read-only session for `scout`.
pub fn headless_spec(scout: &ScoutSpec, ctx: &ScoutContext) -> HeadlessSpec {
    let route = scout_route(ctx);
    let claude = route.runtime == Runtime::Claude;
    let (instructions, mut allowed) = match scout.kind {
        ScoutKind::Area => (SCOUT_CONTRACT, vec![SUBMIT_TOOL, "Read", "Glob", "Grep"]),
        ScoutKind::Onboarding => (
            ONBOARDING_CONTRACT,
            vec![SUBMIT_TOOL, "Bash", "Read", "Glob", "Grep"],
        ),
    };
    if scout.kind == ScoutKind::Area && scout.web {
        allowed.extend(["WebFetch", "WebSearch"]);
    }
    let run_id = scout.run_id.clone().unwrap_or_default();
    let codex_guard = route.runtime == Runtime::Codex
        && ctx.caps.codex_project_config() == CodexProjectConfig::Loaded;
    HeadlessSpec {
        runtime: route.runtime,
        model: route.model.clone(),
        effort: route.effort,
        cwd: scout.cwd.clone(),
        instructions: instructions.to_string(),
        mcp: Some(McpTarget {
            role: AgentRole::Scout,
            run_id: run_id.clone(),
            task_id: None,
            scout_id: Some(scout.id.clone()),
        }),
        allowed_tools: allowed.into_iter().map(String::from).collect(),
        claude_permission_mode: claude.then(|| REVIEWER_PERMISSION_MODE.to_string()),
        claude_disallowed_tools: if claude {
            REVIEWER_DISALLOWED_TOOLS.map(String::from).to_vec()
        } else {
            Vec::new()
        },
        // Always, whatever `worker_sandbox` says: no writable root, and (ruling R-T1-1)
        // the checkout itself and every repository path denied, since Claude's sandbox
        // leaves the working directory writable even with an empty `allowWrite`.
        claude_sandbox: claude.then(|| ClaudeSandbox {
            writable_roots: Vec::new(),
            deny_write: scout_denials(scout),
        }),
        codex_sandbox: REVIEWER_CODEX_SANDBOX.to_string(),
        codex_writable_roots: Vec::new(),
        env: Vec::new(),
        claude_auth: ctx.claude.auth,
        api_key_helper: None,
        run_ref: match scout.kind {
            ScoutKind::Area => Some(RunRef {
                run_id,
                task_id: None,
                role: AgentRole::Scout,
                session: 1,
            }),
            ScoutKind::Onboarding => None,
        },
        codex_config_guard: codex_guard.then(|| CodexConfigGuard {
            format: ObjectFormat::of(&scout.base_sha),
            entries: scout.codex_config.clone(),
        }),
        // Ruling R-T8-1: a scout never gets the output filter.
        output_filter: None,
    }
}

/// The protected agent-config paths, then the checkout, the project and every other
/// repository path the scout can see, each once.
fn scout_denials(scout: &ScoutSpec) -> Vec<PathBuf> {
    let mut denied = protected_write_denials(&scout.cwd, &[]);
    let seen = [&scout.cwd, &scout.project]
        .into_iter()
        .chain(scout.repo_paths.iter());
    for path in seen {
        if !denied.contains(path) {
            denied.push(path.clone());
        }
    }
    denied
}

/// Decision 13: `^[a-z0-9][a-z0-9-]{0,47}$`.
pub fn valid_id(id: &str) -> bool {
    let mut chars = id.chars();
    let first_ok = chars
        .next()
        .is_some_and(|c| c.is_ascii_lowercase() || c.is_ascii_digit());
    first_ok
        && id.len() <= 48
        && chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
}
