//! Milestone 9.10 task M9.10.4, fix round 1: the queue's ordering against `reject`,
//! a crash and an unreadable file at restore, a queue that outlives the detection's
//! later phases, a drain that starts a goal once, and a detection's `Ready` that
//! stores for a queued `--yes` goal. A real service on a scratch data directory and
//! real git repositories; no agent runs (Claude and Codex are paths that do not exist).

use std::sync::mpsc;

use proto::{ProfileReply, ProfileRequest, ProposalState, RepoProfile, SetupState};

use super::queue::{self, QUEUE_FILE};
use super::service::RESTART_REASON;
use super::service_run::Job;
use super::store::{self, Stored};
use super::tests_queue::goal;
use super::tests_queue_service::{done, ready_proposal, recorder, until, write_queue};
use super::tests_ready::{Rig, record, repo, service};
use crate::run::driver::unix_now;

/// I1: a goal queued while a `reject` holds `writes` is queued after it, so the reject
/// does not drop it. The reject is frozen inside its hold by the table's lock, held
/// from a plain thread, and the goal's `queue_goal` is polled once so it waits on
/// `writes` behind the reject (tokio's mutex is first come, first served).
#[tokio::test(flavor = "multi_thread")]
async fn a_goal_queued_while_a_reject_runs_is_kept() {
    let rig = Rig::new();
    let seen = recorder(&rig.profiles, Ok("run-1"));
    let project = repo(rig.dir.path(), "app");
    ready_proposal(&rig, &project, Vec::new());

    let (held_tx, held_rx) = mpsc::channel();
    let (release_tx, release_rx) = mpsc::channel::<()>();
    let profiles = rig.profiles.clone();
    let holder = std::thread::spawn(move || {
        let _table = crate::lock(&profiles.table);
        held_tx.send(()).unwrap();
        release_rx.recv().unwrap();
    });
    held_rx.recv().unwrap();
    let reject = tokio::spawn({
        let (profiles, dir) = (rig.profiles.clone(), project.clone());
        async move { profiles.request(ProfileRequest::Reject { dir }).await }
    });
    // Nothing else in this test takes `writes`, so a refused try is the reject's hold.
    until("the reject to hold writes", || {
        rig.profiles.writes.try_lock().is_err()
    })
    .await;
    let queued = rig
        .profiles
        .queue_goal(goal(&project, "after the reject", false));
    tokio::pin!(queued);
    tokio::select! {
        biased;
        early = &mut queued => panic!("queued inside the reject's hold: {early:?}"),
        () = std::future::ready(()) => {}
    }
    release_tx.send(()).unwrap();
    holder.join().unwrap();

    queued.await.unwrap();
    let message = done(reject.await.unwrap());
    assert!(
        message.starts_with("rejected the proposal for ") && !message.contains("dropped"),
        "{message}"
    );
    let listed = rig.profiles.queued_goals();
    assert_eq!(listed.len(), 1, "{listed:?}");
    assert_eq!(listed[0].goal, "after the reject");
    assert!(rig.profiles.queued_for(&project).1.is_empty());
    assert!(crate::lock(&seen).is_empty());
}

/// I1: a queue left with no proposal and no stored profile (a crash between a
/// proposal's delete and its queue's drop, or before a goal's set-up wrote its first
/// record) is not shown setting up forever: restore fails its set-up, so it can be
/// retried.
#[tokio::test(flavor = "multi_thread")]
async fn a_restored_queue_with_no_set_up_is_failed() {
    let rig = Rig::new();
    let project = repo(rig.dir.path(), "app");
    write_queue(&rig, &project, vec![goal(&project, "add a flag", false)]);

    let fresh = service(rig.dir.path());
    fresh.restore().await;
    let queued = fresh.queued_goals();
    assert_eq!(queued.len(), 1, "{queued:?}");
    let failed = ProposalState::Failed {
        reason: RESTART_REASON.to_string(),
    };
    assert_eq!(
        queued[0].setup,
        SetupState::Failed {
            reason: RESTART_REASON.to_string()
        }
    );
    let on_disk = store::load_proposal(&rig.repo_dir(&project)).unwrap();
    assert_eq!(on_disk.map(|record| record.state), Some(failed));
}

/// I2: an unreadable queue file is set aside, not overwritten, and the loss is shown.
#[tokio::test(flavor = "multi_thread")]
async fn restore_sets_an_unreadable_queue_aside() {
    let rig = Rig::new();
    let project = repo(rig.dir.path(), "app");
    ready_proposal(&rig, &project, Vec::new());
    let repo_dir = rig.repo_dir(&project);
    std::fs::write(repo_dir.join(QUEUE_FILE), "not json").unwrap();

    let fresh = service(rig.dir.path());
    fresh.restore().await;
    let aside: Vec<_> = std::fs::read_dir(&repo_dir)
        .unwrap()
        .filter_map(Result::ok)
        .filter(|entry| {
            let name = entry.file_name().to_string_lossy().into_owned();
            name.starts_with("queued_goals.json.unreadable-")
        })
        .collect();
    assert_eq!(aside.len(), 1, "the file is kept aside once");
    assert_eq!(
        std::fs::read_to_string(aside[0].path()).unwrap(),
        "not json"
    );
    let dropped = fresh.queued_for(&project).1;
    assert_eq!(dropped.len(), 1, "{dropped:?}");
    assert_eq!(dropped[0].goal, "(unreadable queue)");
    assert!(
        dropped[0].reason.contains("queued_goals.json.unreadable-"),
        "{}",
        dropped[0].reason
    );
    // The record of the loss is itself a readable queue.
    assert_eq!(queue::load(&repo_dir).unwrap().dropped, dropped);
}

/// Review focus 1: a goal queued while the scout runs survives every later write of
/// the job's own clone of the record (decision 4's reason for a file of its own).
#[tokio::test(flavor = "multi_thread")]
async fn a_goal_queued_while_scouting_survives_the_later_phases() {
    let rig = Rig::new();
    let project = repo(rig.dir.path(), "app");
    let (generation, _token) = rig.profiles.register(&project).expect("nothing runs");
    let job_record = |state| record(&project, state, 1_790_000_000);
    assert!(
        rig.profiles
            .save_if_current(generation, &job_record(ProposalState::Scouting))
            .await
    );
    rig.profiles
        .queue_goal(goal(&project, "add a flag", false))
        .await
        .unwrap();
    for state in [ProposalState::Verifying, ProposalState::Ready] {
        assert!(
            rig.profiles
                .save_if_current(generation, &job_record(state))
                .await
        );
    }
    let on_disk = queue::load(&rig.repo_dir(&project)).unwrap();
    assert_eq!(on_disk.goals.len(), 1);
    let listed = rig.profiles.queued_goals();
    assert_eq!(listed.len(), 1, "{listed:?}");
    assert_eq!(listed[0].setup, SetupState::NeedsReview);
}

/// Review focus 2: two confirms and a later drain start the goal exactly once.
#[tokio::test(flavor = "multi_thread")]
async fn two_confirms_start_a_goal_once() {
    let rig = Rig::new();
    let seen = recorder(&rig.profiles, Ok("run-1"));
    let project = repo(rig.dir.path(), "app");
    ready_proposal(&rig, &project, Vec::new());
    rig.profiles
        .queue_goal(goal(&project, "add a flag", false))
        .await
        .unwrap();
    let confirm = || {
        rig.profiles.request(ProfileRequest::Confirm {
            dir: project.clone(),
            shown: None,
        })
    };
    let (first, second) = tokio::join!(confirm(), confirm());
    let stored = [&first, &second]
        .iter()
        .filter(|reply| matches!(reply, ProfileReply::Done { .. }))
        .count();
    assert_eq!(stored, 1, "{first:?} {second:?}");
    until("the goal to start", || !crate::lock(&seen).is_empty()).await;
    // A drain racing the confirm's, as a restart's would, finds the queue empty.
    rig.profiles.clone().start_queued(project.clone()).await;
    assert_eq!(crate::lock(&seen).len(), 1);
    assert!(rig.profiles.queued_goals().is_empty());
}

/// Mutation M4: a detection's own `Ready` (`verify_phase`) stores the proposal for a
/// queued `--yes` goal and starts it. A real verification of `check = "true"`.
#[tokio::test(flavor = "multi_thread")]
async fn a_detections_ready_stores_for_a_queued_yes_goal() {
    let rig = Rig::new();
    let seen = recorder(&rig.profiles, Ok("run-1"));
    let project = repo(rig.dir.path(), "app");
    rig.profiles
        .queue_goal(goal(&project, "add a flag", true))
        .await
        .unwrap();
    let pre = rig.profiles.preflight(project.clone()).await.unwrap();
    let (generation, token) = rig.profiles.register(&project).expect("nothing runs");
    let mut record = record(&project, ProposalState::Preparing, unix_now());
    record.profile = None;
    record.proposed = Some(RepoProfile {
        check: Some("true".into()),
        ..Default::default()
    });
    record.base_sha = pre.base_sha.clone();
    let job = Job {
        generation,
        token,
        pre,
        record,
        codex_config: Vec::new(),
        route: None,
    };
    rig.profiles.clone().verify_in_background(job).await;
    assert!(
        matches!(store::load(&rig.repo_dir(&project)), Stored::Found { .. }),
        "the ready proposal was stored for the --yes goal"
    );
    until("the goal to start", || crate::lock(&seen).len() == 1).await;
}
