//! A scout's headless session (milestone 8b decision 12): its route and its read-only
//! launch spec. Pure.

use std::path::PathBuf;
use std::sync::Arc;

use proto::{AgentRole, Effort, ModelEntry, Route, RunRef, Runtime, ScoutKind, Strength};

use super::contract::{ONBOARDING_CONTRACT, SCOUT_CONTRACT};
use crate::headless::argv::{CliCaps, CodexProjectConfig};
use crate::headless::codex_guard::{CodexConfigGuard, GuardEntry, ObjectFormat};
use crate::headless::{ClaudeSandbox, HeadlessSpec, McpTarget};
use crate::live_config::LiveSettings;
use crate::run::role_launch::{
    REVIEWER_CODEX_SANDBOX, REVIEWER_DISALLOWED_TOOLS, REVIEWER_PERMISSION_MODE,
    protected_write_denials,
};
use crate::run::roster::{lowest_at_or_above, peer};

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

/// Where a scout's roster comes from (milestone 9.0.6 decision 29): the daemon's live
/// settings, read at each spawn, or a fixed list (tests).
#[derive(Debug, Clone)]
pub enum Roster {
    Fixed(Vec<ModelEntry>),
    Live(Arc<LiveSettings>),
}

impl Roster {
    /// The roster now.
    pub fn current(&self) -> Vec<ModelEntry> {
        match self {
            Roster::Fixed(models) => models.clone(),
            Roster::Live(live) => live.current().orchestrator.models.clone(),
        }
    }
}

impl From<Vec<ModelEntry>> for Roster {
    fn from(models: Vec<ModelEntry>) -> Self {
        Roster::Fixed(models)
    }
}

/// Decision 12's route: the first roster entry of `runtime` at the lowest strength at or
/// above `strength`, else the same on the peer runtime, else the first entry of
/// `runtime`, else `runtime` with no model (the CLI's default).
pub fn route(roster: &[ModelEntry], runtime: Runtime, strength: Strength, effort: Effort) -> Route {
    route_within(roster, runtime, strength, effort, true)
}

/// [`route`], stepping to the peer runtime only when `peer_allowed` (M9.17 fix round 2:
/// a run's sub-planners never step to a runtime its start check found not installed).
pub fn route_within(
    roster: &[ModelEntry],
    runtime: Runtime,
    strength: Strength,
    effort: Effort,
    peer_allowed: bool,
) -> Route {
    let entry = lowest_at_or_above(roster, runtime, strength, None)
        .or_else(|| {
            peer_allowed
                .then(|| lowest_at_or_above(roster, peer(runtime), strength, None))
                .flatten()
        })
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
    route(
        &ctx.roster.current(),
        runtime,
        ctx.scouts.strength,
        ctx.scouts.effort,
    )
}

/// `[orchestrator.scouts]`'s route keys and `orchestrator.default_runtime`, as a run
/// froze them at its start (whole-branch review, item 1): its scouts' runtime is one its
/// start checked (decisions 50 and 53), whatever the daemon's config says later.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ScoutRouting {
    pub runtime: Option<Runtime>,
    pub default_runtime: Runtime,
    pub strength: Strength,
    pub effort: Effort,
}

impl ScoutRouting {
    /// The keys `config` gives a run built now.
    pub fn from_config(config: &config::Orchestrator) -> ScoutRouting {
        ScoutRouting {
            runtime: config.scouts.runtime,
            default_runtime: config.default_runtime,
            strength: config.scouts.strength,
            effort: config.scouts.effort,
        }
    }

    /// The scout service's live keys (a run recorded before they were frozen).
    pub fn of(ctx: &ScoutContext) -> ScoutRouting {
        ScoutRouting {
            runtime: ctx.scouts.runtime,
            default_runtime: ctx.default_runtime,
            strength: ctx.scouts.strength,
            effort: ctx.scouts.effort,
        }
    }
}

/// A run scout's route (M9.17 fix round 3): [`scout_route`]'s rule on `routing` and
/// `roster`, over the runtimes the run's start found installed (`run.orch.installed`,
/// keyed by runtime label). When nothing pins `[orchestrator.scouts] runtime` and the
/// default runtime is not installed, the installed peer is used; the strength step
/// reaches the peer only when it is installed. A runtime with nothing recorded counts
/// as installed. Onboarding scouts keep [`scout_route`].
pub fn run_scout_route(
    roster: &[ModelEntry],
    routing: &ScoutRouting,
    installed: &std::collections::BTreeMap<String, bool>,
) -> Route {
    let missing = |runtime: Runtime| installed.get(runtime.label()) == Some(&false);
    let mut runtime = routing.runtime.unwrap_or(routing.default_runtime);
    if routing.runtime.is_none() && missing(runtime) && !missing(peer(runtime)) {
        runtime = peer(runtime);
    }
    let peer_allowed = !missing(peer(runtime));
    route_within(
        roster,
        runtime,
        routing.strength,
        routing.effort,
        peer_allowed,
    )
}

/// Decision 12's read-only session for `scout`, on [`scout_route`].
pub fn headless_spec(scout: &ScoutSpec, ctx: &ScoutContext) -> HeadlessSpec {
    headless_spec_on(scout, ctx, &scout_route(ctx))
}

/// Decision 12's read-only session for `scout` on `route` (a run scout's is
/// [`run_scout_route`]).
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
        effort: route.effort,
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
