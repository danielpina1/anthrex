//! Ruling C-28 (2): a rejected edit's verification waiting for a scheduler slot stops
//! waiting. A real `ProfileService` (the daemon's own `wire`) over a real
//! `RunService` and its scheduler, a real repository and a stored profile; the test
//! holds every slot, so verification's first command waits. Rejecting it must end the
//! job promptly, withdraw its slot request, and let a new `profile edit` in.
//! Nothing here starts an agent: an edit's verification runs no scout.

use crate::support;

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
    dir: tempfile::TempDir,
    repo: PathBuf,
    profiles: Arc<ProfileService>,
    runs: Arc<RunService>,
    sched: Arc<TestScheduler>,
    shutdown: CancellationToken,
    stopped: bool,
    /// The verification checkout was seen to exist ([`Rig::saw_checkout`]), so `stop`'s
    /// wait for it to go cannot pass before a job has made it.
    checkout_seen: bool,
}

/// M9.12.13: once [`Rig::stop`] has stopped its services, the task temporary
/// directories of the checkouts made under the rig's directory go with it. A rig
/// dropped without `stop` fails the test (a failing test only cancels).
impl Drop for Rig {
    fn drop(&mut self) {
        self.shutdown.cancel();
        if std::thread::panicking() {
            return;
        }
        assert!(
            self.stopped,
            "a rig must be stopped with `Rig::stop().await`"
        );
        daemon::run::git::remove_task_tmps_beneath(self.dir.path()).unwrap();
    }
}

impl Rig {
    fn checkout(&self) -> PathBuf {
        daemon::profile::verify::checkout_path(&self.dir.path().join("worktrees"), &self.repo)
    }

    /// A verification job has made its checkout: `verify` prepares it before its first
    /// command asks for a slot, so it exists while a request waits.
    fn saw_checkout(&mut self) {
        let checkout = self.checkout();
        assert!(
            checkout.exists(),
            "a verification waits for a slot, but its checkout {} is not there",
            checkout.display()
        );
        self.checkout_seen = true;
    }

    /// M9.12.13 (fix round 1): every verification job has ended, which its own
    /// clean-up shows by removing the verification checkout, within [`LET_GO_WAIT`];
    /// then the run service stops (its last `run.json` written), within
    /// `RunService::stop`'s acknowledgement wait plus 10 s.
    async fn stop(mut self) {
        assert!(
            self.checkout_seen,
            "the rig never saw the verification checkout, so its going would prove nothing"
        );
        let checkout = self.checkout();
        let deadline = Instant::now() + LET_GO_WAIT;
        while checkout.exists() {
            assert!(
                Instant::now() < deadline,
                "a verification job never ended: {} is still there",
                checkout.display()
            );
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        self.shutdown.cancel();
        let bound = daemon::run::driver::STOP_WAIT + Duration::from_secs(10);
        tokio::time::timeout(bound, self.runs.stop())
            .await
            .unwrap_or_else(|_| panic!("the run service did not stop within {bound:?}"));
        self.stopped = true;
    }
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
        dir,
        repo,
        sched: runs.scheduler().clone(),
        profiles,
        runs,
        shutdown,
        stopped: false,
        checkout_seen: false,
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
    let mut rig = rig().await;
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
    rig.saw_checkout();
    // M9.12.13: the new edit is rejected too, while it still waits, so no verification
    // runs (and makes a checkout's temporary directory) once the slot is let go and the
    // test's directory is gone.
    let reply = rig
        .profiles
        .request(ProfileRequest::Reject {
            dir: rig.repo.clone(),
        })
        .await;
    assert!(message(&reply).is_some(), "{reply:?}");
    let deadline = Instant::now() + LET_GO_WAIT;
    while rig.sched.waiting() != 0 {
        assert!(Instant::now() < deadline, "the second reject never let go");
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    drop(held);
    rig.stop().await;
}
