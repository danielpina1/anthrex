//! The background work of one proposal (decisions 8 and 9): the onboarding checkout
//! and scout, then verification in a fresh checkout, then `Ready` (and, for `profile
//! edit --yes`, the confirmation). Every state change goes through
//! [`ProfileService::save_if_current`], so a rejected proposal is never written back.
//!
//! No lock is held here across an `.await`: the table is only read through
//! `current`/`register`/`unregister`, each of which takes and drops it at once.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use proto::{ProfileMeta, ProposalOrigin, ProposalRecord, ProposalState, RepoProfile, ScoutKind};
use tokio_util::sync::CancellationToken;

use super::proposal::{apply_verification, from_findings};
use super::service::{ProfileService, blocking, next_scout_secs};
use super::store;
use super::verify::{self, VerifyJob};
use crate::headless::codex_guard::GuardEntry;
use crate::run::driver::unix_now;
use crate::run::git::{Git, checkout_repo_dir, nul_fields, os};
use crate::run::plan::Preflight;
use crate::scout::contract::onboarding_first_turn;
use crate::scout::service::ScoutOutcome;
use crate::scout::spec::ScoutSpec;
use crate::worktree::repo_worktrees_dir;

/// The question an onboarding scout answers (its `ScoutInfo.question`).
pub const ONBOARDING_QUESTION: &str = "How is this repository set up, built and tested?";

/// One proposal's work.
pub(super) struct Job {
    pub(super) generation: u64,
    pub(super) token: CancellationToken,
    pub(super) pre: Preflight,
    pub(super) record: ProposalRecord,
    pub(super) codex_config: Vec<GuardEntry>,
    /// The onboarding scout's route, picked at the start over what was installed then
    /// (milestone 9.5 rulings RL-2, I6); `None` for a job that starts no scout.
    pub(super) route: Option<proto::Route>,
}

/// How a phase ended short of `Ready`.
enum Stop {
    Failed(String),
    /// Rejected: the proposal is the user's to delete, nothing is written.
    Cancelled,
}

impl ProfileService {
    /// `<wt>/runs/<name>` and its repository `<repo_dir>/tasks/<name>`.
    pub(super) fn checkout(&self, project: &Path, name: &str) -> (PathBuf, PathBuf) {
        let path = repo_worktrees_dir(&self.ctx.worktrees_root, project)
            .join("runs")
            .join(name);
        let repo = checkout_repo_dir(&self.repo_dir(project), &path);
        (path, repo)
    }

    /// The detection checkout `name` salvaged if dirty and removed with its repository,
    /// through the project's write queue; nothing to do when it is not there.
    pub(super) async fn discard_checkout(
        &self,
        project: &Path,
        name: &str,
    ) -> Result<Option<String>, String> {
        let (path, repo) = self.checkout(project, name);
        self.discard_at(project, path, repo).await
    }

    /// The checkout at `path` (its repository `repo`) salvaged if dirty and removed,
    /// behind `root`'s write queue, git run as `root`'s; nothing to do when neither is
    /// there.
    pub(super) async fn discard_at(
        &self,
        root: &Path,
        path: PathBuf,
        repo: PathBuf,
    ) -> Result<Option<String>, String> {
        let (p, r) = (path.clone(), repo.clone());
        let present =
            blocking(move || Ok(std::fs::symlink_metadata(&p).is_ok() || r.exists())).await?;
        if !present {
            return Ok(None);
        }
        let (git, root_dir, timeout) =
            (self.ctx.git.clone(), root.to_path_buf(), self.git_timeout());
        let secs = unix_now();
        self.ctx
            .git_queue
            .write(root, move || {
                verify::discard_named(&git, &root_dir, &path, &repo, secs, timeout)
            })
            .await
    }

    /// Saves `job.record` with `state` (while current). `false`: rejected meanwhile.
    async fn advance(&self, job: &mut Job, state: ProposalState) -> bool {
        job.record.state = state;
        job.record.updated_at = unix_now();
        !job.token.is_cancelled() && self.save_if_current(job.generation, &job.record).await
    }

    /// Detection end to end (decision 8), then its registration ends.
    pub(super) async fn detect_in_background(self: Arc<Self>, mut job: Job) {
        let (project, generation) = (job.pre.project.clone(), job.generation);
        let stop = match self.mark(&project).await {
            Ok(()) => match self.scout_phase(&mut job).await {
                Ok(proposed) => self.verify_phase(&mut job, proposed).await,
                Err(stop) => Err(stop),
            },
            Err(stop) => Err(stop),
        };
        self.end(&mut job, stop).await;
        self.unmark(&project).await;
        self.unregister(&project, generation);
    }

    /// The detection marker (`store::DETECTION_FILE`), before any checkout is made: a
    /// restart then always knows the project to salvage into, even after `reject`
    /// deleted `proposal.json` (task 11 re-review, C1).
    async fn mark(&self, project: &Path) -> Result<(), Stop> {
        let (dir, p) = (self.repo_dir(project), project.to_path_buf());
        blocking(move || store::save_detection(&dir, &p).map_err(|e| e.to_string()))
            .await
            .map_err(|error| Stop::Failed(format!("could not record the detection: {error}")))
    }

    /// Removes the marker once neither detection checkout is left; a checkout the work
    /// could not discard keeps it, for the next start.
    async fn unmark(&self, project: &Path) {
        let left: Vec<(PathBuf, PathBuf)> = [super::ONBOARDING_CHECKOUT, super::VERIFY_CHECKOUT]
            .iter()
            .map(|name| self.checkout(project, name))
            .collect();
        let dir = self.repo_dir(project);
        let removed = blocking(move || {
            let any = left
                .iter()
                .any(|(path, repo)| std::fs::symlink_metadata(path).is_ok() || repo.exists());
            if any {
                return Ok(false);
            }
            store::delete_detection(&dir).map_err(|e| e.to_string())?;
            Ok(true)
        })
        .await;
        match removed {
            Ok(true) => {}
            Ok(false) => {
                tracing::warn!(project = %project.display(), "a detection checkout is left; the next start cleans it")
            }
            Err(error) => tracing::warn!(%error, "could not remove the detection marker"),
        }
    }

    /// An edit's verification (decision 10): no scout, straight to `Verifying`.
    pub(super) async fn verify_in_background(self: Arc<Self>, mut job: Job) {
        let (project, generation) = (job.pre.project.clone(), job.generation);
        let proposed = job.record.proposed.clone().unwrap_or_default();
        let stop = match self.mark(&project).await {
            Ok(()) => self.verify_phase(&mut job, proposed).await,
            Err(stop) => Err(stop),
        };
        self.end(&mut job, stop).await;
        self.unmark(&project).await;
        self.unregister(&project, generation);
    }

    async fn end(&self, job: &mut Job, stop: Result<(), Stop>) {
        if let Err(Stop::Failed(reason)) = stop {
            self.advance(job, ProposalState::Failed { reason }).await;
        }
    }

    /// The checkout's tracked-file count and top-level names at `base`, for the scout's
    /// first message.
    async fn tree_summary(&self, pre: &Preflight) -> Result<(usize, Vec<String>), String> {
        let (g, root, base, timeout) = (
            self.ctx.git.clone(),
            pre.root.clone(),
            pre.base_sha.clone(),
            self.git_timeout(),
        );
        blocking(move || {
            let git = Git::new(&g, timeout);
            let ls = |recursive: bool| {
                let mut args = vec![os("ls-tree"), os("-z"), os("--name-only")];
                if recursive {
                    args.push(os("-r"));
                }
                args.push(os(&base));
                git.ok(&root, &args)
            };
            let tracked = nul_fields(&ls(true)?).count();
            let top = nul_fields(&ls(false)?).map(str::to_string).collect();
            Ok((tracked, top))
        })
        .await
    }

    /// A fresh onboarding scout id (ruling R-T9-2): unique in this daemon, and never
    /// one whose report file is already there. The table is held only to take a number.
    async fn scout_id(&self, project: &Path) -> String {
        let scouts = self
            .repo_dir(project)
            .join(crate::scout::report::SCOUTS_DIR);
        loop {
            let secs = {
                let mut table = crate::lock(&self.table);
                let secs = next_scout_secs(table.last_scout_secs, unix_now());
                table.last_scout_secs = secs;
                secs
            };
            let id = format!("onboarding-{secs}");
            let report = scouts.join(format!("{id}.json"));
            let taken = blocking(move || Ok(std::fs::symlink_metadata(report).is_ok()))
                .await
                .unwrap_or(false);
            if !taken {
                return id;
            }
        }
    }

    /// Notes `id` as the scout this proposal runs, so `reject` can stop it.
    fn note_scout(&self, project: &Path, generation: u64, id: &str) {
        let mut table = crate::lock(&self.table);
        if let Some(active) = table
            .active
            .get_mut(project)
            .filter(|active| active.generation == generation)
        {
            active.scout_id = Some(id.to_string());
        }
    }

    /// Preparing and Scouting (decision 8, steps 1 and 2); the checkout is discarded
    /// whatever happens. `Ok` is the scout's proposed profile.
    async fn scout_phase(&self, job: &mut Job) -> Result<RepoProfile, Stop> {
        let project = job.pre.project.clone();
        let outcome = self.scout(job).await;
        let discarded = self
            .discard_checkout(&project, super::ONBOARDING_CHECKOUT)
            .await;
        let proposed = outcome?;
        if job.token.is_cancelled() {
            return Err(Stop::Cancelled);
        }
        discarded.map_err(|error| {
            Stop::Failed(format!("could not remove the onboarding checkout: {error}"))
        })?;
        Ok(proposed)
    }

    async fn scout(&self, job: &mut Job) -> Result<RepoProfile, Stop> {
        let pre = job.pre.clone();
        let (path, repo) = self.checkout(&pre.project, super::ONBOARDING_CHECKOUT);
        // A leftover of an earlier daemon is salvaged and removed first.
        self.discard_checkout(&pre.project, super::ONBOARDING_CHECKOUT)
            .await
            .map_err(|error| {
                Stop::Failed(format!(
                    "could not discard the leftover onboarding checkout {}: {error}",
                    path.display()
                ))
            })?;
        let (git, p, r, timeout) = (
            self.ctx.git.clone(),
            path.clone(),
            repo.clone(),
            self.git_timeout(),
        );
        let prepared_pre = pre.clone();
        self.ctx
            .git_queue
            .write(&pre.project, move || {
                verify::prepare(&git, &prepared_pre, &p, &r, timeout)
            })
            .await
            .map_err(|error| {
                Stop::Failed(format!(
                    "could not prepare the onboarding checkout: {error}"
                ))
            })?;
        let (tracked, top) = self.tree_summary(&pre).await.map_err(Stop::Failed)?;
        if job.token.is_cancelled() {
            return Err(Stop::Cancelled);
        }
        let id = self.scout_id(&pre.project).await;
        self.note_scout(&pre.project, job.generation, &id);
        job.record.scout_id = Some(id.clone());
        if !self.advance(job, ProposalState::Scouting).await {
            return Err(Stop::Cancelled);
        }
        let spec = ScoutSpec {
            id: id.clone(),
            kind: ScoutKind::Onboarding,
            run_id: None,
            question: ONBOARDING_QUESTION.to_string(),
            first_turn: onboarding_first_turn(&pre.project, &path, tracked, &top),
            cwd: path.clone(),
            project: pre.project.clone(),
            web: false,
            codex_config: job.codex_config.clone(),
            base_sha: pre.base_sha.clone(),
            // Ruling R-T9-1: the checkout's own repository, and the object directory it
            // borrows.
            repo_paths: vec![repo, pre.git_common_dir.join("objects")],
        };
        let started = match job.route.clone() {
            Some(route) => self.scouts.start_on(spec, route).await,
            None => self.scouts.start(spec).await,
        };
        let handle = started.map_err(|error| {
            Stop::Failed(format!("could not start the onboarding scout: {error}"))
        })?;
        job.record.window_id = Some(handle.window_id);
        if !self.advance(job, ProposalState::Scouting).await {
            self.scouts.stop(&id);
            return Err(Stop::Cancelled);
        }
        let outcome = tokio::select! {
            outcome = handle.outcome => outcome,
            _ = job.token.cancelled() => {
                self.scouts.stop(&id);
                return Err(Stop::Cancelled);
            }
        };
        match outcome {
            Ok(ScoutOutcome::Report(report)) => report.profile.ok_or_else(|| {
                Stop::Failed("the onboarding scout reported no profile".to_string())
            }),
            Ok(ScoutOutcome::Failed { reason } | ScoutOutcome::Unsubmitted { reason }) => {
                Err(Stop::Failed(reason))
            }
            // Only a sub-planner's session ends accepted (milestone 9 decision 22).
            Ok(ScoutOutcome::Accepted) => Err(Stop::Failed(
                "the onboarding scout reported nothing".to_string(),
            )),
            Err(_) => Err(Stop::Failed(
                "the onboarding scout ended without an outcome".to_string(),
            )),
        }
    }

    /// Verifying and Ready (decision 8, steps 3 and 4; decision 9). The proposal keeps
    /// the scout's raw profile in `proposed`; verification runs `from_findings`'
    /// (ruling R-T10-1), and an `Err` from it fails the proposal with its reason.
    async fn verify_phase(&self, job: &mut Job, proposed: RepoProfile) -> Result<(), Stop> {
        job.record.proposed = Some(proposed.clone());
        if !self.advance(job, ProposalState::Verifying).await {
            return Err(Stop::Cancelled);
        }
        let findings = from_findings(&proposed);
        let pre = job.pre.clone();
        let repo_dir = self.repo_dir(&pre.project);
        let config = &self.ctx.orchestrator;
        let confine = verify::confine_spec(config, &repo_dir, &pre, &self.ctx.daemon_socket);
        let verify_job = VerifyJob {
            git: self.ctx.git.clone(),
            pre: pre.clone(),
            repo_dir,
            worktrees_root: self.ctx.worktrees_root.clone(),
            profile: findings.clone(),
            confine,
            timeout: std::time::Duration::from_secs(config.onboarding.verify_timeout_secs),
            git_timeout: self.git_timeout(),
            sched: self.ctx.scheduler.clone(),
            token: job.token.clone(),
        };
        let verified = verify::verify(&self.ctx.git_queue, verify_job)
            .await
            .map_err(Stop::Failed)?;
        let (mut profile, dropped) =
            apply_verification(&findings, &verified.verification, &pre.root);
        // Milestone 9.2 decision 3; an edit keeps the stored table the user chose.
        if !matches!(job.record.origin, ProposalOrigin::Edit { .. }) {
            profile.delivery = super::delivery::detected(self.code_host(), pre.root.clone()).await;
        }
        job.record.profile = Some(profile);
        job.record.verification = Some(verified.verification);
        job.record.dropped = dropped;
        if !self.advance(job, ProposalState::Ready).await {
            return Err(Stop::Cancelled);
        }
        if job.record.auto_confirm && self.may_auto_confirm(&job.record) {
            let _writes = self.writes.lock().await;
            if self.current(&pre.project, job.generation) {
                let record = job.record.clone();
                let (dir, project) = (self.repo_dir(&pre.project), pre.project.clone());
                match blocking(move || confirm_record(&dir, &project, &record)).await {
                    Ok(_) => self.note_proposal(&pre.project, None),
                    Err(error) => tracing::warn!(%error, "could not confirm an edited profile"),
                }
            }
        }
        Ok(())
    }

    /// `edit --yes` (task 11 review, I2): only an edit, and only when verification
    /// dropped nothing at all, so no command the user did not touch is lost without a
    /// human confirming it. This implies decision 10's "nothing the edit touched was
    /// dropped".
    fn may_auto_confirm(&self, record: &ProposalRecord) -> bool {
        matches!(record.origin, ProposalOrigin::Edit { .. }) && record.dropped.is_empty()
    }
}

/// Decision 10's confirmation of a `Ready` proposal: `profile.toml` and
/// `profile.meta.json` (the fingerprint computed now), then `proposal.json` deleted.
/// Blocking.
pub fn confirm_record(
    repo_dir: &Path,
    project: &Path,
    record: &ProposalRecord,
) -> Result<String, String> {
    let Some(profile) = record.profile.clone() else {
        return Err("the proposal has no profile".to_string());
    };
    let (report, edited_keys) = match &record.origin {
        ProposalOrigin::Edit { keys } => {
            let mut edited = match store::load(repo_dir) {
                store::Stored::Found { meta, .. } => meta.edited_keys,
                _ => Vec::new(),
            };
            for key in keys {
                if !edited.contains(key) {
                    edited.push(key.clone());
                }
            }
            (None, edited)
        }
        _ => (record.scout_id.clone(), Vec::new()),
    };
    let watched: Vec<String> = profile
        .conventions
        .iter()
        .chain(profile.manifests.iter())
        .cloned()
        .collect();
    let meta = ProfileMeta {
        confirmed_at: unix_now(),
        report,
        verification: record.verification.clone(),
        fingerprint: store::fingerprint(project, &watched),
        edited_keys,
        project: Some(project.to_path_buf()),
    };
    store::save(repo_dir, &profile, &meta).map_err(|e| e.to_string())?;
    store::delete_proposal(repo_dir).map_err(|e| e.to_string())?;
    Ok(format!(
        "stored the profile for {} at {}",
        project.display(),
        repo_dir.join(store::PROFILE_FILE).display()
    ))
}
