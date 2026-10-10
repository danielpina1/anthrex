//! `anthrex profile edit` and `RevertEdit` (milestone 9.10 decisions 15 to 21): a row
//! edit of the stored profile or of a review proposal, checked once, then saved, saved
//! anyway or reverted.
//!
//! - On the stored profile the edit becomes an `Edit`-origin proposal, verified by
//!   `verify_in_background` → `verify_phase` as before; `settle_row_edit` gives its ✓/✗.
//! - On a review proposal the proposal stays `Ready` throughout: `verify_row_edit` checks
//!   the edited profile and writes only its outcome.
//!
//! Locks as in `service.rs`: the table only in one-expression blocks, every file on
//! `blocking`, and each check-then-write of `proposal.json` under one hold of `writes`.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use proto::{
    ProfileReply, ProposalOrigin, ProposalRecord, ProposalState, RepoProfile, RowEdit, RowEditState,
};
use tokio_util::sync::CancellationToken;

use super::proposal::apply_edit;
use super::row_edit::{
    changed, failed_edit, is_review, no_failed_edit, no_proposal_to_edit, proposal_outcome,
    review_waiting, saved_anyway, still_checking,
};
use super::service::{ProfileService, already_running, blocking};
use super::service_requests::{LIVE_RUN, no_stored, ready_profile, unparseable};
use super::service_run::{Job, confirm_record};
use super::store::{self, Stored};
use crate::run::driver::unix_now;

/// One `ProfileRequest::Edit` (Interfaces).
pub(super) struct EditRequest {
    pub dir: PathBuf,
    pub key: String,
    pub value: Option<String>,
    pub yes: bool,
    pub unconfined_checks: bool,
    pub anyway: bool,
    pub on_proposal: bool,
}

fn done(message: String) -> Result<ProfileReply, String> {
    Ok(ProfileReply::Done { message })
}

impl ProfileService {
    /// `profile edit` (decisions 15 to 20), on the stored profile or on the ready review
    /// proposal.
    pub(super) async fn edit(
        self: &Arc<Self>,
        request: EditRequest,
    ) -> Result<ProfileReply, String> {
        let project = self.project_of(request.dir.clone()).await?;
        // Decision 20: both targets, `anyway` too.
        if !self.live_runs(&project).is_empty() {
            return Err(LIVE_RUN.to_string());
        }
        let shown = format!(
            "proposed: {} = {}",
            request.key,
            request.value.as_deref().unwrap_or("(unset)")
        );
        if request.on_proposal {
            self.edit_proposal(project, request, shown).await
        } else {
            self.edit_stored(project, request, shown).await
        }
    }

    /// The stored profile with one value changed becomes an `Edit`-origin proposal,
    /// verified again when a command's input changed (M8b decision 10).
    async fn edit_stored(
        self: &Arc<Self>,
        project: PathBuf,
        request: EditRequest,
        shown: String,
    ) -> Result<ProfileReply, String> {
        let EditRequest {
            dir,
            key,
            value,
            yes,
            unconfined_checks,
            anyway,
            ..
        } = request;
        let yes = yes || anyway;
        let (stored, proposal) = self.load(&project).await?;
        // Decision 15: never written over a proposal the user chose to review later.
        let waiting = proposal.as_ref().is_some_and(|record| {
            is_review(record) && !matches!(record.state, ProposalState::Failed { .. })
        });
        if waiting {
            return Err(review_waiting(&project));
        }
        // Before `refuse_if_running`: the ✗ is written a moment before its work ends.
        if anyway && self.store_anyway(&project, &key, value.as_deref()).await? {
            return done(format!("{shown}; stored although its check failed"));
        }
        self.refuse_if_running(&project).await?;
        let (stored, meta) = match stored {
            Stored::Found { profile, meta, .. } => (profile, meta),
            Stored::Unparseable { path, error } => return Err(unparseable(&path, &error)),
            Stored::Absent => return Err(no_stored(&project)),
        };
        let (edited, reverify) = apply_edit(&stored, &key, value.as_deref())?;
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
            record.edit = Some(RowEdit {
                key,
                value,
                state: RowEditState::Verifying,
            });
            self.save_if_current(generation, &record).await;
            let job = Job {
                generation,
                token,
                pre,
                record,
                codex_config: Vec::new(),
                route: None,
                anyway,
            };
            tokio::spawn(self.clone().verify_in_background(job));
            return done(if anyway {
                format!(
                    "{shown}; it is stored once verification has run, whatever it finds (anthrex profile status)"
                )
            } else if yes {
                format!(
                    "{shown}; it is stored as soon as verification passes (anthrex profile status)"
                )
            } else {
                format!(
                    "{shown}; verifying (anthrex profile status), then confirm with anthrex profile confirm"
                )
            });
        }
        record.profile = Some(edited);
        record.verification = meta.verification;
        let _writes = self.writes.lock().await;
        let active = crate::lock(&self.table).active.contains_key(&project);
        if active {
            return Err(super::service_requests::stopping(&project));
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
        done(if yes {
            format!("{shown}; stored (it needed no verification)")
        } else {
            format!("{shown}; confirm with anthrex profile confirm")
        })
    }

    /// Decision 18 on the stored profile: the held ✗ edit of `key` set to `value` is
    /// stored as typed (`record.proposed`) with its verification, ✗ kept. `false`: there
    /// is no such edit, so the caller verifies once.
    async fn store_anyway(
        &self,
        project: &Path,
        key: &str,
        value: Option<&str>,
    ) -> Result<bool, String> {
        let _writes = self.writes.lock().await;
        let (dir, p) = (self.repo_dir(project), project.to_path_buf());
        let (key, value) = (key.to_string(), value.map(str::to_string));
        let stored = blocking(move || {
            let Some(mut record) = store::load_proposal(&dir)? else {
                return Ok(false);
            };
            if is_review(&record) || failed_edit(&record, &key, value.as_deref()).is_none() {
                return Ok(false);
            }
            record.profile = record.proposed.clone();
            confirm_record(&dir, &p, &record).map(|_| true)
        })
        .await?;
        if stored {
            self.note_proposal(project, None);
        }
        Ok(stored)
    }

    /// Decision 15 on a review proposal: it must be `Ready` with no row edit verifying;
    /// a non-command key is written at once, a command key is checked by
    /// `verify_row_edit` while the proposal stays `Ready`.
    async fn edit_proposal(
        self: &Arc<Self>,
        project: PathBuf,
        request: EditRequest,
        shown: String,
    ) -> Result<ProfileReply, String> {
        let EditRequest {
            dir,
            key,
            value,
            unconfined_checks,
            anyway,
            ..
        } = request;
        let record = self.load(&project).await?.1.filter(is_review);
        let record = record.ok_or_else(|| no_proposal_to_edit(&project))?;
        if let Some(edit) = &record.edit
            && edit.state == RowEditState::Verifying
        {
            return Err(still_checking(&edit.key));
        }
        // Other work is refused under `writes` (`write_proposal`, `start_row_edit`), where
        // a row edit's outcome and its end are one step.
        let profile = ready_profile(&record)?;
        let (edited, reverify) = apply_edit(&profile, &key, value.as_deref())?;
        let mut next = record.clone();
        next.updated_at = unix_now();
        if let Some(failed) = anyway
            .then(|| failed_edit(&record, &key, value.as_deref()))
            .flatten()
        {
            saved_anyway(
                &mut next,
                edited,
                &key,
                failed,
                (unix_now(), self.verify_confined()),
            );
            self.write_proposal(&project, &record, &next).await?;
            return done(format!(
                "{shown}; saved in the proposal although its check failed"
            ));
        }
        if !reverify {
            next.profile = Some(edited);
            if next.edit.as_ref().is_some_and(|edit| edit.key == key) {
                next.edit = None;
            }
            self.write_proposal(&project, &record, &next).await?;
            return done(format!("{shown}; saved in the proposal"));
        }
        self.confinement_refusal(unconfined_checks || record.unconfined_checks)?;
        let pre = self.preflight(dir).await?;
        next.edit = Some(RowEdit {
            key,
            value,
            state: RowEditState::Verifying,
        });
        let (generation, token) = self.start_row_edit(&project, &record, &next).await?;
        let job = Job {
            generation,
            token,
            pre,
            record: next,
            codex_config: Vec::new(),
            route: None,
            anyway,
        };
        tokio::spawn(self.clone().verify_row_edit(job, edited));
        done(format!(
            "{shown}; checking it in the proposal (anthrex profile status)"
        ))
    }

    /// `next` written over `expected`, which must still be `proposal.json`, under one
    /// hold of `writes`.
    async fn write_proposal(
        &self,
        project: &Path,
        expected: &ProposalRecord,
        next: &ProposalRecord,
    ) -> Result<(), String> {
        let _writes = self.writes.lock().await;
        if let Some(refusal) = self.busy(project) {
            return Err(refusal);
        }
        self.save_over(project, expected, next).await
    }

    /// Why `project`'s work refuses a proposal edit: a reject still stopping it, or work
    /// that runs. Called under `writes`.
    fn busy(&self, project: &Path) -> Option<String> {
        let cancelled = crate::lock(&self.table)
            .active
            .get(project)
            .map(|active| active.token.is_cancelled());
        match cancelled {
            Some(true) => Some(super::service_requests::stopping(project)),
            Some(false) => Some(already_running(project, &ProposalState::Ready)),
            None => None,
        }
    }

    async fn save_over(
        &self,
        project: &Path,
        expected: &ProposalRecord,
        next: &ProposalRecord,
    ) -> Result<(), String> {
        let (dir, expected, written) = (self.repo_dir(project), expected.clone(), next.clone());
        let unchanged = blocking(move || {
            if store::load_proposal(&dir)?.as_ref() != Some(&expected) {
                return Ok(false);
            }
            store::save_proposal(&dir, &written).map_err(|e| e.to_string())?;
            Ok(true)
        })
        .await?;
        if !unchanged {
            return Err(changed(project));
        }
        self.note_proposal(project, Some(next));
        Ok(())
    }

    /// The proposal row edit's work registered and its `Verifying` row edit written,
    /// under one hold of `writes` (so no `Use this` stores the proposal in between).
    async fn start_row_edit(
        &self,
        project: &Path,
        expected: &ProposalRecord,
        next: &ProposalRecord,
    ) -> Result<(u64, CancellationToken), String> {
        let _writes = self.writes.lock().await;
        let Some((generation, token)) = self.register(project) else {
            return Err(self.busy(project).unwrap_or_else(|| changed(project)));
        };
        if let Err(error) = self.save_over(project, expected, next).await {
            self.unregister(project, generation);
            return Err(error);
        }
        Ok((generation, token))
    }

    /// Decision 15: a proposal row edit's check. It never changes the proposal's state,
    /// verifies the edited profile (not `record.proposed`), keeps its `[delivery]`, and
    /// calls no `after_ready`: the proposal still needs **Use this**.
    pub(super) async fn verify_row_edit(self: Arc<Self>, job: Job, edited: RepoProfile) {
        let (project, generation) = (job.pre.project.clone(), job.generation);
        let verified = match self.mark(&project).await {
            Ok(()) => self.run_verification(&job, &edited).await,
            Err(stop) => Err(stop),
        };
        let confined = self.verify_confined();
        let outcome = proposal_outcome(&job, edited, verified, (unix_now(), confined));
        self.unmark(&project).await;
        // The outcome and the end of the work under one hold of `writes`: whoever sees the
        // outcome there sees the work ended.
        let _writes = self.writes.lock().await;
        if let Some(record) = outcome.filter(|_| self.current(&project, generation)) {
            let (dir, written) = (self.repo_dir(&project), record.clone());
            let saved =
                blocking(move || store::save_proposal(&dir, &written).map_err(|e| e.to_string()))
                    .await;
            match saved {
                Ok(()) => self.note_proposal(&project, Some(&record)),
                Err(error) => tracing::warn!(%error, "could not save a row edit's outcome"),
            }
        }
        self.unregister(&project, generation);
    }

    /// Decision 19: a `Failed` row edit reverted. On the stored profile its `Edit`-origin
    /// proposal is deleted; on a proposal the row edit is cleared.
    pub(super) async fn revert_edit(&self, dir: PathBuf) -> Result<ProfileReply, String> {
        let project = self.project_of(dir).await?;
        // Asked before `writes` is taken: the callback runs with no lock of ours held.
        let live = !self.live_runs(&project).is_empty();
        let _writes = self.writes.lock().await;
        let repo_dir = self.repo_dir(&project);
        let loaded = blocking(move || store::load_proposal(&repo_dir)).await?;
        let Some(record) = loaded else {
            return Err(no_failed_edit(&project));
        };
        let key = match &record.edit {
            Some(RowEdit {
                key,
                state: RowEditState::Failed { .. },
                ..
            }) => key.clone(),
            Some(edit) => return Err(still_checking(&edit.key)),
            None => return Err(no_failed_edit(&project)),
        };
        if is_review(&record) {
            let mut next = record.clone();
            next.edit = None;
            next.updated_at = unix_now();
            self.save_over(&project, &record, &next).await?;
            return done(format!("reverted {key}; the proposal is unchanged"));
        }
        // Decision 20: the stored profile's edit waits for its live runs.
        if live {
            return Err(LIVE_RUN.to_string());
        }
        let repo_dir = self.repo_dir(&project);
        blocking(move || store::delete_proposal(&repo_dir).map_err(|e| e.to_string())).await?;
        self.note_proposal(&project, None);
        done(format!("reverted {key}; the profile is unchanged"))
    }
}
