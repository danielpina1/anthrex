//! A scout's headless session (milestone 8b decision 12): its route and its read-only
//! launch spec. Pure.

use std::path::PathBuf;
use std::sync::Arc;

use proto::models::{ModelTable, Role};
use proto::{AgentRole, Route, RunRef, Runtime, ScoutKind};

use super::contract::{ONBOARDING_CONTRACT, SCOUT_CONTRACT};
use crate::headless::argv::{CliCaps, CodexProjectConfig};
use crate::headless::codex_guard::{CodexConfigGuard, GuardEntry, ObjectFormat};
use crate::headless::{ClaudeSandbox, HeadlessSpec, McpTarget};
use crate::live_config::LiveSettings;
use crate::run::model_roles::RunModels;
use crate::run::role_launch::{
    REVIEWER_CODEX_SANDBOX, REVIEWER_DISALLOWED_TOOLS, REVIEWER_PERMISSION_MODE,
    protected_write_denials,
};

/// The scout's own MCP tool, as Claude names it.
pub const SUBMIT_TOOL: &str = "mcp__anthrex__submit_scout_report";

/// An area scout's read tools, after its submission tool; milestone 9.6's design agents
/// end their allowlists with them (`design_spec.rs`).
pub const AREA_SCOUT_READ_TOOLS: [&str; 3] = ["Read", "Glob", "Grep"];

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
    pub roster: Roster,
    pub default_runtime: Runtime,
    pub scouts: config::Scouts,
    pub claude: config::ClaudeHeadless,
    pub caps: CliCaps,
    pub data_dir: PathBuf,
}

/// Where a scout's role table comes from (milestone 9.0.6 decision 29, a roster until
/// milestone 9.8): the daemon's live settings, read at each spawn, or a fixed table
/// (tests).
#[derive(Debug, Clone)]
pub enum Roster {
    Fixed(ModelTable),
    Live(Arc<LiveSettings>),
}

impl Roster {
    /// Milestone 9.8: the global role table now.
    pub fn roles(&self) -> ModelTable {
        match self {
            Roster::Fixed(roles) => roles.clone(),
            Roster::Live(live) => live.current().orchestrator.roles.clone(),
        }
    }
}

impl From<ModelTable> for Roster {
    fn from(roles: ModelTable) -> Self {
        Roster::Fixed(roles)
    }
}

/// Milestone 9.8 (MR §3.1): the route `ctx` gives a scout with no run and no
/// repository file: the live table's `research` row. The onboarding scout's adds its
/// repository's file and what is installed (`profile/service_start.rs`).
pub fn scout_route(ctx: &ScoutContext) -> Route {
    let choice = config::models::resolve(Role::Research, None, &ctx.roster.roles());
    RunModels::route_of(&choice.model, choice.effort.as_deref())
}

/// Decision 12's read-only session for `scout`, on [`scout_route`].
pub fn headless_spec(scout: &ScoutSpec, ctx: &ScoutContext) -> HeadlessSpec {
    headless_spec_on(scout, ctx, &scout_route(ctx))
}

/// Decision 12's read-only session for `scout` on `route` (a run scout's is its run's
/// `research` row, `run::orch::launch::scout_route`).
pub fn headless_spec_on(scout: &ScoutSpec, ctx: &ScoutContext, route: &Route) -> HeadlessSpec {
    let claude = route.runtime == Runtime::Claude;
    let (instructions, mut allowed) = match scout.kind {
        ScoutKind::Area => (
            SCOUT_CONTRACT,
            [&[SUBMIT_TOOL][..], &AREA_SCOUT_READ_TOOLS].concat(),
        ),
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
        effort: route.effort.clone(),
        cwd: scout.cwd.clone(),
        instructions: instructions.to_string(),
        mcp: Some(McpTarget {
            role: AgentRole::Scout,
            run_id: run_id.clone(),
            task_id: None,
            scout_id: Some(scout.id.clone()),
            epic: None,
            chain: None,
            lane: None,
            agent_label: None,
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
            deny_read: Vec::new(),
            writable_roots: Vec::new(),
            deny_write: scout_denials(scout),
        }),
        codex_sandbox: REVIEWER_CODEX_SANDBOX.to_string(),
        codex_writable_roots: Vec::new(),
        codex_read_only: Vec::new(),
        codex_grant_dialect: None,
        env: Vec::new(),
        claude_auth: ctx.claude.auth,
        api_key_helper: None,
        run_ref: match scout.kind {
            ScoutKind::Area => Some(RunRef {
                run_id,
                task_id: None,
                role: AgentRole::Scout,
                session: 1,
                lane: None,
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
