//! Milestone 9.10 final review fixes (daemon) I1 and M5: a drain takes one goal out at
//! a time, so a daemon that stops mid-drain loses no goal it had not started, and the
//! goal it was starting is a dropped goal with a note. A real service on a scratch data
//! directory, with test starters; no agent runs.

use std::sync::{Arc, Mutex};

use proto::ProfileRequest;
use tokio_util::sync::CancellationToken;

use super::queue::{self, QUEUE_FILE, QueuedGoal};
use super::service_queue::{GoalStarter, INTERRUPTED};
use super::tests_queue::goal;
use super::tests_queue_service::{Seen, done, ready_proposal, recorder, status, until};
use super::tests_ready::{Rig, repo, service};

/// A starter that records each goal and never answers for the goal `hang`: the start
/// a daemon stop interrupts.
fn hanging(rig: &Rig, hang: &'static str) -> Seen {
    let seen: Seen = Arc::default();
    let inner = seen.clone();
    let starter: GoalStarter = Arc::new(move |goal: QueuedGoal| {
        crate::lock(&inner).push(goal.clone());
        Box::pin(async move {
            if goal.goal == hang {
                std::future::pending::<()>().await;
            }
            Ok("run-1".to_string())
        })
    });
    rig.profiles.set_goal_starter(starter);
    seen
}

fn texts(seen: &Mutex<Vec<QueuedGoal>>) -> Vec<String> {
    crate::lock(seen).iter().map(|g| g.goal.clone()).collect()
}

/// I1: the daemon stops while the first of three goals starts. The two not yet handed
/// to the starter wait in the file and start after the restart; the one in flight is a
/// dropped goal with [`INTERRUPTED`], never started again (decision 6, at most once).
#[tokio::test(flavor = "multi_thread")]
async fn a_drain_cut_short_loses_no_goal_it_had_not_started() {
    let rig = Rig::new();
    let seen = hanging(&rig, "first");
    let project = repo(rig.dir.path(), "app");
    ready_proposal(&rig, &project, Vec::new());
    for text in ["first", "second", "third"] {
        rig.profiles
            .queue_goal(goal(&project, text, false))
            .await
            .unwrap();
    }
    let message = done(
        rig.profiles
            .request(ProfileRequest::Confirm {
                dir: project.clone(),
                shown: None,
            })
            .await,
    );
    assert!(message.ends_with("; starting 3 queued goals"), "{message}");
    until("the first start to begin", || texts(&seen) == ["first"]).await;
    // Goals waiting their turn in a drain are not shown as waiting for a set-up.
    assert!(rig.profiles.queued_goals().is_empty());

    // The crash: a fresh daemon on the files the first one left.
    let fresh = service(rig.dir.path());
    let started = recorder(&fresh, Ok("run-2"));
    fresh.restore().await;
    let token = CancellationToken::new();
    fresh.spawn(token.clone());
    until("the waiting goals to start", || texts(&started).len() == 2).await;
    assert_eq!(texts(&started), ["second", "third"]);
    let status = status(&fresh, &project).await;
    let dropped: Vec<_> = status
        .dropped_goals
        .iter()
        .map(|d| (d.goal.as_str(), d.reason.as_str()))
        .collect();
    assert_eq!(dropped, [("first", INTERRUPTED)]);
    assert!(status.queued.is_empty());
    token.cancel();
}

/// I1, the file itself: a queue file whose `starting` list survived a stop restores as
/// dropped goals, and the starter never gets them; a goal still waiting starts.
#[tokio::test(flavor = "multi_thread")]
async fn a_restored_starting_goal_is_dropped_never_started() {
    let rig = Rig::new();
    let project = repo(rig.dir.path(), "app");
    rig.store_profile(&project);
    let dir = rig.repo_dir(&project);
    std::fs::create_dir_all(&dir).unwrap();
    let waiting = serde_json::to_value(goal(&project, "waiting", false)).unwrap();
    let starting = serde_json::to_value(goal(&project, "in flight", false)).unwrap();
    let file = serde_json::json!({ "goals": [waiting], "dropped": [], "starting": [starting] });
    std::fs::write(dir.join(QUEUE_FILE), file.to_string()).unwrap();

    let fresh = service(rig.dir.path());
    let seen = recorder(&fresh, Ok("run-1"));
    fresh.restore().await;
    let (_, dropped) = fresh.queued_for(&project);
    assert_eq!(dropped.len(), 1, "{dropped:?}");
    assert_eq!(dropped[0].goal, "in flight");
    assert_eq!(dropped[0].reason, INTERRUPTED);
    let token = CancellationToken::new();
    fresh.spawn(token.clone());
    // The drain starts in queue order and ends with the file emptied of goals.
    until("the waiting goal's drain to end", || {
        texts(&seen) == ["waiting"] && queue::load(&dir).is_ok_and(|q| q.starting.is_empty())
    })
    .await;
    let on_file = queue::load(&dir).unwrap();
    assert!(on_file.goals.is_empty(), "{on_file:?}");
    assert_eq!(on_file.dropped.len(), 1, "{on_file:?}");
    assert_eq!(
        texts(&seen),
        ["waiting"],
        "a starting goal was started again"
    );
    token.cancel();
}

/// M5: a take-out whose file write fails starts nothing and leaves no goal waiting
/// behind the stored profile: each is a dropped goal that says why.
#[tokio::test(flavor = "multi_thread")]
async fn a_failed_take_out_drops_the_goals_with_the_error() {
    let rig = Rig::new();
    let seen = recorder(&rig.profiles, Ok("run-1"));
    let project = repo(rig.dir.path(), "app");
    rig.profiles
        .queue_goal(goal(&project, "add a flag", false))
        .await
        .unwrap();
    rig.store_profile(&project);
    // The queue's file cannot be replaced: a non-empty directory stands in its place.
    let file = rig.repo_dir(&project).join(QUEUE_FILE);
    std::fs::remove_file(&file).unwrap();
    std::fs::create_dir_all(file.join("blocker")).unwrap();

    rig.profiles.clone().start_queued(project.clone()).await;
    assert!(crate::lock(&seen).is_empty());
    let (queued, dropped) = rig.profiles.queued_for(&project);
    assert!(queued.is_empty(), "{queued:?}");
    assert_eq!(dropped.len(), 1, "{dropped:?}");
    assert_eq!(dropped[0].goal, "add a flag");
    assert!(
        dropped[0]
            .reason
            .starts_with("could not take the goal out of its queue (")
            && dropped[0].reason.ends_with("); start it again"),
        "{}",
        dropped[0].reason
    );
}
