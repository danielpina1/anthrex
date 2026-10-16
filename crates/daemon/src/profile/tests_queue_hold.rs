//! Milestone 9.16 decision 20 and ruling C9: queued goals wait under the hold without
//! being dropped, and `hold off` (`drain_all`) starts them in queue order. A real
//! service on a scratch data directory with a test starter and a test hold probe; no
//! agent runs.

use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use proto::ProfileRequest;
use tokio_util::sync::CancellationToken;

use super::queue::{self, QUEUE_FILE, QueuedGoal};
use super::service::ProfileService;
use super::service_queue::GoalStarter;
use super::tests_queue::goal;
use super::tests_queue_service::{Seen, done, ready_proposal, recorder, status, until};
use super::tests_ready::{Rig, repo, service};
use crate::run::engine::brakes::HOLD_TEXT;

/// A hold probe the test turns, and how often it was asked.
fn probe(profiles: &ProfileService) -> (Arc<AtomicBool>, Arc<AtomicUsize>) {
    let held = Arc::new(AtomicBool::new(true));
    let asked = Arc::new(AtomicUsize::new(0));
    let (h, a) = (held.clone(), asked.clone());
    profiles.set_dispatch_held(Arc::new(move || {
        a.fetch_add(1, Ordering::SeqCst);
        h.load(Ordering::SeqCst)
    }));
    (held, asked)
}

fn texts(seen: &Seen) -> Vec<String> {
    crate::lock(seen).iter().map(|g| g.goal.clone()).collect()
}

/// Whether `project`'s queue lists `n` goals outside any drain.
fn listed(profiles: &ProfileService, project: &Path, n: usize) -> bool {
    let waiting = profiles.queued_goals();
    waiting.iter().filter(|g| g.project == project).count() == n
        && !crate::lock(&profiles.table).draining.contains(project)
}

#[tokio::test(flavor = "multi_thread")]
async fn queued_goal_stays_queued_under_hold() {
    let rig = Rig::new();
    let seen = recorder(&rig.profiles, Ok("run-1"));
    let (held, asked) = probe(&rig.profiles);
    let project = repo(rig.dir.path(), "app");
    ready_proposal(&rig, &project, Vec::new());
    for text in ["first", "second"] {
        rig.profiles
            .queue_goal(goal(&project, text, false))
            .await
            .unwrap()
            .expect("queued");
    }
    // The profile is confirmed under the hold.
    done(
        rig.profiles
            .request(ProfileRequest::Confirm {
                dir: project.clone(),
                shown: None,
            })
            .await,
    );
    until("the held drain to end", || {
        asked.load(Ordering::SeqCst) > 0 && listed(&rig.profiles, &project, 2)
    })
    .await;
    assert!(texts(&seen).is_empty(), "a goal started under the hold");
    let status = status(&rig.profiles, &project).await;
    assert!(
        status.dropped_goals.is_empty(),
        "{:?}",
        status.dropped_goals
    );
    let on_file = queue::load(&rig.repo_dir(&project)).unwrap();
    assert_eq!(on_file.goals.len(), 2, "{on_file:?}");
    assert!(on_file.starting.is_empty() && on_file.dropped.is_empty());

    // `hold off`: every held goal starts, in queue order.
    held.store(false, Ordering::SeqCst);
    rig.profiles.drain_all();
    until("both goals to start", || texts(&seen).len() == 2).await;
    assert_eq!(texts(&seen), ["first", "second"]);
    let file = rig.repo_dir(&project).join(QUEUE_FILE);
    until("the queue to empty", || !file.exists()).await;
    assert!(
        status_of(&rig, &project).await.is_empty(),
        "a goal was dropped"
    );
}

async fn status_of(rig: &Rig, project: &Path) -> Vec<proto::DroppedGoal> {
    status(&rig.profiles, project).await.dropped_goals
}

/// Ruling C9: a start refused with exactly `HOLD_TEXT` (the hold came on between the
/// probe and the start) goes back to the head of the queue and is recorded as nothing.
#[tokio::test(flavor = "multi_thread")]
async fn a_start_refused_by_the_hold_goes_back_to_the_head() {
    let rig = Rig::new();
    let (held, _asked) = probe(&rig.profiles);
    held.store(false, Ordering::SeqCst);
    let calls = Arc::new(AtomicUsize::new(0));
    let (c, h) = (calls.clone(), held.clone());
    let starter: GoalStarter = Arc::new(move |_goal: QueuedGoal| {
        c.fetch_add(1, Ordering::SeqCst);
        // The hold comes on as the start is made.
        h.store(true, Ordering::SeqCst);
        Box::pin(async move { Err(HOLD_TEXT.to_string()) })
    });
    rig.profiles.set_goal_starter(starter);
    let project = repo(rig.dir.path(), "app");
    for text in ["first", "second"] {
        rig.profiles
            .queue_goal(goal(&project, text, false))
            .await
            .unwrap();
    }
    rig.store_profile(&project);
    rig.profiles.drain_all();
    until("the refused drain to end", || {
        calls.load(Ordering::SeqCst) == 1 && listed(&rig.profiles, &project, 2)
    })
    .await;
    let on_file = queue::load(&rig.repo_dir(&project)).unwrap();
    let order: Vec<&str> = on_file.goals.iter().map(|g| g.goal.as_str()).collect();
    assert_eq!(order, ["first", "second"]);
    assert!(on_file.dropped.is_empty() && on_file.starting.is_empty());
    assert_eq!(calls.load(Ordering::SeqCst), 1, "a second goal was tried");
}

/// Ruling C9: a daemon restarted with the hold persisted drains nothing at start; the
/// goals start once the hold is lifted.
#[tokio::test(flavor = "multi_thread")]
async fn restore_drain_waits_for_a_persisted_hold() {
    let rig = Rig::new();
    let project = repo(rig.dir.path(), "app");
    rig.store_profile(&project);
    super::tests_queue_service::write_queue(&rig, &project, vec![goal(&project, "waiting", false)]);

    let fresh = service(rig.dir.path());
    let seen = recorder(&fresh, Ok("run-1"));
    let (held, asked) = probe(&fresh);
    fresh.restore().await;
    let token = CancellationToken::new();
    fresh.spawn(token.clone());
    until("the start's drain to find the hold", || {
        asked.load(Ordering::SeqCst) > 0 && listed(&fresh, &project, 1)
    })
    .await;
    assert!(texts(&seen).is_empty(), "a goal started under the hold");
    held.store(false, Ordering::SeqCst);
    fresh.drain_all();
    until("the goal to start", || texts(&seen) == ["waiting"]).await;
    token.cancel();
}

/// A stored profile for `project` whose fingerprint no longer matches its tree, so
/// the start's staleness check (decision 7) would re-detect it.
fn store_stale(rig: &Rig, project: &Path) {
    let dir = rig.repo_dir(project);
    std::fs::create_dir_all(&dir).unwrap();
    let meta = proto::ProfileMeta {
        confirmed_at: 1,
        report: None,
        verification: None,
        fingerprint: [("src/lib.rs".to_string(), "stale".to_string())].into(),
        edited_keys: Vec::new(),
        project: Some(project.to_path_buf()),
    };
    super::store::save(&dir, &proto::RepoProfile::default(), &meta).unwrap();
}

/// Whether a detection was started (or recorded) for `project`.
fn detected(rig: &Rig, project: &Path) -> bool {
    super::store::load_proposal(&rig.repo_dir(project))
        .ok()
        .flatten()
        .is_some()
}

/// Task 8's review I1 (the controller's ruling): `profile detect` is refused with the
/// hold's text while the hold is on, and no scout starts.
#[tokio::test(flavor = "multi_thread")]
async fn detect_is_refused_under_the_hold() {
    let rig = Rig::new();
    let (_held, _asked) = probe(&rig.profiles);
    let project = repo(rig.dir.path(), "app");
    let detect = ProfileRequest::Detect {
        dir: project.clone(),
        trust_project: false,
        unconfined_checks: true,
    };
    let reply = rig.profiles.request(detect).await;
    assert_eq!(
        reply,
        proto::ProfileReply::Refused {
            message: HOLD_TEXT.to_string()
        }
    );
    assert!(
        !detected(&rig, &project),
        "a detection started under the hold"
    );
    assert!(crate::lock(&rig.profiles.table).active.is_empty());
}

/// Task 8's review I1: the start's staleness check scouts nothing while the hold is
/// on, and runs once `hold off` (`drain_all`) is issued.
#[tokio::test(flavor = "multi_thread")]
async fn the_starts_staleness_check_waits_for_hold_off() {
    let rig = Rig::new();
    let project = repo(rig.dir.path(), "app");
    store_stale(&rig, &project);
    let fresh = service(rig.dir.path());
    let (held, asked) = probe(&fresh);
    fresh.restore().await;
    let token = CancellationToken::new();
    fresh.spawn(token.clone());
    until("the staleness check to find the hold", || {
        asked.load(Ordering::SeqCst) > 0 && crate::lock(&fresh.table).auto_at_start.len() == 1
    })
    .await;
    assert!(!detected(&rig, &project), "a scout started under the hold");
    held.store(false, Ordering::SeqCst);
    fresh.drain_all();
    until("the re-detection to start", || detected(&rig, &project)).await;
    assert!(crate::lock(&fresh.table).auto_at_start.is_empty());
    // Whatever the detection started (its agent does not exist), stop it.
    let _ = fresh.request(ProfileRequest::Reject { dir: project }).await;
    token.cancel();
}
