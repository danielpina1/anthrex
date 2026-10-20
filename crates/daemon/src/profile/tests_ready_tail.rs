//! The profile "Use this" race (CI, `profile_cli_tui::e2e_tui_queued_goal_review_and_use_this`):
//! a detection publishes `Ready` (the alert, `proposal.json`) while it is still registered,
//! and stays registered for its tail (`after_ready`, `unmark`, then `unregister`). A
//! `confirm`, `detect` or `edit` in that window waits for `unregister`, bounded by
//! `PROFILE_READY_TAIL_BOUND`, instead of refusing "already running (state ready)". A
//! detection still at work is refused at once, as before.
//! A real service on a scratch data directory and a real git repository; no agent runs.

use std::path::Path;
use std::time::{Duration, Instant};

use proto::{ProfileReply, ProfileRequest, ProposalState};

use super::service::already_running;
use super::store::{self, Stored};
use super::tests_ready::{Rig, record, repo};

/// One request of the service: a git call and a few small file reads on
/// `spawn_blocking`; only a failure waits this long (`tests_ready_gap.rs`'s bound).
const REQUEST_WAIT: Duration = Duration::from_secs(75);

/// A tail bound no test reaches unless the request waits for it wrongly.
const LONG_BOUND: Duration = Duration::from_secs(3600);

/// The timeout test's injected bound.
const SHORT_BOUND: Duration = Duration::from_millis(300);

impl Rig {
    /// `project`'s proposal written in `state` and noted, as a detection's save leaves it.
    fn proposal(&self, project: &Path, state: ProposalState) {
        let written = record(project, state, 1_790_000_000);
        let dir = self.repo_dir(project);
        std::fs::create_dir_all(&dir).unwrap();
        store::save_proposal(&dir, &written).unwrap();
        self.profiles.note_proposal(project, Some(&written));
    }

    fn set_tail_bound(&self, bound: Duration) {
        *crate::lock(&self.profiles.tail_bound) = Some(bound);
    }

    /// How many requests wait on `project`'s registration's tail.
    fn tail_waiters(&self, project: &Path) -> usize {
        crate::lock(&self.profiles.table)
            .active
            .get(project)
            .map_or(0, |active| active.done.receiver_count())
    }

    /// Deadline loop: until a request waits on the tail, or `task` has already ended.
    async fn until_waiting<T>(&self, project: &Path, task: &tokio::task::JoinHandle<T>) {
        let deadline = Instant::now() + REQUEST_WAIT;
        while self.tail_waiters(project) == 0 && !task.is_finished() {
            assert!(
                Instant::now() < deadline,
                "the request never started waiting"
            );
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }
}

fn confirm(project: &Path) -> ProfileRequest {
    ProfileRequest::Confirm {
        dir: project.to_path_buf(),
        shown: None,
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn a_confirm_in_the_ready_tail_waits_for_unregister_and_stores() {
    let rig = Rig::new();
    rig.set_tail_bound(LONG_BOUND);
    let project = repo(rig.dir.path(), "app");
    let (generation, _token) = rig.profiles.register(&project).expect("registered");
    rig.proposal(&project, ProposalState::Ready);

    let task = tokio::spawn({
        let (profiles, request) = (rig.profiles.clone(), confirm(&project));
        async move { profiles.request(request).await }
    });
    rig.until_waiting(&project, &task).await;
    assert!(
        !task.is_finished(),
        "confirm answered before the detection unregistered: {:?}",
        task.await.unwrap()
    );
    rig.profiles.unregister(&project, generation);

    let reply = tokio::time::timeout(REQUEST_WAIT, task)
        .await
        .expect("confirm ends after unregister")
        .unwrap();
    assert!(matches!(reply, ProfileReply::Done { .. }), "{reply:?}");
    assert!(
        rig.on_disk(&project).is_none(),
        "the proposal is still on disk"
    );
    let stored = store::load(&rig.repo_dir(&project));
    assert!(matches!(stored, Stored::Found { .. }), "nothing stored");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_confirm_whose_tail_never_ends_refuses_after_the_bound() {
    let rig = Rig::new();
    rig.set_tail_bound(SHORT_BOUND);
    let project = repo(rig.dir.path(), "app");
    let (_generation, _token) = rig.profiles.register(&project).expect("registered");
    rig.proposal(&project, ProposalState::Ready);

    let started = Instant::now();
    let reply = tokio::time::timeout(REQUEST_WAIT, rig.profiles.request(confirm(&project)))
        .await
        .expect("confirm ends");
    let elapsed = started.elapsed();
    // Today's refusal text, unchanged.
    let expected = already_running(&project, &ProposalState::Ready);
    assert!(
        matches!(&reply, ProfileReply::Refused { message } if *message == expected),
        "{reply:?}"
    );
    assert!(
        elapsed >= SHORT_BOUND,
        "refused after {elapsed:?}, before the {SHORT_BOUND:?} bound"
    );
    assert!(rig.on_disk(&project).is_some(), "the proposal was consumed");
}

/// A guard: it passes on the base too. A detection still at work (`Verifying`) is refused
/// at once, never after the tail bound. The limit is the request's own legal worst case
/// (`project_of`'s git call, `git_timeout`, and the file reads), far below the bound.
#[tokio::test(flavor = "multi_thread")]
async fn a_confirm_while_detection_still_runs_refuses_at_once() {
    let rig = Rig::new();
    rig.set_tail_bound(LONG_BOUND);
    let project = repo(rig.dir.path(), "app");
    let (_generation, _token) = rig.profiles.register(&project).expect("registered");
    rig.proposal(&project, ProposalState::Verifying);

    let limit = rig.profiles.git_timeout() + REQUEST_WAIT;
    assert!(limit < LONG_BOUND);
    let reply = tokio::time::timeout(limit, rig.profiles.request(confirm(&project)))
        .await
        .expect("a running detection is refused at once, not after the tail bound");
    let expected = already_running(&project, &ProposalState::Verifying);
    assert!(
        matches!(&reply, ProfileReply::Refused { message } if *message == expected),
        "{reply:?}"
    );
    assert_eq!(rig.tail_waiters(&project), 0);
}

/// `detect` and `edit` refuse through `refuse_if_running`: it waits for the tail too.
#[tokio::test(flavor = "multi_thread")]
async fn refuse_if_running_in_the_ready_tail_waits_for_unregister() {
    let rig = Rig::new();
    rig.set_tail_bound(LONG_BOUND);
    let project = repo(rig.dir.path(), "app");
    let (generation, _token) = rig.profiles.register(&project).expect("registered");
    rig.proposal(&project, ProposalState::Ready);

    let task = tokio::spawn({
        let (profiles, project) = (rig.profiles.clone(), project.clone());
        async move { profiles.refuse_if_running(&project).await }
    });
    rig.until_waiting(&project, &task).await;
    assert!(
        !task.is_finished(),
        "refused before the detection unregistered: {:?}",
        task.await.unwrap()
    );
    rig.profiles.unregister(&project, generation);

    let answer = tokio::time::timeout(REQUEST_WAIT, task)
        .await
        .expect("the check ends after unregister")
        .unwrap();
    assert_eq!(answer, Ok(()));
}

/// A guard for `refuse_if_running`: a detection still at work is refused at once.
#[tokio::test(flavor = "multi_thread")]
async fn refuse_if_running_while_detection_still_runs_refuses_at_once() {
    let rig = Rig::new();
    rig.set_tail_bound(LONG_BOUND);
    let project = repo(rig.dir.path(), "app");
    let (_generation, _token) = rig.profiles.register(&project).expect("registered");
    rig.proposal(&project, ProposalState::Scouting);

    let answer = tokio::time::timeout(REQUEST_WAIT, rig.profiles.refuse_if_running(&project))
        .await
        .expect("refused at once, not after the tail bound");
    assert_eq!(
        answer,
        Err(already_running(&project, &ProposalState::Scouting))
    );
}
