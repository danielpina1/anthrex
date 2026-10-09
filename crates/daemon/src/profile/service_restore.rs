//! Decisions 7 and 11 at daemon start, and decision 7's automatic re-detection shared
//! by `profile status`, `run start` and the start: `ProfileService::restore` and
//! `ProfileService::auto_on_stale`.
//!
//! No lock is held across an await here: the table is only read in one expression.

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use proto::{AgentRole, ProposalOrigin, ProposalState};

use super::queue::{self, GoalQueue};
use super::service::{ProfileService, RESTART_REASON, auto_allowed, blocking, in_progress};
use super::store::{self, Stored};
use crate::run::driver::unix_now;
use crate::run::git::checkout_repo_dir;
use crate::run::plan::Preflight;

/// The two detection checkouts, by name under `<wt>/runs/`.
const CHECKOUTS: [&str; 2] = [super::ONBOARDING_CHECKOUT, super::VERIFY_CHECKOUT];

/// The directory names under `dir` (empty when it is missing).
fn names_in(dir: &Path) -> Result<Vec<OsString>, String> {
    match std::fs::read_dir(dir) {
        Ok(entries) => Ok(entries
            .filter_map(Result::ok)
            .filter(|entry| entry.path().is_dir())
            .map(|entry| entry.file_name())
            .collect()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(Vec::new()),
        Err(error) => Err(error.to_string()),
    }
}

/// What the start found in one repository's data directory.
struct Found {
    name: OsString,
    /// The project a trusted record names (task 11 re-review, C1): the detection
    /// marker, `proposal.json` or the stored meta, each only when its `repo_dir` is
    /// this very directory. Never read from a checkout or its repository, which
    /// confined commands can write.
    project: Option<PathBuf>,
    /// The stored profile's project and meta, when it records its project.
    stored: Option<(PathBuf, proto::ProfileMeta)>,
}

impl ProfileService {
    /// Decision 11, after `RunService::restore` and before the socket binds:
    /// 1. every restored run-less scout window removed (ruling R-T9-1);
    /// 2. in every repository's data directory, leftover temp files swept (ruling
    ///    R-T4-1), then an interrupted proposal failed with [`RESTART_REASON`];
    /// 3. every detection checkout found under the worktrees root or the data
    ///    directory salvaged and removed when a trusted record names its project;
    ///    otherwise kept, with no git run, and logged (review m1, re-review C1);
    /// 4. decision 7: every stored profile that records its project queued for the
    ///    staleness check, which [`ProfileService::spawn`] starts once the scout
    ///    service listens (re-review r1).
    pub async fn restore(self: &Arc<Self>) {
        for window in self.manager.list() {
            let scout = window.run.is_none()
                && self
                    .manager
                    .headless_spec(window.id)
                    .and_then(|spec| spec.mcp)
                    .is_some_and(|target| target.role == AgentRole::Scout);
            if scout && let Err(error) = self.manager.remove(window.id) {
                tracing::warn!(window = window.id, %error, "could not remove a leftover scout window");
            }
        }
        let repos = self.ctx.data_dir.join("repos");
        let worktrees = self.ctx.worktrees_root.clone();
        let (r, w) = (repos.clone(), worktrees.clone());
        let names = blocking(move || {
            let mut names = names_in(&r)?;
            for name in names_in(&w)? {
                if !names.contains(&name) {
                    names.push(name);
                }
            }
            Ok(names)
        })
        .await;
        let names = match names {
            Ok(names) => names,
            Err(error) => {
                tracing::warn!(%error, "could not list the repositories' data directories");
                return;
            }
        };
        let mut found = Vec::new();
        for name in names {
            found.push(self.restore_repo(&repos.join(&name), name).await);
        }
        for repo in &found {
            self.discard_leftovers(repo).await;
        }
        if !self.ctx.orchestrator.onboarding.auto {
            return;
        }
        let queued: Vec<_> = found.into_iter().filter_map(|f| f.stored).collect();
        crate::lock(&self.table).auto_at_start = queued;
    }

    /// Starts the scout service's listener, then (re-review r1) the staleness checks
    /// `restore` queued, so an automatic scout's first events always have a listener.
    pub fn spawn(self: &Arc<Self>, shutdown: tokio_util::sync::CancellationToken) {
        self.scouts.spawn(shutdown);
        let queued = std::mem::take(&mut crate::lock(&self.table).auto_at_start);
        for (project, meta) in queued {
            let service = self.clone();
            tokio::spawn(async move { service.auto_at_start(project, meta).await });
        }
        // Milestone 9.10 decision 11: the queues `restore` loaded.
        let waiting: Vec<PathBuf> = crate::lock(&self.table).queued.keys().cloned().collect();
        for project in waiting {
            tokio::spawn(self.clone().drain_at_start(project));
        }
    }

    /// Step 2 for one repository data directory, and what it records.
    async fn restore_repo(&self, repo_dir: &Path, name: OsString) -> Found {
        let dir = repo_dir.to_path_buf();
        let loaded = blocking(move || {
            store::sweep_leftovers(&dir).map_err(|e| e.to_string())?;
            let proposal = store::load_proposal(&dir).ok().flatten();
            // Milestone 9.10 decision 11: the goals waiting for this profile.
            let queue = queue::load(&dir).unwrap_or_else(|error| {
                tracing::warn!(%error, "a goal queue that does not read is ignored");
                GoalQueue::default()
            });
            let loaded = (store::load(&dir), store::load_detection(&dir));
            Ok((proposal, loaded.0, loaded.1, queue))
        })
        .await;
        let (proposal, stored, marker, queue) = match loaded {
            Ok(loaded) => loaded,
            Err(error) => {
                tracing::warn!(repo_dir = %repo_dir.display(), %error, "profile restore");
                (None, Stored::Absent, None, GoalQueue::default())
            }
        };
        // A record counts only for the data directory its own project keys to, so one
        // repository's record cannot name another.
        let ours = |project: &PathBuf| {
            super::repo_dir(&self.ctx.data_dir, project).file_name() == Some(name.as_os_str())
        };
        let stored = match stored {
            Stored::Found { meta, .. } => meta
                .project
                .clone()
                .filter(|project| ours(project))
                .map(|project| (project, meta)),
            _ => None,
        };
        let mut project = stored.as_ref().map(|(project, _)| project.clone());
        if let Some(record) = proposal.filter(|record| ours(&record.project)) {
            project = Some(record.project.clone());
            // Decision 10 (M9.0.5): the ready list is rebuilt from disk, loaded off the
            // table's lock above; an interrupted proposal fails below, so is not ready.
            self.note_proposal(&record.project, Some(&record));
            if in_progress(&record.state) {
                let mut failed = record;
                failed.state = ProposalState::Failed {
                    reason: RESTART_REASON.to_string(),
                };
                failed.updated_at = unix_now();
                let (dir, written) = (repo_dir.to_path_buf(), failed.clone());
                let saved = blocking(move || {
                    store::save_proposal(&dir, &written).map_err(|e| e.to_string())
                })
                .await;
                match saved {
                    // Milestone 9.10 decision 10: memory says `Failed` too.
                    Ok(()) => self.note_proposal(&failed.project, Some(&failed)),
                    Err(error) => {
                        tracing::warn!(repo_dir = %repo_dir.display(), %error, "profile restore")
                    }
                }
            }
        }
        let queued = queue.goals.first().map(|goal| goal.project.clone());
        let queued = queued.filter(|project| ours(project)).or(project.clone());
        if let Some(queued) = queued.filter(|_| !queue.is_empty()) {
            self.remember_queue(&queued, queue);
        }
        Found {
            project: marker.filter(|project| ours(project)).or(project),
            name,
            stored,
        }
    }

    /// Step 3 for one repository: each detection checkout present is salvaged and
    /// removed, git run in the trusted project only. Without one no git runs: the
    /// leftover is kept (spec §17, nothing dirty is deleted) and logged with its path.
    /// The detection marker goes once no checkout is left.
    async fn discard_leftovers(&self, found: &Found) {
        let wt = self.ctx.worktrees_root.join(&found.name).join("runs");
        let repo_dir = self.ctx.data_dir.join("repos").join(&found.name);
        let mut left = false;
        for name in CHECKOUTS {
            let path = wt.join(name);
            let repo = checkout_repo_dir(&repo_dir, &path);
            let (p, r) = (path.clone(), repo.clone());
            let present = blocking(move || Ok(std::fs::symlink_metadata(&p).is_ok() || r.exists()))
                .await
                .unwrap_or(true);
            if !present {
                continue;
            }
            let Some(project) = &found.project else {
                tracing::warn!(
                    path = %path.display(),
                    repo = %repo.display(),
                    "a leftover detection checkout with no trusted project is kept; no git runs for it"
                );
                left = true;
                continue;
            };
            if let Err(error) = self.discard_at(project, path.clone(), repo).await {
                tracing::warn!(path = %path.display(), %error, "leftover detection checkout");
                left = true;
            }
        }
        if !left {
            let dir = repo_dir.clone();
            if let Err(error) =
                blocking(move || store::delete_detection(&dir).map_err(|e| e.to_string())).await
            {
                tracing::warn!(%error, "could not remove the detection marker");
            }
        }
    }

    /// Step 4 for one stored profile: its staleness, then [`Self::auto_on_stale`] after
    /// a preflight of its project (a dirty tree skips it, as at `profile status`).
    async fn auto_at_start(self: Arc<Self>, project: PathBuf, meta: proto::ProfileMeta) {
        let p = project.clone();
        let Ok(stale) = blocking(move || Ok(store::stale(&p, &meta))).await else {
            return;
        };
        if stale.is_empty() {
            return;
        }
        match self.preflight(project).await {
            Ok(pre) => self.auto_on_stale(&pre, stale).await,
            Err(error) => tracing::info!(%error, "no automatic re-detection at start"),
        }
    }

    /// Decision 7: a stale stored profile starts a re-detection when `onboarding.auto`
    /// is on, no work runs for the project, and `auto_allowed` holds. `pre` is a
    /// preflight that passed (a clean tree). Called from `profile status`, `run start`
    /// and the daemon's start.
    pub async fn auto_on_stale(self: &Arc<Self>, pre: &Preflight, stale: Vec<String>) {
        if !self.ctx.orchestrator.onboarding.auto || stale.is_empty() {
            return;
        }
        let busy = crate::lock(&self.table).active.contains_key(&pre.project);
        if busy {
            return;
        }
        let dir = self.repo_dir(&pre.project);
        let proposal = blocking(move || store::load_proposal(&dir))
            .await
            .ok()
            .flatten();
        if auto_allowed(proposal.as_ref(), unix_now()) {
            self.auto_detect(pre, ProposalOrigin::Auto { stale }).await;
        }
    }
}
