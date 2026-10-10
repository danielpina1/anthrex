//! M9.11.33b: a proposal's `Ready` is never visible on disk before memory has it. A
//! `profile status` reads `proposal.json`; the snapshot (`ready_proposals`, the queued
//! goals' set-up) reads memory. Stopped between `save_if_current`'s two steps by its
//! seam, a status that shows `Ready` must find the snapshot `Ready` too, and the snapshot
//! already behaves as `Ready`. Before the fix the file was written first, so a status
//! could show `Ready` while the snapshot still said "checking" (the flake of
//! `profile_queue.rs::e2e_a_goal_with_yes_waits_for_review_when_a_command_was_dropped`).
//! A real service on a scratch data directory and a real git repository; no agent runs.

use std::sync::Arc;
use std::time::Duration;

use proto::{ProposalState, SetupState};

use super::store;
use super::tests_queue::goal;
use super::tests_queue_service::status;
use super::tests_ready::{Rig, record, repo};

/// One step of the service: a few small file writes on `spawn_blocking` and no git;
/// only a failure waits this long. The CLI harness's `REQUEST_WAIT`, as
/// `tests_queue_service.rs` uses it.
const REQUEST_WAIT: Duration = Duration::from_secs(75);

#[tokio::test(flavor = "multi_thread")]
async fn a_status_in_the_ready_gap_never_sees_ready_before_the_snapshot() {
    let rig = Rig::new();
    let project = repo(rig.dir.path(), "app");
    // A detection checking its commands: `Verifying` on disk and in memory.
    let verifying = record(&project, ProposalState::Verifying, 1_790_000_000);
    let dir = rig.repo_dir(&project);
    std::fs::create_dir_all(&dir).unwrap();
    store::save_proposal(&dir, &verifying).unwrap();
    rig.profiles.note_proposal(&project, Some(&verifying));
    let (generation, _token) = rig.profiles.register(&project).expect("registered");
    rig.profiles
        .queue_goal(goal(&project, "add a flag", false))
        .await
        .unwrap()
        .expect("queued");

    let (reached, resume) = (
        Arc::new(tokio::sync::Notify::new()),
        Arc::new(tokio::sync::Notify::new()),
    );
    *crate::lock(&rig.profiles.ready_gap) = Some((reached.clone(), resume.clone()));
    // `updated_at` differs from the `Verifying` record's, so a mix-up shows.
    let ready = record(&project, ProposalState::Ready, 1_790_000_077);
    let save = tokio::spawn({
        let (profiles, ready) = (rig.profiles.clone(), ready.clone());
        async move { profiles.save_if_current(generation, &ready).await }
    });
    tokio::time::timeout(REQUEST_WAIT, reached.notified())
        .await
        .expect("the save never reached its gap");

    // In the gap: what a client polling status and reading snapshots sees.
    let shown = status(&rig.profiles, &project).await;
    let (_, listed) = rig.profiles.ready_proposals();
    let goals = rig.profiles.queued_goals();
    let snapshot_ready = listed
        .iter()
        .any(|p| p.project == project && p.updated_at == 1_790_000_077);
    let snapshot_setup = goals.first().map(|g| g.setup.clone());
    if shown.proposal.as_ref().map(|r| &r.state) == Some(&ProposalState::Ready) {
        assert!(
            snapshot_ready,
            "status shows Ready while the snapshot lists no ready proposal: {listed:?}"
        );
        assert_eq!(snapshot_setup, Some(SetupState::NeedsReview), "{goals:?}");
    }
    assert!(
        snapshot_ready,
        "the snapshot does not behave as Ready in the gap: {listed:?}"
    );
    assert_eq!(snapshot_setup, Some(SetupState::NeedsReview), "{goals:?}");

    resume.notify_one();
    let saved = tokio::time::timeout(REQUEST_WAIT, save)
        .await
        .expect("the save ends")
        .unwrap();
    assert!(saved);
    let after = status(&rig.profiles, &project).await;
    assert_eq!(after.proposal.map(|r| r.state), Some(ProposalState::Ready));
    assert_eq!(after.queued[0].setup, SetupState::NeedsReview);
}
