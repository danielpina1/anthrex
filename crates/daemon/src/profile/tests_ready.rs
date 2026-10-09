//! Milestone 9.0.5 decision 10: the ready proposals `ProfileService` keeps in memory for
//! the Alerts box, kept in step with `proposal.json` across a confirm, a reject, a new
//! proposal and a daemon restart. A real service on a scratch data directory and a real
//! git repository; no agent runs (Claude and Codex are paths that do not exist).

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;
use std::time::Duration;

use proto::{
    ProfileMeta, ProfileReply, ProfileRequest, ProposalOrigin, ProposalRecord, ProposalState,
    RepoProfile,
};

use super::service::ProfileService;
use super::store;
use crate::launch::LaunchGate;
use crate::manager::{ManagerConfig, WindowManager};
use crate::run::driver::{RunContext, RunService};
use crate::server::GitWiring;

pub(super) struct Rig {
    pub(super) dir: tempfile::TempDir,
    data: PathBuf,
    pub(super) profiles: Arc<ProfileService>,
}

fn git(dir: &Path, args: &[&str]) {
    let output = Command::new("git")
        .args([
            "--no-optional-locks",
            "-c",
            "user.name=t",
            "-c",
            "user.email=t@t",
            "-c",
            "commit.gpgsign=false",
        ])
        .args(args)
        .current_dir(dir)
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env_remove("GIT_DIR")
        .env_remove("GIT_WORK_TREE")
        .env_remove("GIT_COMMON_DIR")
        .env_remove("GIT_INDEX_FILE")
        .env_remove("GIT_PREFIX")
        .output()
        .unwrap();
    assert!(output.status.success(), "git {args:?}: {output:?}");
}

/// A committed repository at `<dir>/<name>`; returns the project `project_of` sees.
pub(super) fn repo(dir: &Path, name: &str) -> PathBuf {
    let root = dir.join(name);
    std::fs::create_dir_all(root.join("src")).unwrap();
    std::fs::write(root.join("src/lib.rs"), "// lib\n").unwrap();
    git(&root, &["init", "-q", "-b", "main"]);
    // The repository's own identity: preflight needs one, and CI has no global one.
    git(&root, &["config", "user.name", "t"]);
    git(&root, &["config", "user.email", "t@t"]);
    git(&root, &["add", "-A"]);
    git(&root, &["commit", "-q", "-m", "base"]);
    crate::project::detect_roots_with("git".as_ref(), &root, Duration::from_secs(10)).project
}

/// A service wired as the daemon wires it, on `dir`'s data directory.
fn service(dir: &Path) -> Arc<ProfileService> {
    let socket = dir.join("d.sock");
    let data = dir.join("data");
    let mut config = ManagerConfig::for_tests(socket.clone(), "/bin/sh".into());
    config.claude_bin = "/nonexistent/anthrex-test/claude".into();
    config.codex_bin = "/nonexistent/anthrex-test/codex".into();
    config.worktrees_root = dir.join("worktrees");
    config.launch_gate = LaunchGate::open_already();
    let (manager, _events) = WindowManager::new(config);
    let git = GitWiring::new(config::Git {
        enabled: false,
        ..config::Git::default()
    });
    let orchestrator = config::Orchestrator::default();
    let ctx = RunContext::new(
        data.clone(),
        manager.config(),
        orchestrator.clone(),
        git.registry.clone(),
    );
    let runs = RunService::new(manager.clone(), ctx);
    crate::profile::service::wire(&manager, &runs, &data, &socket, &orchestrator)
}

impl Rig {
    pub(super) fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let data = dir.path().join("data");
        let profiles = service(dir.path());
        Rig {
            dir,
            data,
            profiles,
        }
    }

    fn repo_dir(&self, project: &Path) -> PathBuf {
        super::repo_dir(&self.data, project)
    }

    /// A stored profile for `project`, so `profile edit` has one to change.
    pub(super) fn store_profile(&self, project: &Path) {
        let dir = self.repo_dir(project);
        std::fs::create_dir_all(&dir).unwrap();
        let meta = ProfileMeta {
            confirmed_at: 1,
            report: None,
            verification: None,
            fingerprint: BTreeMap::new(),
            edited_keys: Vec::new(),
            project: Some(project.to_path_buf()),
        };
        let profile = RepoProfile {
            check: Some("true".into()),
            ..Default::default()
        };
        store::save(&dir, &profile, &meta).unwrap();
    }

    /// `profile edit modules ["src/*"]`: a key that needs no verification, so the
    /// proposal is written `Ready` at once.
    pub(super) async fn edit(&self, project: &Path) {
        let reply = self
            .profiles
            .request(ProfileRequest::Edit {
                dir: project.to_path_buf(),
                key: "modules".into(),
                value: Some("[\"src/*\"]".into()),
                yes: false,
                unconfined_checks: false,
                anyway: false,
                on_proposal: false,
            })
            .await;
        assert!(matches!(reply, ProfileReply::Done { .. }), "{reply:?}");
    }

    fn ready(&self) -> (u64, Vec<(PathBuf, u64)>) {
        let (generation, list) = self.profiles.ready_proposals();
        (
            generation,
            list.into_iter()
                .map(|p| (p.project, p.updated_at))
                .collect(),
        )
    }

    pub(super) fn on_disk(&self, project: &Path) -> Option<ProposalRecord> {
        store::load_proposal(&self.repo_dir(project)).unwrap()
    }
}

fn record(project: &Path, state: ProposalState, updated_at: u64) -> ProposalRecord {
    ProposalRecord {
        project: project.to_path_buf(),
        state,
        origin: ProposalOrigin::Detect,
        started_at: updated_at,
        updated_at,
        base_sha: String::new(),
        scout_id: None,
        window_id: None,
        profile: Some(RepoProfile::default()),
        verification: None,
        dropped: Vec::new(),
        proposed: None,
        trusted_project: Vec::new(),
        unconfined_checks: false,
        auto_confirm: false,
        edit: None,
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn a_ready_proposal_is_listed_and_a_confirm_removes_it() {
    let rig = Rig::new();
    let project = repo(rig.dir.path(), "app");
    rig.store_profile(&project);
    let (before, listed) = rig.ready();
    assert!(listed.is_empty());

    rig.edit(&project).await;
    let written = rig.on_disk(&project).expect("the edit wrote a proposal");
    assert_eq!(written.state, ProposalState::Ready);
    let (listed_at, listed) = rig.ready();
    assert_eq!(listed, vec![(project.clone(), written.updated_at)]);
    assert_ne!(
        listed_at, before,
        "a new ready proposal moves the generation"
    );

    let reply = rig
        .profiles
        .request(ProfileRequest::Confirm {
            dir: project.clone(),
            shown: None,
        })
        .await;
    assert!(matches!(reply, ProfileReply::Done { .. }), "{reply:?}");
    assert!(rig.on_disk(&project).is_none());
    let (confirmed_at, listed) = rig.ready();
    assert!(listed.is_empty(), "{listed:?}");
    assert_ne!(confirmed_at, listed_at);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_rejected_proposal_is_removed() {
    let rig = Rig::new();
    let project = repo(rig.dir.path(), "app");
    rig.store_profile(&project);
    rig.edit(&project).await;
    let (listed_at, listed) = rig.ready();
    assert_eq!(listed.len(), 1);

    let reply = rig
        .profiles
        .request(ProfileRequest::Reject {
            dir: project.clone(),
        })
        .await;
    assert!(matches!(reply, ProfileReply::Done { .. }), "{reply:?}");
    assert!(rig.on_disk(&project).is_none());
    let (rejected_at, listed) = rig.ready();
    assert!(listed.is_empty(), "{listed:?}");
    assert_ne!(rejected_at, listed_at);
}

/// A new proposal for the same project replaces the ready one (a detection starting
/// writes `Preparing`, a refused automatic one `Failed`): it leaves the list. Through
/// the recorder every successful write calls, since starting a detection needs a scout.
#[tokio::test(flavor = "multi_thread")]
async fn a_new_proposal_replaces_a_ready_one() {
    let rig = Rig::new();
    let project = repo(rig.dir.path(), "app");
    rig.store_profile(&project);
    rig.edit(&project).await;
    let (listed_at, listed) = rig.ready();
    assert_eq!(listed.len(), 1);
    // A later ready proposal is listed with its own time.
    let later = record(&project, ProposalState::Ready, listed[0].1 + 7);
    rig.profiles.note_proposal(&project, Some(&later));
    let (later_at, listed) = rig.ready();
    assert_eq!(listed, vec![(project.clone(), later.updated_at)]);
    assert_ne!(later_at, listed_at);
    // The same record again changes nothing, so nothing is published for it.
    rig.profiles.note_proposal(&project, Some(&later));
    assert_eq!(rig.ready().0, later_at);
    let preparing = record(&project, ProposalState::Preparing, later.updated_at + 1);
    rig.profiles.note_proposal(&project, Some(&preparing));
    let (replaced_at, listed) = rig.ready();
    assert!(listed.is_empty(), "{listed:?}");
    assert_ne!(replaced_at, later_at);
}

/// A daemon restart rebuilds the list from disk: a `Ready` proposal is listed with its
/// `updated_at`; one a detection left in progress is failed by `restore`, so it is not.
#[tokio::test(flavor = "multi_thread")]
async fn restore_seeds_ready_proposals() {
    let dir = tempfile::tempdir().unwrap();
    let data = dir.path().join("data");
    let (ready, running) = (
        PathBuf::from("/work/ready-app"),
        PathBuf::from("/work/running-app"),
    );
    for (project, state) in [
        (&ready, ProposalState::Ready),
        (&running, ProposalState::Scouting),
    ] {
        let repo_dir = super::repo_dir(&data, project);
        std::fs::create_dir_all(&repo_dir).unwrap();
        store::save_proposal(&repo_dir, &record(project, state, 1_790_000_000)).unwrap();
    }
    let profiles = service(dir.path());
    assert!(profiles.ready_proposals().1.is_empty());
    profiles.restore().await;
    let (generation, listed) = profiles.ready_proposals();
    let listed: Vec<_> = listed
        .into_iter()
        .map(|p| (p.project, p.updated_at))
        .collect();
    assert_eq!(listed, vec![(ready, 1_790_000_000)]);
    assert_ne!(generation, 0);
}

/// `save_if_current`, the path every write of a proposal's own work takes: a `Ready`
/// write is listed, a `Failed` one over it leaves the list, and a write from a stale
/// generation is neither written nor listed.
#[tokio::test(flavor = "multi_thread")]
async fn save_if_current_lists_only_a_current_ready_write() {
    let rig = Rig::new();
    let project = repo(rig.dir.path(), "app");
    rig.store_profile(&project);
    let (generation, _token) = rig.profiles.register(&project).expect("nothing runs yet");

    let ready = record(&project, ProposalState::Ready, 1_790_000_000);
    assert!(rig.profiles.save_if_current(generation, &ready).await);
    assert_eq!(
        rig.on_disk(&project).map(|r| r.state),
        Some(ProposalState::Ready)
    );
    let (listed_at, listed) = rig.ready();
    assert_eq!(listed, vec![(project.clone(), ready.updated_at)]);

    let failed_state = ProposalState::Failed {
        reason: "the check failed".into(),
    };
    let failed = record(&project, failed_state.clone(), ready.updated_at + 1);
    assert!(rig.profiles.save_if_current(generation, &failed).await);
    assert_eq!(
        rig.on_disk(&project).map(|r| r.state),
        Some(failed_state.clone())
    );
    let (failed_at, listed) = rig.ready();
    assert!(listed.is_empty(), "{listed:?}");
    assert_ne!(failed_at, listed_at);

    rig.profiles.unregister(&project, generation);
    let (newer, _token) = rig.profiles.register(&project).expect("the old work ended");
    assert_ne!(newer, generation);
    let stale = record(&project, ProposalState::Ready, failed.updated_at + 1);
    assert!(!rig.profiles.save_if_current(generation, &stale).await);
    assert_eq!(
        rig.on_disk(&project).map(|r| r.state),
        Some(failed_state.clone())
    );
    assert_eq!(rig.ready(), (failed_at, Vec::new()));
}
