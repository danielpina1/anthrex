//! Milestone 9.10 final re-review residuals: what may happen to a project's queue
//! while a drain runs. A `profile reject` leaves the goals waiting their turn (N1), a
//! second drain of the same project returns at once and the order holds (the surviving
//! mutant), and a starter that panics is a failed start, not a stuck drain. A real
//! service on a scratch data directory, with test starters; no agent runs.

use std::sync::Arc;

use proto::{ProfileReply, ProfileRequest};
use tokio::sync::Semaphore;

use super::queue::QueuedGoal;
use super::service_queue::GoalStarter;
use super::tests_queue::goal;
use super::tests_queue_service::{Seen, done, ready_proposal, status, until};
use super::tests_ready::{Rig, repo};

/// A starter that records each goal and holds the start of `hold` until the returned
/// gate gets a permit.
fn gated(rig: &Rig, hold: &'static str) -> (Seen, Arc<Semaphore>) {
    let seen: Seen = Arc::default();
    let gate = Arc::new(Semaphore::new(0));
    let (inner, held) = (seen.clone(), gate.clone());
    let starter: GoalStarter = Arc::new(move |goal: QueuedGoal| {
        crate::lock(&inner).push(goal.clone());
        let held = held.clone();
        Box::pin(async move {
            if goal.goal == hold {
                held.acquire().await.unwrap().forget();
            }
            Ok("run-1".to_string())
        })
    });
    rig.profiles.set_goal_starter(starter);
    (seen, gate)
}

fn texts(seen: &Seen) -> Vec<String> {
    crate::lock(seen).iter().map(|g| g.goal.clone()).collect()
}

/// Three goals queued behind a ready proposal, and a **Use this** whose drain is held
/// in the first goal's start.
async fn drain_held_on_first(rig: &Rig, project: &std::path::Path) -> (Seen, Arc<Semaphore>) {
    let (seen, gate) = gated(rig, "first");
    ready_proposal(rig, project, Vec::new());
    for text in ["first", "second", "third"] {
        rig.profiles
            .queue_goal(goal(project, text, false))
            .await
            .unwrap();
    }
    let message = done(
        rig.profiles
            .request(ProfileRequest::Confirm {
                dir: project.to_path_buf(),
                shown: None,
            })
            .await,
    );
    assert!(message.ends_with("; starting 3 queued goals"), "{message}");
    until("the first start to begin", || texts(&seen) == ["first"]).await;
    (seen, gate)
}

/// N1: `profile reject` while a drain is starting the first of three goals drops
/// neither of the two waiting their turn. They wait on the stored profile, not on a
/// proposal, and both start once the first start returns.
#[tokio::test(flavor = "multi_thread")]
async fn a_reject_during_a_drain_keeps_the_goals_waiting_their_turn() {
    let rig = Rig::new();
    let project = repo(rig.dir.path(), "app");
    let (seen, gate) = drain_held_on_first(&rig, &project).await;

    let reply = rig
        .profiles
        .request(ProfileRequest::Reject {
            dir: project.clone(),
        })
        .await;
    match &reply {
        ProfileReply::Refused { message } => {
            assert!(message.starts_with("no proposal for "), "{message}");
        }
        other => panic!("reject dropped or rejected something: {other:?}"),
    }
    assert!(
        status(&rig.profiles, &project)
            .await
            .dropped_goals
            .is_empty()
    );

    gate.add_permits(1);
    until("the waiting goals to start", || texts(&seen).len() == 3).await;
    assert_eq!(texts(&seen), ["first", "second", "third"]);
    let status = status(&rig.profiles, &project).await;
    assert!(
        status.dropped_goals.is_empty(),
        "{:?}",
        status.dropped_goals
    );
}

/// The surviving mutant: one drain per project. A second **Use this** (a profile edit
/// stored while the first drain is held) and restore's own drain both find the drain
/// running and return without starting anything; the running drain starts every goal
/// in queue order, and only its end clears the project's drain.
#[tokio::test(flavor = "multi_thread")]
async fn one_drain_per_project_keeps_the_queue_order() {
    let rig = Rig::new();
    let project = repo(rig.dir.path(), "app");
    let (seen, gate) = drain_held_on_first(&rig, &project).await;

    rig.edit(&project).await;
    let message = done(
        rig.profiles
            .request(ProfileRequest::Confirm {
                dir: project.clone(),
                shown: None,
            })
            .await,
    );
    assert!(message.ends_with("; starting 2 queued goals"), "{message}");
    // Restore's drain of the same project, awaited: it must return at once.
    rig.profiles.clone().start_queued(project.clone()).await;
    assert_eq!(texts(&seen), ["first"], "a second drain started a goal");
    // The goals still wait their turn in the running drain, not for a set-up.
    assert!(rig.profiles.queued_goals().is_empty());
    assert!(crate::lock(&rig.profiles.table).draining.contains(&project));

    gate.add_permits(1);
    until("the drain to end", || {
        !crate::lock(&rig.profiles.table).draining.contains(&project)
    })
    .await;
    assert_eq!(texts(&seen), ["first", "second", "third"]);
}

/// Residual ruling: a starter that panics is a failed start. The goal is dropped with
/// a note, the next goal still starts, and the drain ends, so a later drain of the
/// project runs.
#[tokio::test(flavor = "multi_thread")]
async fn a_panicking_start_is_a_failed_start_and_the_drain_ends() {
    let rig = Rig::new();
    let project = repo(rig.dir.path(), "app");
    let seen: Seen = Arc::default();
    let inner = seen.clone();
    let starter: GoalStarter = Arc::new(move |goal: QueuedGoal| {
        crate::lock(&inner).push(goal.clone());
        Box::pin(async move {
            assert!(goal.goal != "boom", "the starter broke");
            Ok("run-1".to_string())
        })
    });
    rig.profiles.set_goal_starter(starter);
    for text in ["boom", "after"] {
        rig.profiles
            .queue_goal(goal(&project, text, false))
            .await
            .unwrap();
    }
    rig.store_profile(&project);

    rig.profiles.clone().start_queued(project.clone()).await;
    assert_eq!(texts(&seen), ["boom", "after"]);
    assert!(!crate::lock(&rig.profiles.table).draining.contains(&project));
    let (queued, dropped) = rig.profiles.queued_for(&project);
    assert!(queued.is_empty(), "{queued:?}");
    assert_eq!(dropped.len(), 1, "{dropped:?}");
    assert_eq!(dropped[0].goal, "boom");
    assert!(
        dropped[0]
            .reason
            .starts_with("the start failed unexpectedly (the starter broke")
            && dropped[0].reason.ends_with("); start it again"),
        "{}",
        dropped[0].reason
    );
}

/// N1, the window before the drain's task runs: **Use this** claims the project's
/// drain under the store's own hold of `writes`, so by its answer the goals already
/// wait their turn in the drain (a `reject` then leaves them, and the snapshot does
/// not show them waiting for a set-up). One thread, so the drain's task cannot run
/// before the check.
#[tokio::test(flavor = "current_thread")]
async fn a_use_this_claims_its_drain_before_it_answers() {
    let rig = Rig::new();
    let project = repo(rig.dir.path(), "app");
    let (_seen, _gate) = gated(&rig, "first");
    ready_proposal(&rig, &project, Vec::new());
    rig.profiles
        .queue_goal(goal(&project, "first", false))
        .await
        .unwrap();
    let message = done(
        rig.profiles
            .request(ProfileRequest::Confirm {
                dir: project.clone(),
                shown: None,
            })
            .await,
    );
    assert!(message.ends_with("; starting 1 queued goal"), "{message}");
    assert!(crate::lock(&rig.profiles.table).draining.contains(&project));
    assert!(rig.profiles.queued_goals().is_empty());
}
