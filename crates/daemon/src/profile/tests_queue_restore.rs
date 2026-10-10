//! Milestone 9.10 final review fixes (daemon): what restore does with a proposal or a
//! queue file it cannot read (final review M2, task 4 re-review minors 1 and 2), and a
//! `reject` retried after a late failure (task 4 re-review minor 3). A real service on
//! a scratch data directory and real git repositories; no agent runs.

use std::path::Path;

use proto::{ProfileReply, ProfileRequest, SetupState};

use super::queue::{self, QUEUE_FILE};
use super::service_restore::UNREADABLE_QUEUE;
use super::store::{self, PROPOSAL_FILE};
use super::tests_queue::goal;
use super::tests_queue_service::{done, status, write_queue};
use super::tests_ready::{Rig, repo, service};

/// The entries of `dir` whose name starts with `prefix`.
fn named(dir: &Path, prefix: &str) -> Vec<std::path::PathBuf> {
    std::fs::read_dir(dir)
        .unwrap()
        .filter_map(Result::ok)
        .filter(|entry| entry.file_name().to_string_lossy().starts_with(prefix))
        .map(|entry| entry.path())
        .collect()
}

/// M2: goals queued beside a `proposal.json` that does not parse. Restore keeps the
/// file aside before it gives the goals a failed set-up they can retry; it never
/// writes over a proposal it could not read.
#[tokio::test(flavor = "multi_thread")]
async fn restore_sets_an_unreadable_proposal_aside() {
    let rig = Rig::new();
    let project = repo(rig.dir.path(), "app");
    write_queue(&rig, &project, vec![goal(&project, "add a flag", false)]);
    let repo_dir = rig.repo_dir(&project);
    std::fs::write(repo_dir.join(PROPOSAL_FILE), "{ not json").unwrap();

    let fresh = service(rig.dir.path());
    fresh.restore().await;
    let aside = named(&repo_dir, "proposal.json.unreadable-");
    assert_eq!(aside.len(), 1, "the proposal is kept aside once");
    assert_eq!(std::fs::read_to_string(&aside[0]).unwrap(), "{ not json");
    let queued = fresh.queued_goals();
    assert_eq!(queued.len(), 1, "{queued:?}");
    let SetupState::Failed { reason } = &queued[0].setup else {
        panic!("{:?}", queued[0].setup);
    };
    assert!(
        reason.starts_with("the proposal could not be read (")
            && reason.contains("proposal.json.unreadable-"),
        "{reason}"
    );
}

/// Task 4 re-review minor 1: a queue file that cannot be read (here a directory where
/// the file goes), as opposed to one that does not parse, stays where it is, and no
/// write replaces it.
#[tokio::test(flavor = "multi_thread")]
async fn a_queue_file_that_cannot_be_read_stays_in_place() {
    let rig = Rig::new();
    let project = repo(rig.dir.path(), "app");
    rig.store_profile(&project);
    let repo_dir = rig.repo_dir(&project);
    std::fs::create_dir_all(repo_dir.join(QUEUE_FILE).join("inside")).unwrap();

    let fresh = service(rig.dir.path());
    fresh.restore().await;
    assert!(named(&repo_dir, "queued_goals.json.unreadable-").is_empty());
    assert!(repo_dir.join(QUEUE_FILE).join("inside").is_dir());
    assert!(fresh.queued_for(&project).1.is_empty());
}

/// Task 4 re-review minor 2: an unreadable queue in a repository with no proposal and
/// no profile still shows its loss in `profile status`, and a goal queued later keeps
/// that record rather than writing over it.
#[tokio::test(flavor = "multi_thread")]
async fn an_unreadable_queue_with_nothing_else_shows_in_status() {
    let rig = Rig::new();
    let project = repo(rig.dir.path(), "app");
    let repo_dir = rig.repo_dir(&project);
    std::fs::create_dir_all(&repo_dir).unwrap();
    std::fs::write(repo_dir.join(QUEUE_FILE), "not json").unwrap();

    let fresh = service(rig.dir.path());
    fresh.restore().await;
    let shown = status(&fresh, &project).await;
    let goals: Vec<_> = shown
        .dropped_goals
        .iter()
        .map(|d| d.goal.as_str())
        .collect();
    assert_eq!(goals, [UNREADABLE_QUEUE]);

    fresh
        .queue_goal(goal(&project, "add a flag", false))
        .await
        .unwrap()
        .expect("queued");
    let on_file = queue::load(&repo_dir).unwrap();
    assert_eq!(on_file.goals.len(), 1);
    assert_eq!(on_file.dropped.len(), 1, "{on_file:?}");
    assert_eq!(on_file.dropped[0].goal, UNREADABLE_QUEUE);
}

/// Task 4 re-review minor 3: a `reject` that failed after deleting the proposal (a
/// checkout it could not discard) left the detection marker; the retry finishes the
/// cleanup and says so, not `no proposal`.
#[tokio::test(flavor = "multi_thread")]
async fn a_retried_reject_finishes_the_cleanup() {
    let rig = Rig::new();
    let project = repo(rig.dir.path(), "app");
    let repo_dir = rig.repo_dir(&project);
    std::fs::create_dir_all(&repo_dir).unwrap();
    store::save_detection(&repo_dir, &project).unwrap();

    let reply = rig
        .profiles
        .request(ProfileRequest::Reject {
            dir: project.clone(),
        })
        .await;
    assert!(
        matches!(&reply, ProfileReply::Done { .. }),
        "the retry was refused: {reply:?}"
    );
    let message = done(reply);
    assert!(
        message.starts_with("rejected the proposal for "),
        "{message}"
    );
    assert!(store::load_detection(&repo_dir).is_none());
}
