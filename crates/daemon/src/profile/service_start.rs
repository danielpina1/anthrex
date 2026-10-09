//! How a proposal's work starts (decisions 8, 9 and 12): the refusals first (work
//! already running, verification that cannot be confined without the user's leave,
//! project settings a scout would run unasked), then the registration, the first
//! `proposal.json` and the background task.

use std::sync::Arc;

use proto::{ProposalOrigin, ProposalRecord, ProposalState, Route, Runtime};

use super::service::{ProfileService, already_running, blocking};
use super::store;
use crate::headless::argv::CodexProjectConfig;
use crate::run::confine;
use crate::run::driver::build::installed::installed_with;
use crate::run::driver::unix_now;
use crate::run::git;
use crate::run::plan::Preflight;
use config::models::{REPO_FILE, load_repo};

/// Decision 12's refusal of project settings a scout would run without asking.
pub fn settings_refusal(paths: &[String]) -> String {
    format!(
        "this repository has project settings that headless sessions would run without asking: {}; review them, then detect again with --trust-project",
        paths.join(", ")
    )
}

impl ProfileService {
    /// Decision 12's project-settings check for the scout this service would start,
    /// and, for a Codex scout that loads project config, the `.codex` entries its guard
    /// needs. `Err` is the refusal (or a git failure).
    pub(super) async fn scout_checks(
        &self,
        pre: &Preflight,
        trust_project: bool,
    ) -> Result<
        (
            Vec<String>,
            Vec<crate::headless::codex_guard::GuardEntry>,
            Route,
        ),
        String,
    > {
        let ctx = self.scouts.context();
        // Milestone 9.5 rulings RL-2, I6: over what is installed now, never under a lock;
        // milestone 9.8: with the repository's `models.toml`, read on the same step.
        let config = self.manager.config();
        let (claude, codex) = (config.claude_bin.clone(), config.codex_bin.clone());
        let file = super::repo_dir(&self.ctx.data_dir, &pre.project).join(REPO_FILE);
        let read = move || load_repo(&file);
        let (installed, read) = installed_with(claude, codex, read).await;
        let (repo, problems) = read.unwrap_or_default();
        for problem in problems {
            tracing::warn!("{}", proto::safe_text::one_line(&problem));
        }
        let route = onboarding_route(ctx, repo.as_ref(), &installed);
        let runtime = route.runtime;
        let caps = self.ctx.cli_caps;
        let timeout = self.git_timeout();
        let codex_loaded =
            runtime == Runtime::Codex && caps.codex_project_config() == CodexProjectConfig::Loaded;
        let claude_unexcluded =
            runtime == Runtime::Claude && caps.claude_user_settings_only.is_none();
        let mut paths = Vec::new();
        if claude_unexcluded || codex_loaded {
            let (g, root, base) = (self.ctx.git.clone(), pre.root.clone(), pre.base_sha.clone());
            let codex = codex_loaded.then_some(caps.codex_project_config_paths);
            paths = blocking(move || {
                git::project_settings(&g, &root, &base, claude_unexcluded, codex, timeout)
            })
            .await?;
        }
        if !paths.is_empty() && !trust_project {
            return Err(settings_refusal(&paths));
        }
        let guard = if codex_loaded {
            let (g, root, base) = (self.ctx.git.clone(), pre.root.clone(), pre.base_sha.clone());
            blocking(move || git::codex_config_tree(&g, &root, &base, timeout)).await?
        } else {
            Vec::new()
        };
        Ok((paths, guard, route))
    }

    /// Decision 8: starts detection for the repository `pre` describes, answering at
    /// once; the work goes on in the background. `Err` is the refusal: detection
    /// already running, a platform that cannot confine verification without the user's
    /// leave (decision 9, M8a's `start_refusal`), or project settings a scout would run
    /// unasked (decision 12).
    pub async fn start_detection(
        self: &Arc<Self>,
        pre: &Preflight,
        origin: ProposalOrigin,
        trust_project: bool,
        unconfined_checks: bool,
    ) -> Result<(), String> {
        if let Some(state) = self.running(&pre.project).await {
            return Err(already_running(&pre.project, &state));
        }
        self.confinement_refusal(unconfined_checks)?;
        let (trusted, codex_config, route) = self.scout_checks(pre, trust_project).await?;
        let Some((generation, token)) = self.register(&pre.project) else {
            return Err(already_running(&pre.project, &ProposalState::Preparing));
        };
        let now = unix_now();
        let record = ProposalRecord {
            project: pre.project.clone(),
            state: ProposalState::Preparing,
            origin,
            started_at: now,
            updated_at: now,
            base_sha: pre.base_sha.clone(),
            scout_id: None,
            window_id: None,
            profile: None,
            verification: None,
            dropped: Vec::new(),
            proposed: None,
            trusted_project: trusted,
            unconfined_checks,
            auto_confirm: false,
            edit: None,
        };
        self.save_if_current(generation, &record).await;
        let job = super::service_run::Job {
            generation,
            token,
            pre: pre.clone(),
            record,
            codex_config,
            route: Some(route),
        };
        tokio::spawn(self.clone().detect_in_background(job));
        Ok(())
    }

    /// M8a's `start_refusal` for verification (decision 9, ruling R-T10-1): a platform
    /// that cannot confine it is refused unless the request or the config allows it.
    pub(super) fn confinement_refusal(&self, unconfined_checks: bool) -> Result<(), String> {
        let config = &self.ctx.orchestrator;
        match confine::start_refusal(
            config.worker_sandbox,
            confine::available(),
            unconfined_checks || config.unconfined_checks,
        ) {
            Some(refusal) => Err(refusal),
            None => Ok(()),
        }
    }

    /// An automatic detection (decisions 7 and 22): a refusal is stored as the
    /// proposal's `Failed` reason, unless the refusal is that one already runs.
    pub async fn auto_detect(self: &Arc<Self>, pre: &Preflight, origin: ProposalOrigin) {
        self.detect_or_record(pre, origin, false, false).await;
    }

    /// `[orchestrator.onboarding] auto`.
    pub fn onboarding_auto(&self) -> bool {
        self.ctx.orchestrator.onboarding.auto
    }

    /// [`auto_detect`](Self::auto_detect) with the request's own flags (decision 22: a
    /// goal's `--trust-project` and `--unconfined-checks`).
    pub async fn detect_or_record(
        self: &Arc<Self>,
        pre: &Preflight,
        origin: ProposalOrigin,
        trust_project: bool,
        unconfined_checks: bool,
    ) {
        let Err(reason) = self
            .start_detection(pre, origin.clone(), trust_project, unconfined_checks)
            .await
        else {
            return;
        };
        if self.running(&pre.project).await.is_some() {
            return;
        }
        let now = unix_now();
        let record = ProposalRecord {
            project: pre.project.clone(),
            state: ProposalState::Failed { reason },
            origin,
            started_at: now,
            updated_at: now,
            base_sha: pre.base_sha.clone(),
            scout_id: None,
            window_id: None,
            profile: None,
            verification: None,
            dropped: Vec::new(),
            proposed: None,
            trusted_project: Vec::new(),
            unconfined_checks: false,
            auto_confirm: false,
            edit: None,
        };
        let _writes = self.writes.lock().await;
        // Review m5: a detection that registered meanwhile keeps its proposal.
        if self.running(&pre.project).await.is_some() {
            return;
        }
        let dir = self.repo_dir(&pre.project);
        let written = record.clone();
        match blocking(move || store::save_proposal(&dir, &record).map_err(|e| e.to_string())).await
        {
            Ok(()) => self.note_proposal(&pre.project, Some(&written)),
            Err(error) => tracing::warn!(%error, "could not record a refused automatic detection"),
        }
    }
}

/// Milestone 9.8 (MR §3.1): the onboarding scout's route, the `research` row of
/// `repo` (the repository's `models.toml`) over the live table, over what its start
/// found `installed` (empty: everything counts as installed): on the row's fallback
/// only when the row's runtime is missing and the fallback's is not (D2).
pub(super) fn onboarding_route(
    ctx: &crate::scout::spec::ScoutContext,
    repo: Option<&proto::models::ModelTable>,
    installed: &crate::run::model_roles::Installed,
) -> Route {
    let research = proto::models::Role::Research;
    let choice = config::models::resolve(research, repo, &ctx.roster.roles());
    crate::run::model_roles::row_route_over(&choice, installed).0
}
