//! Ruling C-28 (2): a rejected edit's verification waiting for a scheduler slot stops
//! waiting. A real `ProfileService` (the daemon's own `wire`) over a real
//! `RunService` and its scheduler, a real repository and a stored profile; the test
//! holds every slot, so verification's first command waits. Rejecting it must end the
//! job promptly, withdraw its slot request, and let a new `profile edit` in.
//! Nothing here starts an agent: an edit's verification runs no scout.

mod support;

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use daemon::manager::{ManagerConfig, WindowManager};
use daemon::profile::service::{ProfileService, wire};
use daemon::profile::store;
use daemon::run::driver::RunService;
use daemon::run::slots::{Priority, SlotRequest, TestScheduler, Want};
use daemon::server::GitWiring;
use proto::{ProfileMeta, ProfileReply, ProfileRequest, RepoProfile};
use tokio_util::sync::CancellationToken;

use support::run_harness::init_repo;
use support::tempdir;

/// Deadline for a rejected job to let go (one scheduler wake-up and the checkout's
/// discard, a few git commands; docs/timing-budgets.md, C-28).
const LET_GO_WAIT: Duration = Duration::from_secs(30);

struct Rig {
    _dir: tempfile::TempDir,
    repo: PathBuf,
    profiles: Arc<ProfileService>,
    sched: Arc<TestScheduler>,
    shutdown: CancellationToken,
}

async fn rig() -> Rig {
    let dir = tempdir();
    let repo = dir.path().join("repo");
    init_repo(&repo, &[("check.sh", "echo checking\n")]);
    let repo = repo.canonicalize().unwrap();
    let socket = dir.path().join("d.sock");
    let mut config = ManagerConfig::for_tests(socket.clone(), "/bin/sh".into());
    // No agent can start here.
    config.claude_bin = "/nonexistent/anthrex-test/claude".into();
    config.codex_bin = "/nonexistent/anthrex-test/codex".into();
    config.worktrees_root = dir.path().join("worktrees");
    let (manager, _events) = WindowManager::new(config);
    let data = dir.path().join("data");
    let git = GitWiring::new(config::Git {
        enabled: false,
        ..config::Git::default()
    });
    let runs = RunService::for_manager(&manager, data.clone(), git.registry.clone());
    let shutdown = CancellationToken::new();
    runs.spawn(shutdown.clone());
    let orchestrator = config::Orchestrator {
        worker_sandbox: false,
        ..config::Orchestrator::default()
    };
    let profiles = wire(&manager, &runs, &data, &socket, &orchestrator);
    let repo_dir = daemon::profile::repo_dir(&data, &repo);
    let profile = RepoProfile {
        check: Some("sh check.sh".into()),
        ..RepoProfile::default()
    };
    let meta = ProfileMeta {
        confirmed_at: 1,
        report: None,
        verification: None,
        fingerprint: Default::default(),
        edited_keys: Vec::new(),
        project: Some(repo.clone()),
    };
    store::save(&repo_dir, &profile, &meta).unwrap();
    Rig {
        _dir: dir,
        repo,
        sched: runs.scheduler().clone(),
        profiles,
        shutdown,
    }
}

fn edit(repo: &Path, value: &str) -> ProfileRequest {
    ProfileRequest::Edit {
        dir: repo.to_path_buf(),
        key: "check".into(),
        value: Some(value.into()),
        yes: false,
        unconfined_checks: true,
        anyway: false,
        on_proposal: false,
    }
}

fn message(reply: &ProfileReply) -> Option<&str> {
    match reply {
        ProfileReply::Done { message } => Some(message),
        _ => None,
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_rejected_verification_waiting_for_a_slot_lets_go() {
    let rig = rig().await;
    // A run's candidate holds every slot.
    let held = rig
        .sched
        .acquire(SlotRequest {
            priority: Priority::Candidate,
            critical: false,
            want: Want::All,
            exclusive: false,
            label: "candidate".into(),
        })
        .await;
    let reply = rig
        .profiles
        .request(edit(&rig.repo, "sh check.sh && true"))
        .await;
    assert!(message(&reply).is_some(), "{reply:?}");
    let deadline = Instant::now() + LET_GO_WAIT;
    while rig.sched.waiting() == 0 {
        assert!(
            Instant::now() < deadline,
            "verification never asked for a slot"
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }

    let reply = rig
        .profiles
        .request(ProfileRequest::Reject {
            dir: rig.repo.clone(),
        })
        .await;
    assert!(message(&reply).is_some(), "{reply:?}");

    // The job ends while the slot is still held: its request is withdrawn and a new
    // edit is accepted.
    let deadline = Instant::now() + LET_GO_WAIT;
    let accepted = loop {
        let reply = rig
            .profiles
            .request(edit(&rig.repo, "sh check.sh && true && true"))
            .await;
        if message(&reply).is_some() {
            break reply;
        }
        assert!(
            Instant::now() < deadline,
            "a new edit is still refused after the reject: {reply:?} (waiting {})",
            rig.sched.waiting()
        );
        tokio::time::sleep(Duration::from_millis(50)).await;
    };
    assert!(
        message(&accepted).is_some_and(|m| m.contains("verifying")),
        "{accepted:?}"
    );
    // Only the new edit's verification waits now (once it has asked).
    let deadline = Instant::now() + LET_GO_WAIT;
    while rig.sched.waiting() != 1 {
        assert!(rig.sched.waiting() < 2, "the rejected request still waits");
        assert!(
            Instant::now() < deadline,
            "the new edit never asked for a slot"
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    drop(held);
    rig.shutdown.cancel();
}
