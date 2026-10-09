//! Milestone 9.10 task M9.10.4: the service's goal queue (decisions 4 to 11). A real
//! service on a scratch data directory and real git repositories, with a test starter
//! in place of `RunService::start_queued_goal`; no agent runs (Claude and Codex are
//! paths that do not exist).

use std::path::Path;
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use proto::{
    DroppedCommand, ProfileReply, ProfileRequest, ProfileStatus, ProposalRecord, ProposalState,
    SetupState,
};
use tokio_util::sync::CancellationToken;

use super::queue::{self, GoalQueue, MAX_QUEUED, QUEUE_FILE, QueuedGoal};
use super::service::{ProfileService, RESTART_REASON};
use super::service_queue::GoalStarter;
use super::store::{self, Stored};
use super::tests_queue::goal;
use super::tests_ready::{Rig, record, repo, service};

/// How long a test waits for a drain the service spawned: the drain is a few small
/// file writes on `spawn_blocking` (the queue file, under `writes`) and the test
/// starter, which answers at once; no git and no agent. This is the CLI harness's
/// `REQUEST_WAIT` (`cli/tests/support/run_harness.rs`), far above that; only a
/// failure waits this long, the poll returns as soon as the effect is seen.
const REQUEST_WAIT: Duration = Duration::from_secs(75);
/// Between two looks.
const POLL: Duration = Duration::from_millis(20);

type Seen = Arc<Mutex<Vec<QueuedGoal>>>;

/// A starter that records each goal it is given and answers `answer`.
fn recorder(profiles: &ProfileService, answer: Result<&'static str, &'static str>) -> Seen {
    let seen: Seen = Arc::default();
    let inner = seen.clone();
    let starter: GoalStarter = Arc::new(move |goal: QueuedGoal| {
        crate::lock(&inner).push(goal);
        Box::pin(async move { answer.map(str::to_string).map_err(str::to_string) })
    });
    profiles.set_goal_starter(starter);
    seen
}

/// Polls `check` every `POLL` until it holds, at most `REQUEST_WAIT`.
async fn until(what: &str, mut check: impl FnMut() -> bool) {
    let deadline = Instant::now() + REQUEST_WAIT;
    while !check() {
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        tokio::time::sleep(POLL).await;
    }
}

/// A `Ready` detection proposal on disk and in the table, as `verify_phase` leaves it.
fn ready_proposal(rig: &Rig, project: &Path, dropped: Vec<DroppedCommand>) -> ProposalRecord {
    let mut ready = record(project, ProposalState::Ready, 1_790_000_000);
    ready.dropped = dropped;
    let dir = rig.repo_dir(project);
    std::fs::create_dir_all(&dir).unwrap();
    store::save_proposal(&dir, &ready).unwrap();
    rig.profiles.note_proposal(project, Some(&ready));
    ready
}

async fn status(profiles: &Arc<ProfileService>, project: &Path) -> ProfileStatus {
    match profiles
        .request(ProfileRequest::Status {
            dir: project.to_path_buf(),
        })
        .await
    {
        ProfileReply::Status(status) => status,
        other => panic!("{other:?}"),
    }
}

fn done(reply: ProfileReply) -> String {
    match reply {
        ProfileReply::Done { message } => message,
        other => panic!("{other:?}"),
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn a_queued_goal_waits_and_a_confirm_starts_it() {
    let rig = Rig::new();
    let seen = recorder(&rig.profiles, Ok("run-1"));
    let project = repo(rig.dir.path(), "app");
    ready_proposal(&rig, &project, Vec::new());

    let queued = rig
        .profiles
        .queue_goal(goal(&project, "add a flag", false))
        .await
        .unwrap();
    assert_eq!(queued.setup, SetupState::NeedsReview);
    let listed = rig.profiles.queued_goals();
    assert_eq!(listed.len(), 1, "{listed:?}");
    assert_eq!(listed[0].goal, "add a flag");
    assert_eq!(listed[0].setup, SetupState::NeedsReview);
    assert!(rig.repo_dir(&project).join(QUEUE_FILE).is_file());

    let message = done(
        rig.profiles
            .request(ProfileRequest::Confirm {
                dir: project.clone(),
                shown: None,
            })
            .await,
    );
    assert!(
        message.starts_with("stored the profile for ")
            && message.ends_with("; starting 1 queued goal"),
        "{message}"
    );
    until("the starter to get the goal", || {
        crate::lock(&seen).len() == 1
    })
    .await;
    assert_eq!(crate::lock(&seen)[0].goal, "add a flag");
    assert!(rig.profiles.queued_goals().is_empty());
    assert!(!rig.repo_dir(&project).join(QUEUE_FILE).exists());
}

#[tokio::test(flavor = "multi_thread")]
async fn a_full_queue_refuses_the_ninth() {
    let rig = Rig::new();
    let _seen = recorder(&rig.profiles, Ok("run-1"));
    let project = repo(rig.dir.path(), "app");
    for n in 0..MAX_QUEUED {
        rig.profiles
            .queue_goal(goal(&project, &format!("goal {n}"), false))
            .await
            .unwrap();
    }
    let refused = rig
        .profiles
        .queue_goal(goal(&project, "one too many", false))
        .await
        .unwrap_err();
    assert_eq!(
        refused,
        "app already has 8 goals waiting for its profile; review it first (C-b P, or anthrex profile status)"
    );
    assert_eq!(rig.profiles.queued_goals().len(), MAX_QUEUED);
}

#[tokio::test(flavor = "multi_thread")]
async fn reject_drops_the_queue_with_a_note() {
    let rig = Rig::new();
    let seen = recorder(&rig.profiles, Ok("run-1"));
    let project = repo(rig.dir.path(), "app");
    ready_proposal(&rig, &project, Vec::new());
    for text in ["first", "second"] {
        rig.profiles
            .queue_goal(goal(&project, text, false))
            .await
            .unwrap();
    }
    let message = done(
        rig.profiles
            .request(ProfileRequest::Reject {
                dir: project.clone(),
            })
            .await,
    );
    assert!(
        message.starts_with("rejected the proposal for ")
            && message.ends_with("; dropped 2 queued goals"),
        "{message}"
    );
    assert!(rig.profiles.queued_goals().is_empty());
    let status = status(&rig.profiles, &project).await;
    assert!(status.queued.is_empty());
    let dropped: Vec<_> = status
        .dropped_goals
        .iter()
        .map(|d| (d.goal.as_str(), d.reason.as_str()))
        .collect();
    assert_eq!(
        dropped,
        [
            ("first", "the proposal was discarded"),
            ("second", "the proposal was discarded")
        ]
    );
    assert!(crate::lock(&seen).is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn a_failed_start_is_dropped_with_its_reason() {
    let rig = Rig::new();
    let seen = recorder(&rig.profiles, Err("base branch main is gone"));
    let project = repo(rig.dir.path(), "app");
    ready_proposal(&rig, &project, Vec::new());
    rig.profiles
        .queue_goal(goal(&project, "add a flag", false))
        .await
        .unwrap();
    done(
        rig.profiles
            .request(ProfileRequest::Confirm {
                dir: project.clone(),
                shown: None,
            })
            .await,
    );
    let repo_dir = rig.repo_dir(&project);
    until("the refused start to be recorded", || {
        queue::load(&repo_dir).is_ok_and(|queue| !queue.dropped.is_empty())
    })
    .await;
    assert_eq!(crate::lock(&seen).len(), 1);
    let status = status(&rig.profiles, &project).await;
    assert_eq!(status.dropped_goals.len(), 1, "{:?}", status.dropped_goals);
    assert_eq!(status.dropped_goals[0].goal, "add a flag");
    assert_eq!(status.dropped_goals[0].reason, "base branch main is gone");
    assert!(status.queued.is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn after_ready_stores_for_yes_when_nothing_was_dropped() {
    let rig = Rig::new();
    let seen = recorder(&rig.profiles, Ok("run-1"));
    let project = repo(rig.dir.path(), "app");
    ready_proposal(&rig, &project, Vec::new());
    rig.profiles
        .queue_goal(goal(&project, "add a flag", true))
        .await
        .unwrap();

    rig.profiles.after_ready(&project).await;
    assert!(matches!(
        store::load(&rig.repo_dir(&project)),
        Stored::Found { .. }
    ));
    assert!(rig.on_disk(&project).is_none());
    assert!(rig.profiles.ready_proposals().1.is_empty());
    until("the starter to get the goal", || {
        crate::lock(&seen).len() == 1
    })
    .await;
    assert!(rig.profiles.queued_goals().is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn after_ready_raises_the_review_when_something_was_dropped() {
    let rig = Rig::new();
    let seen = recorder(&rig.profiles, Ok("run-1"));
    let project = repo(rig.dir.path(), "app");
    let dropped = DroppedCommand {
        key: "check".into(),
        command: "make check".into(),
        reason: "exited 2".into(),
        tail: String::new(),
    };
    ready_proposal(&rig, &project, vec![dropped]);
    rig.profiles
        .queue_goal(goal(&project, "add a flag", true))
        .await
        .unwrap();

    rig.profiles.after_ready(&project).await;
    assert!(matches!(
        store::load(&rig.repo_dir(&project)),
        Stored::Absent
    ));
    let listed = rig.profiles.ready_proposals().1;
    assert_eq!(listed.len(), 1, "{listed:?}");
    assert_eq!(listed[0].project, project);
    let queued = rig.profiles.queued_goals();
    assert_eq!(queued.len(), 1);
    assert_eq!(queued[0].setup, SetupState::NeedsReview);
    assert!(crate::lock(&seen).is_empty());
}

/// A queue file and a stored profile from before a restart.
fn write_queue(rig: &Rig, project: &Path, goals: Vec<QueuedGoal>) {
    let dir = rig.repo_dir(project);
    std::fs::create_dir_all(&dir).unwrap();
    queue::save(
        &dir,
        &GoalQueue {
            goals,
            dropped: Vec::new(),
        },
    )
    .unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn restore_keeps_the_queue_and_drains_a_stored_repository() {
    let rig = Rig::new();
    let project = repo(rig.dir.path(), "app");
    rig.store_profile(&project);
    write_queue(&rig, &project, vec![goal(&project, "add a flag", false)]);

    let fresh = service(rig.dir.path());
    let seen = recorder(&fresh, Ok("run-1"));
    fresh.restore().await;
    assert_eq!(fresh.queued_goals().len(), 1);
    let token = CancellationToken::new();
    fresh.spawn(token.clone());
    until("the restored goal to start", || {
        crate::lock(&seen).len() == 1
    })
    .await;
    assert_eq!(crate::lock(&seen)[0].goal, "add a flag");
    until("the queue to empty", || fresh.queued_goals().is_empty()).await;
    token.cancel();
}

#[tokio::test(flavor = "multi_thread")]
async fn restore_drops_a_goal_whose_directory_is_gone() {
    let rig = Rig::new();
    let project = rig.dir.path().join("gone-app");
    write_queue(&rig, &project, vec![goal(&project, "add a flag", false)]);

    let fresh = service(rig.dir.path());
    let seen = recorder(&fresh, Ok("run-1"));
    fresh.restore().await;
    assert_eq!(fresh.queued_goals().len(), 1);
    let token = CancellationToken::new();
    fresh.spawn(token.clone());
    let repo_dir = rig.repo_dir(&project);
    until("the gone goal to be dropped", || {
        queue::load(&repo_dir)
            .is_ok_and(|queue| queue.goals.is_empty() && !queue.dropped.is_empty())
    })
    .await;
    let queue = queue::load(&repo_dir).unwrap();
    assert_eq!(queue.dropped[0].goal, "add a flag");
    assert_eq!(queue.dropped[0].reason, "the repository is gone");
    assert!(fresh.queued_goals().is_empty());
    assert!(crate::lock(&seen).is_empty());
    token.cancel();
}

#[tokio::test(flavor = "multi_thread")]
async fn restore_shows_an_interrupted_set_up_as_failed() {
    let rig = Rig::new();
    let project = repo(rig.dir.path(), "app");
    write_queue(&rig, &project, vec![goal(&project, "add a flag", false)]);
    store::save_proposal(
        &rig.repo_dir(&project),
        &record(&project, ProposalState::Scouting, 1_790_000_000),
    )
    .unwrap();

    let fresh = service(rig.dir.path());
    fresh.restore().await;
    let queued = fresh.queued_goals();
    assert_eq!(queued.len(), 1, "{queued:?}");
    assert_eq!(
        queued[0].setup,
        SetupState::Failed {
            reason: RESTART_REASON.to_string()
        }
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn the_snapshot_generation_moves_with_the_queue_and_the_progress() {
    let rig = Rig::new();
    let project = repo(rig.dir.path(), "app");
    let profiles = &rig.profiles;
    let mut last = profiles.snapshot_generation();
    let mut moved = |what: &str, now: u64| {
        assert!(now > last, "{what}: {now} is not above {last}");
        last = now;
    };

    profiles
        .queue_goal(goal(&project, "add a flag", false))
        .await
        .unwrap();
    moved("queue_goal", profiles.snapshot_generation());

    let (generation, _token) = profiles.register(&project).expect("nothing runs yet");
    let ready_before = profiles.ready_proposals();
    for state in [
        ProposalState::Preparing,
        ProposalState::Scouting,
        ProposalState::Verifying,
        ProposalState::Failed {
            reason: "the check failed".into(),
        },
    ] {
        let written = record(&project, state.clone(), 1_790_000_000);
        assert!(profiles.save_if_current(generation, &written).await);
        moved(&format!("{state:?}"), profiles.snapshot_generation());
    }
    // None of those writes changed the ready list.
    assert_eq!(profiles.ready_proposals(), ready_before);

    let counter = profiles
        .counter(&project, generation)
        .expect("the registration's counter");
    counter.done.fetch_add(1, Ordering::Relaxed);
    counter.ticks.fetch_add(1, Ordering::Relaxed);
    moved("a command's tick", profiles.snapshot_generation());

    assert_eq!(
        profiles
            .drop_queued(&project, "the proposal was discarded")
            .await,
        1
    );
    moved("drop_queued", profiles.snapshot_generation());

    profiles.unregister(&project, generation);
    assert!(
        profiles.snapshot_generation() >= last,
        "the generation went down when the job ended"
    );
}
