//! Milestone 9.10 decisions 12 and 13 in the driver: a continued goal with no stored
//! profile keeps its refusal, and a goal with no profile is queued and starts the
//! set-up whatever `[orchestrator.onboarding] auto` says. Real git repositories, a
//! real profile service wired as the daemon wires it; the agents' binaries are paths
//! that do not exist, so no agent runs.

use std::path::Path;
use std::time::{Duration, Instant};

use proto::run_wire::request;
use proto::{ProposalOrigin, ProposalState, RunReply};
use tokio_util::sync::CancellationToken;

use super::super::CONTINUE_NEEDS_PROFILE;
use crate::launch::LaunchGate;
use crate::manager::{ManagerConfig, WindowManager};
use crate::profile::service::Effective;
use crate::run::driver::delivery::tests::git;
use crate::run::driver::orch::chain_goal::tests::{ANSWER, ChainRig, PREV};
use crate::run::driver::{RunContext, RunService};
use crate::server::GitWiring;

/// Decision 13: `--continue` in a repository whose profile went away is refused with
/// exactly `CONTINUE_NEEDS_PROFILE`; nothing is queued and no detection starts.
#[tokio::test(flavor = "multi_thread")]
async fn a_continued_goal_with_no_profile_is_refused() {
    let rig = ChainRig::new(|_| {}).await;
    let work = rig.checkout.work.clone();
    let pre = crate::run::git::preflight("git".as_ref(), &work, ANSWER).unwrap();
    let data = rig.checkout.tmp.path().join("data");
    let repo_dir = crate::profile::repo_dir(&data, &pre.project);
    std::fs::remove_file(repo_dir.join(crate::profile::store::PROFILE_FILE)).unwrap();
    std::fs::remove_file(repo_dir.join(crate::profile::store::META_FILE)).unwrap();
    let reply = rig.continued(PREV, &work).await;
    assert_eq!(
        reply,
        RunReply::refused(request::START_GOAL, CONTINUE_NEEDS_PROFILE.to_string())
    );
    assert!(!repo_dir.join("proposal.json").exists());
    assert!(!repo_dir.join(crate::profile::queue::QUEUE_FILE).exists());
    assert!(rig.new_runs().is_empty());
    rig.stop().await;
}

/// A committed repository at `<dir>/app`.
fn repo(dir: &Path) -> std::path::PathBuf {
    let root = dir.join("app");
    std::fs::create_dir_all(root.join("src")).unwrap();
    std::fs::write(root.join("src/lib.rs"), "// lib\n").unwrap();
    git(&root, &["init", "-q", "-b", "main"]);
    git(&root, &["config", "user.name", "t"]);
    git(&root, &["config", "user.email", "t@t"]);
    git(&root, &["add", "-A"]);
    git(&root, &["commit", "-q", "-m", "base"]);
    root
}

/// The service's `git_timeout_secs`.
const GIT_TIMEOUT_SECS: u64 = 5;

/// How long the set-up with no agent binary takes to fail (`docs/timing-budgets.md`,
/// M9.10.5): the onboarding checkout's prepare and discard (4 git calls at
/// `GIT_TIMEOUT_SECS`, 20 s) and the scout's failed spawn; under a second here.
const SETTLE: Duration = Duration::from_secs(120);

/// Decision 12 (R): with `[orchestrator.onboarding] auto = false` a goal with no
/// profile is still queued, and its explicit set-up starts.
#[tokio::test(flavor = "multi_thread")]
async fn onboarding_auto_off_still_sets_up_a_goal() {
    let dir = tempfile::tempdir().unwrap();
    let root = repo(dir.path());
    let data = dir.path().join("data");
    let socket = dir.path().join("d.sock");
    let mut manager = ManagerConfig::for_tests(socket.clone(), "/bin/sh".into());
    manager.claude_bin = "/nonexistent/anthrex-test/claude".into();
    manager.codex_bin = "/nonexistent/anthrex-test/codex".into();
    manager.decider_bin = Some("/nonexistent/anthrex-test/decider".into());
    manager.worktrees_root = dir.path().join("worktrees");
    manager.launch_gate = LaunchGate::open_already();
    let (manager, _events) = WindowManager::new(manager);
    let wiring = GitWiring::new(config::Git {
        enabled: false,
        ..config::Git::default()
    });
    let mut orchestrator = config::Orchestrator::default();
    orchestrator.onboarding.auto = false;
    orchestrator.git_timeout_secs = GIT_TIMEOUT_SECS;
    let ctx = RunContext::new(
        data.clone(),
        manager.config(),
        orchestrator.clone(),
        wiring.registry.clone(),
    );
    let runs = RunService::new(manager.clone(), ctx);
    let profiles = crate::profile::service::wire(&manager, &runs, &data, &socket, &orchestrator);
    assert!(!profiles.onboarding_auto());
    let shutdown = CancellationToken::new();
    runs.spawn(shutdown.clone());

    let reply = runs
        .start_goal(
            "add a".into(),
            root.clone(),
            (false, true),
            false,
            (None, None, None),
        )
        .await;
    let RunReply::Queued {
        goal_id, project, ..
    } = reply
    else {
        panic!("not queued: {reply:?}");
    };
    let queued = profiles.queued_goals();
    assert_eq!(queued.len(), 1, "{queued:?}");
    assert_eq!((&queued[0].id, &queued[0].project), (&goal_id, &project));
    let proposal = match profiles.effective(&project).await {
        Effective::Absent { proposal } => proposal,
        _ => panic!("a profile was stored"),
    };
    assert!(proposal.is_some(), "the set-up has started");
    let record = crate::profile::store::load_proposal(&crate::profile::repo_dir(&data, &project))
        .unwrap()
        .expect("a proposal");
    assert_eq!(record.origin, ProposalOrigin::Goal);

    // The set-up fails (there is no agent), and its goal keeps waiting (decision 7).
    let deadline = Instant::now() + SETTLE;
    loop {
        if let Effective::Absent {
            proposal: Some(ProposalState::Failed { .. }),
        } = profiles.effective(&project).await
        {
            break;
        }
        assert!(Instant::now() < deadline, "the set-up did not settle");
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    assert_eq!(profiles.queued_goals().len(), 1);
    shutdown.cancel();
    runs.stop().await;
}
