//! `ProfileService` (milestone 8b decisions 8, 10 and 11): detection, verification,
//! confirmation and correction of each repository's profile, one proposal per
//! repository in `<repo_dir>/proposal.json`.
//!
//! - `service.rs`: the service, its table, `start_detection`, `restore` and `effective`.
//! - `service_run.rs`: the background work of one proposal: the onboarding checkout,
//!   the scout, verification and the automatic confirmation of `edit --yes`.
//! - `service_requests.rs`: `anthrex profile status|detect|show|confirm|reject|edit`.
//!
//! **Locks.** The table (`crate::lock(&self.table)`) is held only to read or change it;
//! every guard ends in its own block, before any `.await`, manager call, scout-service
//! call, git command or file access. Files and git run on `spawn_blocking` or behind
//! `GitQueue::write` (AGENTS.md rules 2 and 10). `writes`, a tokio mutex (not
//! `daemon::lock`), orders every `proposal.json` change of a proposal's own work
//! against `profile reject`, so nothing a stopped detection still does can bring back a
//! proposal the user rejected.
//!
//! **Nothing is written under a repository root**: the proposal, the profile and the
//! scout's report are in `<data>/repos/<repo>/`, the two disposable checkouts under
//! `<data>/worktrees/<repo>/runs/`, and the only writes into the repository's `.git`
//! are M8a's salvage refs.

use std::collections::HashMap;
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use proto::{
    ProfileMeta, ProfileReply, ProfileRequest, ProposalRecord, ProposalState, RepoProfile,
};
use tokio_util::sync::CancellationToken;

use super::store::{self, Stored};
use crate::headless::argv::CliCaps;
use crate::manager::WindowManager;
use crate::run::confine;
use crate::run::git::GitQueue;
use crate::scout::service::ScoutService;

/// Decision 11: the reason a detection the daemon's restart interrupted failed.
pub const RESTART_REASON: &str =
    "the daemon restarted during detection; run anthrex profile detect";

/// Decision 7: an automatic re-detection does not start within this long of a failed
/// one.
pub const AUTO_RETRY_AFTER_SECS: u64 = 3600;

/// What the service needs from the daemon (Interfaces).
pub struct ProfileContext {
    pub data_dir: PathBuf,
    pub worktrees_root: PathBuf,
    pub git: OsString,
    pub orchestrator: config::Orchestrator,
    pub git_queue: Arc<GitQueue>,
    pub cli_caps: CliCaps,
    pub daemon_socket: PathBuf,
    /// The daemon's test scheduler: verification's commands take its slots (decision
    /// 23, ruling C-27 I-2).
    pub scheduler: Arc<crate::run::slots::TestScheduler>,
}

/// A repository's profile as a run or milestone 9 sees it.
#[derive(Debug, Clone, PartialEq, Eq)]
#[allow(clippy::large_enum_variant)]
pub enum Effective {
    Stored {
        profile: RepoProfile,
        meta: ProfileMeta,
        path: PathBuf,
        stale: Vec<String>,
    },
    Unparseable {
        path: PathBuf,
        error: String,
    },
    Absent {
        proposal: Option<ProposalState>,
    },
}

/// One proposal whose work runs now, by project.
pub(super) struct Active {
    pub(super) generation: u64,
    pub(super) token: CancellationToken,
    /// The scout this proposal started, or is about to start.
    pub(super) scout_id: Option<String>,
}

#[derive(Default)]
pub(super) struct Table {
    pub(super) active: HashMap<PathBuf, Active>,
    /// The number in the last onboarding scout id handed out (ruling R-T9-2).
    pub(super) last_scout_secs: u64,
    /// Stored profiles `restore` found, checked for staleness by `spawn` (re-review r1).
    pub(super) auto_at_start: Vec<(PathBuf, ProfileMeta)>,
}

/// See the module doc.
pub struct ProfileService {
    pub(super) scouts: Arc<ScoutService>,
    pub(super) manager: Arc<WindowManager>,
    pub(super) ctx: ProfileContext,
    pub(super) table: Mutex<Table>,
    pub(super) writes: tokio::sync::Mutex<()>,
    next_generation: AtomicU64,
}

/// A blocking step on `spawn_blocking`, its panic an error.
pub(super) async fn blocking<T: Send + 'static>(
    f: impl FnOnce() -> Result<T, String> + Send + 'static,
) -> Result<T, String> {
    tokio::task::spawn_blocking(f)
        .await
        .map_err(|error| format!("a blocking step did not finish: {error}"))?
}

/// `state`'s label in messages: `preparing`, `scouting`, `verifying`, `ready`, `failed`.
pub fn state_label(state: &ProposalState) -> &'static str {
    match state {
        ProposalState::Preparing => "preparing",
        ProposalState::Scouting => "scouting",
        ProposalState::Verifying => "verifying",
        ProposalState::Ready => "ready",
        ProposalState::Failed { .. } => "failed",
    }
}

/// Whether a proposal in `state` still has work running.
pub fn in_progress(state: &ProposalState) -> bool {
    matches!(
        state,
        ProposalState::Preparing | ProposalState::Scouting | ProposalState::Verifying
    )
}

/// Decision 8's refusal of a second detection (and of an edit) while one runs.
pub fn already_running(project: &Path, state: &ProposalState) -> String {
    format!(
        "detection is already running for {} (state {}); anthrex profile reject stops it",
        project.display(),
        state_label(state)
    )
}

/// Ruling R-T9-2: the number of the next onboarding scout id, `onboarding-<n>`: the
/// current unix second, or one more than the last number handed out, whichever is
/// larger, so two detections in one second never share an id.
pub fn next_scout_secs(last: u64, now: u64) -> u64 {
    now.max(last.saturating_add(1))
}

/// Decision 7: whether an automatic re-detection may start beside `proposal`: none is
/// pending, and none failed within the last hour.
pub fn auto_allowed(proposal: Option<&ProposalRecord>, now: u64) -> bool {
    match proposal {
        None => true,
        Some(record) => match record.state {
            ProposalState::Failed { .. } => {
                now.saturating_sub(record.updated_at) >= AUTO_RETRY_AFTER_SECS
            }
            _ => false,
        },
    }
}

impl ProfileService {
    pub fn new(
        scouts: Arc<ScoutService>,
        manager: Arc<WindowManager>,
        ctx: ProfileContext,
    ) -> Arc<Self> {
        Arc::new(ProfileService {
            scouts,
            manager,
            ctx,
            table: Mutex::new(Table::default()),
            writes: tokio::sync::Mutex::new(()),
            next_generation: AtomicU64::new(1),
        })
    }

    /// The scout service this one starts onboarding scouts with.
    pub fn scouts(&self) -> &Arc<ScoutService> {
        &self.scouts
    }

    pub(super) fn repo_dir(&self, project: &Path) -> PathBuf {
        super::repo_dir(&self.ctx.data_dir, project)
    }

    pub(super) fn git_timeout(&self) -> Duration {
        Duration::from_secs(self.ctx.orchestrator.git_timeout_secs)
    }

    /// Whether verification would run confined here (decision 9).
    pub(super) fn verify_confined(&self) -> bool {
        self.ctx.orchestrator.worker_sandbox && confine::available()
    }

    /// Decision 6's input for milestone 9: the stored profile and its staleness, or why
    /// there is none.
    pub async fn effective(&self, project: &Path) -> Effective {
        let dir = self.repo_dir(project);
        let project = project.to_path_buf();
        let loaded = blocking(move || {
            Ok(match store::load(&dir) {
                Stored::Found {
                    profile,
                    meta,
                    path,
                } => {
                    let stale = store::stale(&project, &meta);
                    Effective::Stored {
                        profile,
                        meta,
                        path,
                        stale,
                    }
                }
                Stored::Unparseable { path, error } => Effective::Unparseable { path, error },
                Stored::Absent => Effective::Absent {
                    proposal: store::load_proposal(&dir)
                        .ok()
                        .flatten()
                        .map(|record| record.state),
                },
            })
        })
        .await;
        loaded.unwrap_or(Effective::Absent { proposal: None })
    }

    /// Answers one `anthrex profile` request at once; detection and verification go on
    /// in the background (decision 10).
    pub async fn request(self: &Arc<Self>, request: ProfileRequest) -> ProfileReply {
        let answered = match request {
            ProfileRequest::Status { dir } => self.status(dir).await.map(ProfileReply::Status),
            ProfileRequest::Detect {
                dir,
                trust_project,
                unconfined_checks,
            } => self.detect(dir, trust_project, unconfined_checks).await,
            ProfileRequest::Show { dir, proposed } => self.show(dir, proposed).await,
            ProfileRequest::Confirm { dir, shown } => self.confirm(dir, shown).await,
            ProfileRequest::Reject { dir } => self.reject(dir).await,
            ProfileRequest::Edit {
                dir,
                key,
                value,
                yes,
                unconfined_checks,
            } => self.edit(dir, key, value, yes, unconfined_checks).await,
        };
        answered.unwrap_or_else(|message| ProfileReply::Refused { message })
    }

    /// The proposal in progress for `project`, from the table or from `proposal.json`.
    pub(super) async fn running(&self, project: &Path) -> Option<ProposalState> {
        let active = crate::lock(&self.table).active.contains_key(project);
        let dir = self.repo_dir(project);
        let state = blocking(move || store::load_proposal(&dir))
            .await
            .ok()
            .flatten()
            .map(|record| record.state);
        match state {
            Some(state) if active || in_progress(&state) => Some(state),
            None if active => Some(ProposalState::Preparing),
            _ => None,
        }
    }

    /// Registers new work for `project`; `None` when some already runs.
    pub(super) fn register(&self, project: &Path) -> Option<(u64, CancellationToken)> {
        let mut table = crate::lock(&self.table);
        if table.active.contains_key(project) {
            return None;
        }
        let generation = self.next_generation.fetch_add(1, Ordering::Relaxed);
        let token = CancellationToken::new();
        table.active.insert(
            project.to_path_buf(),
            Active {
                generation,
                token: token.clone(),
                scout_id: None,
            },
        );
        Some((generation, token))
    }

    /// Ends `generation`'s registration for `project`, if it is still the current one.
    pub(super) fn unregister(&self, project: &Path, generation: u64) {
        let mut table = crate::lock(&self.table);
        if table
            .active
            .get(project)
            .is_some_and(|active| active.generation == generation)
        {
            table.active.remove(project);
        }
    }

    /// Whether `generation` still owns `project`'s proposal (not rejected meanwhile).
    pub(super) fn current(&self, project: &Path, generation: u64) -> bool {
        crate::lock(&self.table)
            .active
            .get(project)
            .is_some_and(|active| active.generation == generation && !active.token.is_cancelled())
    }

    /// Saves `record` only while `generation` still owns its proposal, ordered against
    /// `reject` by `writes`. `false`: the proposal was rejected, nothing was written.
    pub(super) async fn save_if_current(&self, generation: u64, record: &ProposalRecord) -> bool {
        let _writes = self.writes.lock().await;
        if !self.current(&record.project, generation) {
            return false;
        }
        let (dir, record) = (self.repo_dir(&record.project), record.clone());
        match blocking(move || store::save_proposal(&dir, &record).map_err(|e| e.to_string())).await
        {
            Ok(()) => true,
            Err(error) => {
                tracing::warn!(%error, "could not save a profile proposal");
                true
            }
        }
    }
}

/// The daemon's milestone-8b services (decisions 8 and 12): the scout service and this
/// one, set on the engine once (`RunService::set_adaptation`). Called by `lifecycle`.
pub fn wire(
    manager: &Arc<WindowManager>,
    runs: &Arc<crate::run::driver::RunService>,
    data_dir: &Path,
    daemon_socket: &Path,
    orchestrator: &config::Orchestrator,
) -> Arc<ProfileService> {
    let scouts = ScoutService::new(
        manager.clone(),
        crate::scout::spec::ScoutContext {
            roster: orchestrator.models.clone(),
            default_runtime: orchestrator.default_runtime,
            scouts: orchestrator.scouts.clone(),
            claude: orchestrator.claude.clone(),
            caps: manager.config().cli_caps,
            data_dir: data_dir.to_path_buf(),
        },
    );
    let profiles = ProfileService::new(
        scouts.clone(),
        manager.clone(),
        ProfileContext {
            data_dir: data_dir.to_path_buf(),
            worktrees_root: manager.config().worktrees_root.clone(),
            git: OsString::from("git"),
            orchestrator: orchestrator.clone(),
            git_queue: runs.git_queue(),
            cli_caps: manager.config().cli_caps,
            daemon_socket: daemon_socket.to_path_buf(),
            scheduler: runs.scheduler().clone(),
        },
    );
    runs.set_adaptation(crate::run::driver::Adaptation {
        profiles: profiles.clone(),
        scouts,
        deciders: crate::decider::DeciderContext::new(orchestrator, manager.config(), data_dir),
    });
    profiles
}

#[cfg(test)]
mod tests {
    use proto::{ProposalOrigin, ProposalRecord, ProposalState};

    use super::{AUTO_RETRY_AFTER_SECS, auto_allowed, next_scout_secs};

    #[test]
    fn scout_ids_are_unique_within_a_second() {
        assert_eq!(next_scout_secs(0, 1_700_000_000), 1_700_000_000);
        let first = next_scout_secs(0, 1_700_000_000);
        let second = next_scout_secs(first, 1_700_000_000);
        assert_ne!(first, second);
        assert_eq!(next_scout_secs(second, 1_700_000_005), 1_700_000_005);
    }

    fn record(state: ProposalState, updated_at: u64) -> ProposalRecord {
        ProposalRecord {
            project: "/p".into(),
            state,
            origin: ProposalOrigin::Detect,
            started_at: 0,
            updated_at,
            base_sha: String::new(),
            scout_id: None,
            window_id: None,
            profile: None,
            verification: None,
            dropped: Vec::new(),
            proposed: None,
            trusted_project: Vec::new(),
            unconfined_checks: false,
            auto_confirm: false,
        }
    }

    #[test]
    fn auto_detection_waits_an_hour_after_a_failure_and_for_any_pending_proposal() {
        let now = 10_000;
        assert!(auto_allowed(None, now));
        let failed = |at| record(ProposalState::Failed { reason: "r".into() }, at);
        assert!(!auto_allowed(Some(&failed(now - 10)), now));
        assert!(auto_allowed(
            Some(&failed(now - AUTO_RETRY_AFTER_SECS)),
            now
        ));
        for state in [
            ProposalState::Preparing,
            ProposalState::Scouting,
            ProposalState::Verifying,
            ProposalState::Ready,
        ] {
            assert!(!auto_allowed(Some(&record(state, 0)), now));
        }
    }
}
