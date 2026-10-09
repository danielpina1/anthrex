//! `run start`'s build and checks, moved out of `requests.rs` (task M9.13's
//! preparatory move; no behaviour change): [`RunService::build_plan`] with decision 15's
//! id draw, decisions 50 and 53's runtime checks, and ruling T22-I1b's
//! `runtime_refusals`. Every git read runs on `spawn_blocking`; the engine lock is taken
//! only to read a run's fields.

use std::collections::BTreeMap;
use std::collections::hash_map::RandomState;
use std::ffi::{OsStr, OsString};
use std::hash::{BuildHasher, Hasher};
use std::path::{Path, PathBuf};
use std::time::Duration;

use proto::{Plan, Runtime};

use super::adapt::BuildError;
use super::build_name::RunName;
use super::delivery::DeliveryStart;
use super::ops::blocking;
use super::{RunService, unix_now};
use crate::run::chain::suffix_taken;
use crate::run::confine;
use crate::run::design::GoalOrigin;
use crate::run::git::{self, Git, os};
use crate::run::globs::ProtectedMatcher;
use crate::run::journal::runs_dir;
use crate::run::model::{ClaudeAuth, Run};
use crate::run::orch::installed::{Missing, not_installed, resolve_planned, resolve_promoted};
use crate::run::orch::make_planned;
use crate::run::plan::{BuildContext, random_suffix, resolve_profile, run_id_taken, slug};
use crate::run::reach::{edits_may_widen, reachable_runtimes};
use crate::worktree::repo_worktrees_dir;

/// Decision 15: an id is redrawn at most this many times.
const ID_DRAWS: usize = 5;

/// Decision 50's refusal.
const API_KEY_NEEDED: &str = "[orchestrator.claude] auth = \"api_key\" needs ANTHROPIC_API_KEY in the daemon's environment or orchestrator.claude.api_key_helper";

/// Decision 53's refusal for the project settings a headless `who` session would run.
fn settings_refusal(who: &str, paths: &[String]) -> String {
    format!(
        "this repository has project settings that headless {who} sessions would run without asking: {}; review them, then start again with --trust-project",
        paths.join(", ")
    )
}

/// Decision 53's refusal for an edit that would reach `who` (T22-P1, F4): a run started
/// without `--trust-project` trusted only the settings it checked, and `run edit` has
/// no `--trust-project` (a run started with it trusts them all, decision 9).
fn edit_settings_refusal(who: &str, paths: &[String]) -> String {
    format!(
        "this edit would start headless {who} sessions, and this repository has project settings they would run without asking: {}; a run trusts only what its start checked, so review them and start a new run with --trust-project",
        paths.join(", ")
    )
}

/// Decisions 9 and 29's refusal of `run promote`, for the project settings `who`
/// sessions of the promoted run would load unasked.
fn promote_settings_refusal(who: &str, paths: &[String]) -> String {
    format!(
        "promoting would start {who} sessions, and this repository has project settings they would run without asking: {}; this run started without --trust-project, so review them and start a new run with it",
        paths.join(", ")
    )
}

/// A random 64-bit nonce from the standard library's per-process random keys.
fn random_nonce() -> u64 {
    let mut hasher = RandomState::new().build_hasher();
    hasher.write_u64(unix_now());
    hasher.finish().max(1)
}

/// What [`RunService::check_runtimes`] found: decision 50's refusal, and each runtime's
/// project settings (decision 53) with the name its refusal gives it.
#[derive(Default)]
struct RuntimeChecks {
    api_key: Vec<(Runtime, String)>,
    settings: Vec<(Runtime, &'static str, Vec<String>)>,
}

/// Every branch under `refs/heads/anthrex/` (decision 15's redraw test).
fn run_refs(git: &OsString, root: &Path, timeout: Duration) -> Result<Vec<String>, String> {
    let listing = Git::new(git, timeout).ok(
        root,
        &[
            os("for-each-ref"),
            os("--format=%(refname)"),
            os("refs/heads/anthrex/"),
        ],
    )?;
    Ok(listing.lines().map(str::to_string).collect())
}

/// Milestone 9.5 decision 12: one start's tuning, shared by a goal's fast build and the
/// planned build it may fall back to, so the start tunes once; and (ruling T9-5) its one
/// read of the settings, so both builds use the same.
#[derive(Default)]
pub(super) struct TuneOnce {
    config: std::sync::OnceLock<config::Orchestrator>,
    tuned: tokio::sync::OnceCell<crate::run::refit::Tuned>,
    /// The run title change: a goal's run name, asked once per start, so a fast build
    /// that falls back to a planned one does not ask again.
    named: tokio::sync::OnceCell<RunName>,
}

impl TuneOnce {
    pub(super) fn new() -> Self {
        Self::default()
    }

    /// A start's settings read, `config`, already made (a goal's).
    pub(super) fn with_config(config: config::Orchestrator) -> Self {
        TuneOnce {
            config: config.into(),
            ..Self::default()
        }
    }
}

/// What [`RunService::build_plan`] builds: a plan file's run, the fast path's one-task
/// run, or a goal's planned run.
pub(super) enum Shape {
    PlanFile,
    Fast,
    Planned(Box<Planned>),
}

/// Decision 26: what makes a built run a planned one. Milestone 9.3 (D14): a continued
/// goal is not triaged, so its `triage` is `None`.
pub(super) struct Planned {
    pub triage: Option<proto::TriageInfo>,
    pub usage: Option<proto::TokenUsage>,
    pub yes: bool,
    pub choice: Option<proto::OrchestratorChoice>,
    /// Milestone 9.6: the mode the start decided (ruling T3-1); `None`: the build's.
    pub design: Option<proto::DesignMode>,
}

/// Decision 17's `installed`, by how a session is launched (M9.17 fix round 3): each
/// runtime's configured binary resolves to an executable file, directly or on `PATH`.
/// `window` is for the orchestrator, whose window starts the agent through
/// `/bin/sh -c 'exec "$0" "$@"'`; `headless` for the sub-planners and scouts, spawned
/// directly, and it is the map a run records (`run.orch.installed`).
#[derive(Debug, Clone)]
pub(crate) struct Found {
    pub window: BTreeMap<String, bool>,
    pub headless: BTreeMap<String, bool>,
}

/// [`Found`] from the daemon's `PATH` and `HOME` (blocking: it stats files).
pub(crate) fn found(claude: &str, codex: &str) -> Found {
    let (path, home) = (std::env::var_os("PATH"), std::env::var_os("HOME"));
    let macos = cfg!(target_os = "macos");
    found_in((claude, codex), path.as_deref(), home.as_deref(), macos)
}

/// [`Found`] for `bins` (Claude's, then Codex's). Only macOS's `/bin/sh` (bash) expands
/// a `~` entry of `PATH` (Linux's dash does not), and a direct spawn's `PATH` search
/// never does, so only `window` on `macos` reads `~` against `home`.
fn found_in(bins: (&str, &str), path: Option<&OsStr>, home: Option<&OsStr>, macos: bool) -> Found {
    let map = |home: Option<&OsStr>| -> BTreeMap<String, bool> {
        [(Runtime::Claude, bins.0), (Runtime::Codex, bins.1)]
            .into_iter()
            .map(|(runtime, bin)| (runtime.label().to_string(), executable_in(bin, path, home)))
            .collect()
    };
    Found {
        window: map(home.filter(|_| macos)),
        headless: map(None),
    }
}

/// What [`not_installed`] reads: a runtime's configured binary (`bins`: Claude's, then
/// Codex's) when `found` says it is not installed.
fn missing_in<'a>(
    found: &'a BTreeMap<String, bool>,
    bins: &'a (String, String),
) -> impl Fn(Runtime) -> Option<String> + 'a {
    move |runtime| {
        let bin = match runtime {
            Runtime::Claude => &bins.0,
            _ => &bins.1,
        };
        let present = found.get(runtime.label()).copied().unwrap_or(false);
        (!present).then(|| bin.clone())
    }
}

/// Decision 26's check of the sub-planners' runtime (`planner_route`, the run's
/// `planner` row since milestone 9.8, which never moves to another model: D2), on a run
/// whose `orch.installed` is set. The row chose the runtime, so the hint names the role
/// table (C-b S) as what to change (MR §7). A runtime the orchestrator's window finds (`window`, the
/// [`Found::window`] map) only through a `~` entry in `PATH` gets that reason instead
/// (whole-branch review, item 3): another `--orchestrator` would not help.
fn planner_refusal(
    run: &Run,
    missing: Missing<'_>,
    window: &BTreeMap<String, bool>,
) -> Option<String> {
    let route = crate::run::orch::launch::planner_route(run)?;
    let label = route.runtime.label();
    if missing(route.runtime).is_some() && window.get(label) == Some(&true) {
        return Some(format!(
            "the sub-planners' runtime {label} is found only through a `~` entry in PATH, which headless sessions (sub-planners and scouts) do not search; put {label}'s directory in PATH as an absolute path"
        ));
    }
    let hint = "choose another model for the planner in C-b S";
    not_installed("the sub-planners'", route.runtime, missing, hint)
}

/// `bin` is an executable file: itself when it names a path, else in a `PATH` entry.
/// A `PATH` entry of `~` or `~/...` is read against `home` (M9.17 fix round 2); with no
/// `home`, or an empty one, it names nothing (fix round 3: never the relative `bin`).
fn executable_in(bin: &str, path: Option<&OsStr>, home: Option<&OsStr>) -> bool {
    use std::os::unix::fs::PermissionsExt;
    let is = |path: &Path| {
        path.metadata()
            .is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
    };
    if bin.contains('/') {
        return is(Path::new(bin));
    }
    path.is_some_and(|paths| {
        std::env::split_paths(paths)
            .filter_map(|dir| path_entry(dir, home))
            .any(|dir| is(&dir.join(bin)))
    })
}

/// A `PATH` entry as [`executable_in`] reads it: `~` or `~/...` against `home`, and
/// nothing with no `home` or an empty one.
fn path_entry(dir: PathBuf, home: Option<&OsStr>) -> Option<PathBuf> {
    let Ok(rest) = dir.strip_prefix("~") else {
        return Some(dir);
    };
    home.filter(|home| !home.is_empty())
        .map(|home| Path::new(home).join(rest))
}

impl RunService {
    /// Decision 26 on a built run: `installed` and the OTLP token (decision 14a) read
    /// on `spawn_blocking`, then the orchestrator's route (decision 6, from the run's
    /// frozen `[orchestrator.agent]`, default runtime and roster) over the installed
    /// runtimes, then `make_planned`, and the sub-planners' runtime checked installed
    /// (the start check decision 26 cites, M9.17 fix round).
    async fn make_planned(
        &self,
        run: &mut Run,
        planned: Planned,
        design: &config::DesignConfig,
    ) -> Result<(), String> {
        // Milestone 9.6 decision 3: the mode, from the start's settings read, frozen.
        let asked = (GoalOrigin::Goal(planned.triage.as_ref()), planned.design);
        run.design_mode = self.design_mode(run, asked, design).await?;
        let config = self.manager.config();
        let (claude, codex) = (config.claude_bin.clone(), config.codex_bin.clone());
        let bins = (claude.clone(), codex.clone());
        let (found, token) = blocking(move || {
            let token = super::orch_ops::fresh_token()
                .inspect_err(|error| tracing::warn!(%error, "no OTLP token"))
                .unwrap_or_default();
            Ok((found(&claude, &codex), token))
        })
        .await?;
        let window = missing_in(&found.window, &bins);
        let resolved = resolve_planned(run, planned.choice.as_ref(), &window)?;
        make_planned(
            run,
            planned.triage,
            resolved,
            planned.yes,
            found.headless.clone(),
        );
        let headless = missing_in(&found.headless, &bins);
        if let Some(refusal) = planner_refusal(run, &headless, &found.window) {
            return Err(refusal);
        }
        run.triage_usage = planned.usage.unwrap_or_default();
        if let Some(o) = run.orch.orchestrator.as_mut() {
            o.otlp_token = token;
        }
        Ok(())
    }

    /// Everything `run start` checks and builds for a parsed plan (M8b decision 22 shares
    /// it with the fast path, [`Shape::Fast`]: its barrier before the runtime checks,
    /// review m1; milestone 9 decision 26 with the planned path, [`Shape::Planned`]: the
    /// orchestrator's record before the runtime checks, so they cover its runtime and
    /// its sub-planners'); decision 6's profile choice right after preflight. Milestone
    /// 9.2 (decisions 3, 17): the delivery resolved and preflighted right after the
    /// profile choice, or already done before triage (`run start --goal`), and frozen.
    /// Milestone 9.5 decision 12: tuned once per `once`, with its settings read.
    pub(super) async fn build_delivered(
        &self,
        mut plan: Plan,
        dir: PathBuf,
        (yes, trust_project, unconfined_checks): (bool, bool, bool),
        shape: Shape,
        delivery: DeliveryStart,
        once: &TuneOnce,
    ) -> Result<Run, BuildError> {
        let fast = matches!(shape, Shape::Fast);
        let read = || self.ctx.settings.current().orchestrator.clone();
        let mut config = once.config.get_or_init(read).clone();
        // Final fix batch F1c round 2: never run worker-written code unconfined unless
        // the user said so, on the command line or in their own config.
        let available = confine::available();
        let allowed = unconfined_checks || config.unconfined_checks;
        if let Some(refusal) = confine::start_refusal(config.worker_sandbox, available, allowed) {
            return Err(refusal.into());
        }
        let timeout = Duration::from_secs(config.git_timeout_secs);
        let git = self.ctx.git.clone();
        let g = git.clone();
        let mut pre = blocking(move || git::preflight(&g, &dir, timeout)).await?;
        let choice = self.choose_profile(&mut plan, &mut config, &pre).await?;
        let delivery = match delivery {
            DeliveryStart::Done(frozen) => frozen,
            DeliveryStart::Resolve(asked) => {
                let profile = choice.delivery.as_ref();
                self.freeze_delivery(&pre, asked, profile).await?
            }
        };

        // Decision 17 (carry): the protected files, by the chosen profile's matcher,
        // before `build_run` turns them into the plan's warnings (decision 56).
        let profile = resolve_profile(&plan.profile, &config.profile);
        let matcher = ProtectedMatcher::new(&profile.protected)?;
        let (g, root, base) = (git.clone(), pre.root.clone(), pre.base_sha.clone());
        pre.protected_files =
            blocking(move || git::protected_files(&g, &root, &base, &matcher, timeout)).await?;

        let (g, root) = (git.clone(), pre.root.clone());
        let refs = blocking(move || run_refs(&g, &root, timeout)).await?;
        // The run title change: a goal's run (fast, planned, or a chain's next goal) is
        // named before its id is drawn; a plan file's keeps today's id and no title.
        let named = match shape {
            Shape::PlanFile => RunName::default(),
            Shape::Fast | Shape::Planned(_) => {
                let name = self.name_run(&plan.goal, &pre.project, &choice.languages);
                once.named.get_or_init(|| name).await.clone()
            }
        };
        let id = self.pick_id(named.id_head(&plan.goal), &refs)?;
        let wt_dir = repo_worktrees_dir(&self.ctx.worktrees_root, &pre.project);
        let now = unix_now();
        // Milestone 9.5 decision 12 (ruling RH-8): every start kind tunes here, once; a
        // goal whose fast build fell back reuses the first build's (`once`).
        let repo_dir = crate::profile::repo_dir(&self.ctx.data_dir, &pre.project);
        let tune = super::tuning::tune_for_start(&config, &self.tuning, &repo_dir, now);
        let tuning = once.tuned.get_or_init(|| tune).await.clone();
        let (models, models_log) = self.freeze_models(&config.roles, &pre.project).await;
        let ctx = BuildContext {
            id: id.clone(),
            wt_dir,
            data_dir: runs_dir(&self.ctx.data_dir).join(&id),
            config: &config,
            testing: &self.ctx.testing,
            delivery: &self.ctx.delivery,
            now,
            yes,
            tuning,
            models,
            models_log,
        };
        let mut run = crate::run::plan::build_run(plan, pre, ctx).map_err(BuildError::Plan)?;
        run.title = named.title;
        if let Some(usage) = named.usage {
            run.decider_usage += usage;
        }
        delivery.apply(&mut run);
        super::adapt::fast_barrier(fast, &run)?;
        super::adapt::apply_choice(&mut run, choice, now);
        run.limits.unconfined_checks = run.limits.worker_sandbox && !available;
        run.session_nonce = random_nonce();
        run.codex_project_config = Some(self.ctx.cli_caps.codex_project_config());
        // Final fix batch F2 (C-I1): the base's `.codex`, which every Codex session's
        // checkout must still hold when its process starts.
        let (g, root, base) = (git.clone(), run.root.clone(), run.base_sha.clone());
        run.codex_config_base =
            blocking(move || git::codex_config_tree(&g, &root, &base, timeout)).await?;
        if let Shape::Planned(planned) = shape {
            self.make_planned(&mut run, *planned, &config.design)
                .await?;
        }

        let runtimes = reachable_runtimes(&run);
        let checks = self.check_runtimes(&run, &runtimes, timeout).await?;
        if let Some((_, text)) = checks.api_key.first() {
            return Err(text.clone().into());
        }
        if !trust_project {
            let refusals: Vec<String> = checks
                .settings
                .iter()
                .filter(|(_, _, paths)| !paths.is_empty())
                .map(|(_, who, paths)| settings_refusal(who, paths))
                .collect();
            if !refusals.is_empty() {
                return Err(refusals.join("\n").into());
            }
        }
        run.trusted_project = checks
            .settings
            .into_iter()
            .flat_map(|(_, _, paths)| paths)
            .collect();
        run.trusted_project.sort();
        run.trusted_project.dedup();
        run.trust_project = trust_project;
        Ok(run)
    }

    /// Ruling T22-I1b (`run edit`, and milestone 9's `edit_plan` and `submit_epic`):
    /// decisions 50 and 53 for each runtime the run cannot reach yet, with trusted
    /// project settings passing; only a batch that adds, splits or amends is probed.
    pub(super) async fn runtime_refusals(
        &self,
        run_id: &str,
        edits: &[proto::PlanEdit],
    ) -> Result<Vec<(Runtime, String)>, String> {
        let run = crate::lock(&self.state).runs.get(run_id).cloned();
        let mut refusals = Vec::new();
        if let Some(run) = run.filter(|_| edits_may_widen(edits)) {
            let reachable = reachable_runtimes(&run);
            let unreached: Vec<Runtime> = [Runtime::Claude, Runtime::Codex]
                .into_iter()
                .filter(|r| !reachable.contains(r))
                .collect();
            let timeout =
                Duration::from_secs(self.ctx.settings.current().orchestrator.git_timeout_secs);
            let checks = self.check_runtimes(&run, &unreached, timeout).await?;
            refusals = checks.api_key;
            for (runtime, who, paths) in checks.settings {
                // Decision 9 (whole-branch review, item 4): a run started with
                // `--trust-project` trusts them too, as `run promote` does.
                let trusted =
                    run.trust_project || paths.iter().all(|p| run.trusted_project.contains(p));
                if !trusted && !refusals.iter().any(|(r, _)| *r == runtime) {
                    refusals.push((runtime, edit_settings_refusal(who, &paths)));
                }
            }
        }
        Ok(refusals)
    }

    /// Milestone 9 decisions 9 and 29: `run promote` repeats the project-settings check
    /// (and decision 50's) against the run's base commit for the runtimes its new
    /// orchestrator and sub-planners reach and the run does not, honouring the
    /// `--trust-project` the run started with. A request the engine will refuse
    /// anyway (not a fast-path run, an ended one, a route that does not resolve) is left
    /// to the engine.
    ///
    /// M9.17 fix round 2: it also repeats decision 26's installed check, for the new
    /// orchestrator's runtime (the engine resolves it the same way, with no fallback)
    /// and the sub-planners'. `Ok(Some(found))`: its `headless` map is what the promoted
    /// run records as `orch.installed`, so its planners' route at spawn agrees with this
    /// check, and its `window` map is what the promotion reads the `orchestrator` list
    /// over (whole-branch review B, M8, as a planned start does).
    pub(super) async fn promote_refusal(
        &self,
        run_id: &str,
        choice: Option<&proto::OrchestratorChoice>,
    ) -> Result<Option<Found>, String> {
        let run = crate::lock(&self.state).runs.get(run_id).cloned();
        let Some(run) = run.filter(|r| {
            r.path == Some(proto::RunPath::Fast)
                && r.orch.orchestrator.is_none()
                && !r.state.is_terminal()
        }) else {
            return Ok(None);
        };
        let config = self.manager.config();
        let bins = (config.claude_bin.clone(), config.codex_bin.clone());
        let (claude, codex) = bins.clone();
        let found = blocking(move || Ok(found(&claude, &codex))).await?;
        // Milestone 9.8: the choice, else the run's frozen `orchestrator` row.
        let resolved = resolve_promoted(&run, choice);
        let window = missing_in(&found.window, &bins);
        let runtime = resolved.route.runtime;
        let hint = crate::run::orch::installed::ORCHESTRATOR_HINT;
        if let Some(refusal) = not_installed("the orchestrator's", runtime, &window, hint) {
            return Err(refusal);
        }
        let missing = missing_in(&found.headless, &bins);
        let mut promoted = run.clone();
        promoted.orch.installed = found.headless.clone();
        promoted.orch.orchestrator = Some(crate::run::orch::OrchestratorRecord::new(
            resolved.route,
            unix_now(),
        ));
        if let Some(refusal) = planner_refusal(&promoted, &missing, &found.window) {
            return Err(refusal);
        }
        let before = reachable_runtimes(&run);
        let unreached: Vec<Runtime> = reachable_runtimes(&promoted)
            .into_iter()
            .filter(|r| !before.contains(r))
            .collect();
        let timeout = Duration::from_secs(run.limits.git_timeout_secs);
        let checks = self.check_runtimes(&run, &unreached, timeout).await?;
        let mut refusals: Vec<String> = checks.api_key.into_iter().map(|(_, t)| t).collect();
        for (_, who, paths) in checks.settings {
            // Decision 9: a run started with `--trust-project` trusts them too.
            if !run.trust_project && !paths.iter().all(|p| run.trusted_project.contains(p)) {
                refusals.push(promote_settings_refusal(who, &paths));
            }
        }
        if refusals.is_empty() {
            Ok(Some(found.clone()))
        } else {
            Err(refusals.join("\n"))
        }
    }

    /// Decisions 50 and 53 for `runtimes`: decision 50's refusal when Claude is among
    /// them and has no key, and the project settings each runtime's sessions would load
    /// unasked, by the caps the sessions are launched with. Claude's only when it cannot
    /// exclude them; Codex's only when it loads project config with no exclusion.
    async fn check_runtimes(
        &self,
        run: &Run,
        runtimes: &[Runtime],
        timeout: Duration,
    ) -> Result<RuntimeChecks, String> {
        let mut checks = RuntimeChecks::default();
        let claude = runtimes.contains(&Runtime::Claude);
        if claude
            && run.limits.claude_auth == ClaudeAuth::ApiKey
            && std::env::var_os("ANTHROPIC_API_KEY").is_none()
            && run.limits.api_key_helper.is_none()
        {
            checks
                .api_key
                .push((Runtime::Claude, API_KEY_NEEDED.to_string()));
        }
        let caps = self.ctx.cli_caps;
        let codex = runtimes.contains(&Runtime::Codex)
            && caps.codex_loads_project_config
            && caps.codex_user_config_only.is_none();
        let (root, base) = (run.root.clone(), run.base_sha.clone());
        if claude && caps.claude_user_settings_only.is_none() {
            let (g, r, b) = (self.ctx.git.clone(), root.clone(), base.clone());
            let paths =
                blocking(move || git::project_settings(&g, &r, &b, true, None, timeout)).await?;
            checks.settings.push((Runtime::Claude, "Claude", paths));
        }
        if codex {
            let (g, paths) = (self.ctx.git.clone(), caps.codex_project_config_paths);
            let paths = blocking(move || {
                git::project_settings(&g, &root, &base, false, Some(paths), timeout)
            })
            .await?;
            checks.settings.push((Runtime::Codex, "Codex", paths));
        }
        Ok(checks)
    }

    /// Decision 15: the slug of `head` and a random suffix, redrawn while the id's
    /// branches, its data directory or an engine run already take it, or (milestone
    /// 9.3) a chain already has its suffix (`chain::suffix_taken`). `head` is the goal,
    /// or the `run_name` decider's slug (the run title change, `RunName::id_head`).
    pub(super) fn pick_id(&self, head: &str, refs: &[String]) -> Result<String, String> {
        for _ in 0..ID_DRAWS {
            let id = slug(head, random_suffix());
            let taken =
                run_id_taken(&id, refs) || runs_dir(&self.ctx.data_dir).join(&id).exists() || {
                    let state = crate::lock(&self.state);
                    state.runs.contains_key(&id) || suffix_taken(&state.chains, &state.runs, &id)
                };
            if !taken {
                return Ok(id);
            }
        }
        Err("could not pick a free run id".to_string())
    }
}

#[path = "build_installed.rs"]
pub(crate) mod installed;

#[cfg(test)]
#[path = "build_tests.rs"]
mod tests;
