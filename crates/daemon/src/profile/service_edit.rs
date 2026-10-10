//! `anthrex profile edit` (milestone 9.10 decisions 15 to 21): a row edit of the stored
//! profile or of a review proposal, checked once, then saved, saved anyway or reverted.

use std::path::PathBuf;
use std::sync::Arc;

use proto::{ProfileReply, ProposalOrigin, ProposalRecord, ProposalState};

use super::proposal::apply_edit;
use super::service::{ProfileService, already_running, blocking};
use super::service_requests::{live_run, no_stored, stopping, unparseable};
use super::service_run::{Job, confirm_record};
use super::store::{self, Stored};
use crate::run::driver::unix_now;

impl ProfileService {
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
