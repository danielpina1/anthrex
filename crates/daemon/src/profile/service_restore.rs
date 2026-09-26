//! Decisions 7 and 11 at daemon start, and decision 7's automatic re-detection shared
//! by `profile status`, `run start` and the start: `ProfileService::restore` and
//! `ProfileService::auto_on_stale`.
//!
//! No lock is held across an await here: the table is only read in one expression.

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use proto::{AgentRole, ProposalOrigin, ProposalState};

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

/// The git common directory a standalone checkout's repository borrows its objects
/// from (`<repo>/git/objects/info/alternates`' first line, less `/objects`): where git
/// for a leftover whose project is not recorded runs.
fn borrowed_common_dir(repo: &Path) -> Option<PathBuf> {
    let text = std::fs::read_to_string(repo.join("git/objects/info/alternates")).ok()?;
    let objects = PathBuf::from(text.lines().find(|l| !l.trim().is_empty())?.trim());
    objects.parent().map(Path::to_path_buf)
}

/// What the start found in one repository's data directory.
struct Found {
    name: OsString,
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
    ///    directory salvaged and removed, with or without a proposal (review m1);
    /// 4. decision 7: every stored profile that records its project checked for
    ///    staleness, and an automatic re-detection started in the background where
    ///    [`ProfileService::auto_on_stale`] allows it.
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
        for (project, meta) in found.into_iter().filter_map(|f| f.stored) {
            let service = self.clone();
            tokio::spawn(async move { service.auto_at_start(project, meta).await });
        }
    }

    /// Step 2 for one repository data directory, and what it records.
    async fn restore_repo(&self, repo_dir: &Path, name: OsString) -> Found {
        let dir = repo_dir.to_path_buf();
        let loaded = blocking(move || {
            store::sweep_leftovers(&dir).map_err(|e| e.to_string())?;
            let proposal = store::load_proposal(&dir).ok().flatten();
            Ok((proposal, store::load(&dir)))
        })
        .await;
        let (proposal, stored) = match loaded {
            Ok(loaded) => loaded,
            Err(error) => {
                tracing::warn!(repo_dir = %repo_dir.display(), %error, "profile restore");
                (None, Stored::Absent)
            }
        };
        let stored = match stored {
            Stored::Found { meta, .. } => meta.project.clone().map(|project| (project, meta)),
            _ => None,
        };
        let mut project = stored.as_ref().map(|(project, _)| project.clone());
        if let Some(record) = proposal {
            project = Some(record.project.clone());
            if in_progress(&record.state) {
                let mut failed = record;
                failed.state = ProposalState::Failed {
                    reason: RESTART_REASON.to_string(),
                };
                failed.updated_at = unix_now();
                let dir = repo_dir.to_path_buf();
                let saved = blocking(move || {
                    store::save_proposal(&dir, &failed).map_err(|e| e.to_string())
                })
                .await;
                if let Err(error) = saved {
                    tracing::warn!(repo_dir = %repo_dir.display(), %error, "profile restore");
                }
            }
        }
        Found {
            name,
            project,
            stored,
        }
    }

    /// Step 3 for one repository: each detection checkout present is discarded, its
    /// git run in the recorded project, else in the common directory its repository
    /// borrows from. One whose repository has neither is left, and logged.
    async fn discard_leftovers(&self, found: &Found) {
        let wt = self.ctx.worktrees_root.join(&found.name).join("runs");
        let repo_dir = self.ctx.data_dir.join("repos").join(&found.name);
        for name in CHECKOUTS {
            let path = wt.join(name);
            let repo = checkout_repo_dir(&repo_dir, &path);
            let (p, r) = (path.clone(), repo.clone());
            let probed = blocking(move || {
                let present = std::fs::symlink_metadata(&p).is_ok() || r.exists();
                let has_repo = r.join("git/HEAD").is_file();
                Ok((present, has_repo, borrowed_common_dir(&r)))
            })
            .await;
            let Ok((true, has_repo, borrowed)) = probed else {
                continue;
            };
            // Without a repository `HEAD` no git runs in it (`verify::discard`), so any
            // root does; with one, git needs the user's repository.
            let root = match (&found.project, borrowed) {
                (Some(project), _) => project.clone(),
                (None, Some(common)) => common,
                (None, None) if !has_repo => wt.clone(),
                (None, None) => {
                    tracing::warn!(path = %path.display(), "a leftover detection checkout whose repository is unknown is kept");
                    continue;
                }
            };
            if let Err(error) = self.discard_at(&root, path.clone(), repo).await {
                tracing::warn!(path = %path.display(), %error, "leftover detection checkout");
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

#[cfg(test)]
mod tests {
    use super::borrowed_common_dir;

    #[test]
    fn a_leftovers_common_dir_is_read_from_its_alternates() {
        let dir = tempfile::tempdir().unwrap();
        let info = dir.path().join("git/objects/info");
        std::fs::create_dir_all(&info).unwrap();
        std::fs::write(info.join("alternates"), "\n/work/app/.git/objects\n").unwrap();
        assert_eq!(
            borrowed_common_dir(dir.path()),
            Some("/work/app/.git".into())
        );
        assert_eq!(borrowed_common_dir(&dir.path().join("none")), None);
    }
}
