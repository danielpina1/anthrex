//! `anthrex profile status|detect|show|confirm|reject|edit` (decision 10). Each answers
//! at once; detection and verification go on in the background (`service_run.rs`).

use std::path::{Path, PathBuf};
use std::sync::Arc;

use proto::{
    ProfileReply, ProfileSource, ProfileStatus, ProposalOrigin, ProposalRecord, ProposalState,
};

use super::proposal::{apply_edit, show_text};
use super::service::{
    ProfileService, already_running, auto_allowed, blocking, in_progress, state_label,
};
use super::service_queue::{DISCARDED, dropped_note};
use super::service_run::{Job, confirm_record};
use super::store::{self, Stored};
use crate::run::driver::unix_now;
use crate::run::git;
use crate::run::plan::Preflight;

/// Decision 6's refusal text for a stored profile that does not parse.
fn unparseable(path: &Path, error: &str) -> String {
    format!(
        "the stored profile at {} does not parse: {error}; fix it with anthrex profile edit or re-detect it with anthrex profile detect",
        path.display()
    )
}

fn no_stored(project: &Path) -> String {
    format!(
        "no stored profile for {}; run anthrex profile detect first",
        project.display()
    )
}

/// Milestone 9.0.6 decision 37: an edit waits for the runs live in its project.
pub fn live_run(run: &str, project: &Path) -> String {
    format!(
        "run {run} is live in {}; edit the profile once it finishes (runs keep the profile they started with)",
        project.display()
    )
}

/// Refused while a rejected detection still cleans up.
fn stopping(project: &Path) -> String {
    format!(
        "detection for {} is stopping after anthrex profile reject; try again in a moment",
        project.display()
    )
}

impl ProfileService {
    /// The project (main checkout) `dir` belongs to, without preflight's clean-tree
    /// rules: `status`, `show`, `confirm`, `reject` and a non-command `edit` work in a
    /// dirty checkout.
    async fn project_of(&self, dir: PathBuf) -> Result<PathBuf, String> {
        let (g, timeout) = (self.ctx.git.clone(), self.git_timeout());
        blocking(move || {
            let roots = crate::project::detect_roots_with(&g, &dir, timeout);
            match roots.worktree {
                Some(_) => Ok(roots.project),
                None if roots.detection_failed => Err(format!(
                    "could not tell whether {} is a git repository: git did not answer; try again",
                    dir.display()
                )),
                None => Err(format!("not a git repository: {}", dir.display())),
            }
        })
        .await
    }

    pub(super) async fn preflight(&self, dir: PathBuf) -> Result<Preflight, String> {
        let (g, timeout) = (self.ctx.git.clone(), self.git_timeout());
        blocking(move || git::preflight(&g, &dir, timeout)).await
    }

    async fn load(&self, project: &Path) -> Result<(Stored, Option<ProposalRecord>), String> {
        let dir = self.repo_dir(project);
        blocking(move || Ok((store::load(&dir), store::load_proposal(&dir)?))).await
    }

    /// Refuses when work for `project` runs (or a rejected one still cleans up).
    async fn refuse_if_running(&self, project: &Path) -> Result<(), String> {
        let stopping_now = crate::lock(&self.table)
            .active
            .get(project)
            .is_some_and(|active| active.token.is_cancelled());
        if stopping_now {
            return Err(stopping(project));
        }
        match self.running(project).await {
            Some(state) => Err(already_running(project, &state)),
            None => Ok(()),
        }
    }

    /// `profile status`. A stale stored profile starts a re-detection when
    /// `onboarding.auto` allows it (decision 7).
    pub(super) async fn status(self: &Arc<Self>, dir: PathBuf) -> Result<ProfileStatus, String> {
        let project = self.project_of(dir.clone()).await?;
        let (stored, mut proposal) = self.load(&project).await?;
        let (mut source, mut confirmed_at, mut stale, mut unparseable) =
            (ProfileSource::None, None, Vec::new(), None);
        let (mut verified_at, mut unreadable_text) = (None, None);
        match stored {
            Stored::Found { meta, .. } => {
                source = ProfileSource::Stored;
                confirmed_at = Some(meta.confirmed_at);
                // Decision 23: an edit-only profile has no verification record.
                verified_at = Some(
                    meta.verification
                        .as_ref()
                        .map_or(meta.confirmed_at, |v| v.at),
                );
                let p = project.clone();
                stale = blocking(move || Ok(store::stale(&p, &meta))).await?;
            }
            Stored::Unparseable { path, error } => {
                unparseable = Some(unparseable_text(&path, &error));
                // Decision 32: the file's own text, for the screen's raw-text page.
                unreadable_text = blocking(move || Ok(store::load_text(&path))).await?;
            }
            Stored::Absent => {}
        }
        let idle = !crate::lock(&self.table).active.contains_key(&project);
        if self.ctx.orchestrator.onboarding.auto
            && !stale.is_empty()
            && idle
            && auto_allowed(proposal.as_ref(), unix_now())
        {
            match self.preflight(dir).await {
                Ok(pre) => {
                    self.auto_on_stale(&pre, stale.clone()).await;
                    proposal = self.load(&project).await?.1;
                }
                Err(error) => tracing::info!(%error, "no automatic re-detection"),
            }
        }
        let scout = proposal
            .as_ref()
            .and_then(|record| record.scout_id.as_deref())
            .and_then(|id| self.scouts.info(id));
        // Decision 10: only while the proposal verifies, from its running counter.
        let checking = match proposal.as_ref().map(|record| &record.state) {
            Some(ProposalState::Verifying) => crate::lock(&self.table)
                .active
                .get(&project)
                .and_then(|active| active.counter.progress()),
            _ => None,
        };
        let (mut queued, dropped_goals) = self.queued_for(&project);
        // M9.10.5: the goals' set-up reads the proposal this reply shows; memory's copy
        // (`Table.states`) is noted just after each write, so it can trail the file.
        let setup = super::queue::setup_state(proposal.as_ref().map(|r| &r.state), checking);
        for goal in &mut queued {
            goal.setup = setup.clone();
        }
        Ok(ProfileStatus {
            repo_dir: self.repo_dir(&project),
            project,
            source,
            confirmed_at,
            stale,
            unparseable,
            proposal,
            scout,
            verify_confined: self.verify_confined(),
            queued,
            checking,
            verified_at,
            unreadable_text,
            dropped_goals,
        })
    }

    /// `profile detect`.
    pub(super) async fn detect(
        self: &Arc<Self>,
        dir: PathBuf,
        trust_project: bool,
        unconfined_checks: bool,
    ) -> Result<ProfileReply, String> {
        let pre = self.preflight(dir).await?;
        self.refuse_if_running(&pre.project).await?;
        self.start_detection(
            &pre,
            ProposalOrigin::Detect,
            trust_project,
            unconfined_checks,
        )
        .await?;
        Ok(ProfileReply::Done {
            message: format!(
                "detection started for {}; follow it with anthrex profile status",
                pre.project.display()
            ),
        })
    }

    /// `profile show [--proposed]`: the stored profile, or the ready proposal, as
    /// `proposal::show_text`. `source` is where the stored profile stands.
    pub(super) async fn show(&self, dir: PathBuf, proposed: bool) -> Result<ProfileReply, String> {
        let project = self.project_of(dir).await?;
        let (stored, proposal) = self.load(&project).await?;
        let source = match stored {
            Stored::Found { .. } => ProfileSource::Stored,
            _ => ProfileSource::None,
        };
        if proposed {
            let record = proposal.ok_or_else(|| {
                format!(
                    "no proposal for {}; run anthrex profile detect",
                    project.display()
                )
            })?;
            let profile = ready_profile(&record)?;
            return Ok(ProfileReply::Shown {
                source,
                toml: show_text(&profile, record.verification.as_ref(), &record.dropped),
                meta: None,
                verification: record.verification,
                dropped: record.dropped,
            });
        }
        match stored {
            Stored::Found { profile, meta, .. } => Ok(ProfileReply::Shown {
                source,
                toml: show_text(&profile, meta.verification.as_ref(), &[]),
                verification: meta.verification.clone(),
                meta: Some(meta),
                dropped: Vec::new(),
            }),
            Stored::Unparseable { path, error } => Err(unparseable(&path, &error)),
            Stored::Absent => Err(no_stored(&project)),
        }
    }

    /// `profile confirm`: the ready proposal stored, the proposal deleted.
    pub(super) async fn confirm(
        self: &Arc<Self>,
        dir: PathBuf,
        shown: Option<String>,
    ) -> Result<ProfileReply, String> {
        let project = self.project_of(dir).await?;
        let _writes = self.writes.lock().await;
        let active = crate::lock(&self.table).active.contains_key(&project);
        if active {
            return Err(match self.running(&project).await {
                Some(state) => already_running(&project, &state),
                None => stopping(&project),
            });
        }
        let record = self.load(&project).await?.1.ok_or_else(|| {
            format!(
                "no proposal to confirm for {}; run anthrex profile detect",
                project.display()
            )
        })?;
        let profile = ready_profile(&record)?;
        // Review m3: only the proposal the user was shown is stored.
        let current = show_text(&profile, record.verification.as_ref(), &record.dropped);
        if shown.is_some_and(|shown| shown != current) {
            return Err(changed_since_shown(&project));
        }
        let dir = self.repo_dir(&project);
        let p = project.clone();
        let message = blocking(move || confirm_record(&dir, &p, &record)).await?;
        self.note_proposal(&project, None);
        // Milestone 9.10 decision 6: the queued goals start once the store is done.
        let starting = self.drain_after_store(&project);
        Ok(ProfileReply::Done {
            message: format!("{message}{starting}"),
        })
    }

    /// `profile reject`: a running scout stopped (through `ScoutService::stop`, the
    /// existing kill), the detection checkouts salvaged and removed (by the stopped
    /// work, or here when none runs), and `proposal.json` deleted.
    pub(super) async fn reject(&self, dir: PathBuf) -> Result<ProfileReply, String> {
        let project = self.project_of(dir).await?;
        let writes = self.writes.lock().await;
        let running = {
            let mut table = crate::lock(&self.table);
            table.active.get_mut(&project).map(|active| {
                active.token.cancel();
                active.scout_id.clone()
            })
        };
        if let Some(Some(id)) = &running {
            self.scouts.stop(id);
        }
        let repo_dir = self.repo_dir(&project);
        let existed = blocking(move || {
            let existed = store::load_proposal(&repo_dir).ok().flatten().is_some();
            store::delete_proposal(&repo_dir).map_err(|e| e.to_string())?;
            Ok(existed)
        })
        .await?;
        self.note_proposal(&project, None);
        if running.is_none() {
            for name in [super::ONBOARDING_CHECKOUT, super::VERIFY_CHECKOUT] {
                self.discard_checkout(&project, name).await?;
            }
            let dir = self.repo_dir(&project);
            blocking(move || store::delete_detection(&dir).map_err(|e| e.to_string())).await?;
        }
        drop(writes);
        // Milestone 9.10 decision 8: a goal cannot start without the proposal.
        let dropped = self.drop_queued(&project, DISCARDED).await;
        if running.is_none() && !existed && dropped == 0 {
            return Err(format!("no proposal for {}", project.display()));
        }
        Ok(ProfileReply::Done {
            message: format!(
                "rejected the proposal for {}{}",
                project.display(),
                dropped_note(dropped)
            ),
        })
    }

    /// `profile edit`: the stored profile with one value changed becomes a proposal,
    /// verified again when a command's input changed (decision 10).
    pub(super) async fn edit(
        self: &Arc<Self>,
        dir: PathBuf,
        key: String,
        value: Option<String>,
        yes: bool,
        unconfined_checks: bool,
    ) -> Result<ProfileReply, String> {
        let project = self.project_of(dir.clone()).await?;
        if let Some(run) = self.live_runs(&project).first() {
            return Err(live_run(run, &project));
        }
        self.refuse_if_running(&project).await?;
        let (stored, meta) = match self.load(&project).await?.0 {
            Stored::Found { profile, meta, .. } => (profile, meta),
            Stored::Unparseable { path, error } => return Err(unparseable(&path, &error)),
            Stored::Absent => return Err(no_stored(&project)),
        };
        let (edited, reverify) = apply_edit(&stored, &key, value.as_deref())?;
        let shown = format!(
            "proposed: {key} = {}",
            value.as_deref().unwrap_or("(unset)")
        );
        let now = unix_now();
        let mut record = ProposalRecord {
            project: project.clone(),
            state: ProposalState::Ready,
            origin: ProposalOrigin::Edit {
                keys: vec![key.clone()],
            },
            started_at: now,
            updated_at: now,
            base_sha: String::new(),
            scout_id: None,
            window_id: None,
            profile: None,
            verification: None,
            dropped: Vec::new(),
            proposed: Some(edited.clone()),
            trusted_project: Vec::new(),
            unconfined_checks,
            auto_confirm: yes,
            edit: None,
        };
        if reverify {
            self.confinement_refusal(unconfined_checks)?;
            let pre = self.preflight(dir).await?;
            let Some((generation, token)) = self.register(&project) else {
                return Err(already_running(&project, &ProposalState::Verifying));
            };
            record.state = ProposalState::Verifying;
            record.base_sha = pre.base_sha.clone();
            self.save_if_current(generation, &record).await;
            let job = Job {
                generation,
                token,
                pre,
                record,
                codex_config: Vec::new(),
                route: None,
            };
            tokio::spawn(self.clone().verify_in_background(job));
            return Ok(ProfileReply::Done {
                message: if yes {
                    format!(
                        "{shown}; it is stored as soon as verification passes (anthrex profile status)"
                    )
                } else {
                    format!(
                        "{shown}; verifying (anthrex profile status), then confirm with anthrex profile confirm"
                    )
                },
            });
        }
        record.profile = Some(edited);
        record.verification = meta.verification;
        let _writes = self.writes.lock().await;
        let active = crate::lock(&self.table).active.contains_key(&project);
        if active {
            return Err(stopping(&project));
        }
        let repo_dir = self.repo_dir(&project);
        let (p, written) = (project.clone(), record.clone());
        blocking(move || {
            if yes {
                confirm_record(&repo_dir, &p, &record).map(|_| ())
            } else {
                store::save_proposal(&repo_dir, &record).map_err(|e| e.to_string())
            }
        })
        .await?;
        // `edit --yes` deleted the proposal; otherwise it is the ready one just written.
        self.note_proposal(&project, (!yes).then_some(&written));
        Ok(ProfileReply::Done {
            message: if yes {
                format!("{shown}; stored (it needed no verification)")
            } else {
                format!("{shown}; confirm with anthrex profile confirm")
            },
        })
    }
}

/// Review m3's refusal: the proposal changed between `show` and `confirm`.
pub fn changed_since_shown(project: &Path) -> String {
    format!(
        "the proposal for {} changed since it was shown; run anthrex profile confirm again",
        project.display()
    )
}

fn unparseable_text(path: &Path, error: &str) -> String {
    format!("{}: {error}", path.display())
}

/// The proposal's profile when it is `Ready`, else why it cannot be shown or stored.
pub(super) fn ready_profile(record: &ProposalRecord) -> Result<proto::RepoProfile, String> {
    match (&record.state, &record.profile) {
        (ProposalState::Ready, Some(profile)) => Ok(profile.clone()),
        (ProposalState::Failed { reason }, _) => Err(format!(
            "the proposal for {} failed: {reason}; run anthrex profile detect",
            record.project.display()
        )),
        (state, _) if in_progress(state) => Err(format!(
            "the proposal for {} is not ready yet (state {}); see anthrex profile status",
            record.project.display(),
            state_label(state)
        )),
        _ => Err(format!(
            "the proposal for {} has no profile",
            record.project.display()
        )),
    }
}
