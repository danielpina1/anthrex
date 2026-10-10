//! Milestone 9.10 task M9.10.6, a row edit of a review proposal (decision 15): the
//! proposal stays `Ready` while the edit is checked, keeps its old value on ✗, and still
//! needs **Use this**. Helpers and the rig as in `tests_edit.rs`; no agent runs.

use proto::{CommandCheck, ProfileRequest, ProposalOrigin, ProposalState, RowEdit, RowEditState};

use super::tests_edit::{
    edit, edit_state, failed, put, refused, revert, review, stored, stored_check, wait, with,
    work_ended,
};
use super::tests_queue::goal;
use super::tests_queue_service::{done, recorder};
use super::tests_ready::{Rig, record, repo, service};

#[tokio::test(flavor = "multi_thread")]
async fn an_edit_on_a_proposal_changes_only_the_proposal() {
    let rig = Rig::new();
    let project = repo(rig.dir.path(), "app");
    let before = review(&rig, &project, ProposalOrigin::Detect, false);
    let request = with(edit(&project, "check", Some("true && true")), false, true);
    let message = done(rig.profiles.request(request).await);
    assert_eq!(
        message,
        "proposed: check = true && true; checking it in the proposal (anthrex profile status)"
    );
    wait("the edit to be saved in the proposal", || {
        let record = rig.on_disk(&project).expect("the proposal stays");
        assert_eq!(
            record.state,
            ProposalState::Ready,
            "never anything but Ready"
        );
        record.edit.is_none()
    })
    .await;
    let after = rig.on_disk(&project).unwrap();
    let profile = after.profile.unwrap();
    assert_eq!(profile.check.as_deref(), Some("true && true"));
    assert_eq!(
        after.proposed, before.proposed,
        "the scout's findings are kept"
    );
    assert_eq!(profile.delivery, before.profile.unwrap().delivery);
    let check = after.verification.and_then(|v| v.check).unwrap();
    assert_eq!((check.command.as_str(), check.ok), ("true && true", true));
    assert!(stored(&rig, &project).is_none(), "nothing stored");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_failing_edit_on_a_proposal_keeps_the_old_value() {
    let rig = Rig::new();
    let project = repo(rig.dir.path(), "app");
    let before = review(&rig, &project, ProposalOrigin::Detect, true);
    let request = with(edit(&project, "check", Some("false")), false, true);
    done(rig.profiles.request(request).await);
    wait("the edit's ✗", || failed(edit_state(&rig, &project))).await;
    let held = rig.on_disk(&project).unwrap();
    assert_eq!(held.state, ProposalState::Ready);
    assert_eq!(held.profile, before.profile, "the old value is kept");
    assert_eq!(held.verification, before.verification);
    assert_eq!(held.dropped, before.dropped);
    let Some(RowEditState::Failed { tail, secs, .. }) = held.edit.map(|e| e.state) else {
        unreachable!()
    };

    let anyway = with(edit(&project, "check", Some("false")), true, true);
    let message = done(rig.profiles.request(anyway).await);
    assert_eq!(
        message,
        "proposed: check = false; saved in the proposal although its check failed"
    );
    let saved = rig.on_disk(&project).unwrap();
    assert_eq!(saved.edit, None);
    assert_eq!(saved.profile.unwrap().check.as_deref(), Some("false"));
    // R22: the ✗ check rebuilt from the row edit's `tail` and `secs`.
    let check = saved.verification.and_then(|v| v.check).unwrap();
    let rebuilt = CommandCheck {
        command: "false".into(),
        ok: false,
        code: None,
        timed_out: false,
        secs,
        tail,
    };
    assert_eq!(check, rebuilt);

    // A second try, reverted.
    let request = with(edit(&project, "check", Some("exit 3")), false, true);
    done(rig.profiles.request(request).await);
    wait("the second ✗", || failed(edit_state(&rig, &project))).await;
    let message = done(rig.profiles.request(revert(&project)).await);
    assert_eq!(message, "reverted check; the proposal is unchanged");
    let reverted = rig.on_disk(&project).unwrap();
    assert_eq!(reverted.edit, None);
    assert_eq!(reverted.profile.unwrap().check.as_deref(), Some("false"));
    assert!(stored(&rig, &project).is_none());
}

#[tokio::test(flavor = "multi_thread")]
async fn a_non_command_edit_on_a_proposal_is_written_at_once() {
    let rig = Rig::new();
    let project = repo(rig.dir.path(), "app");
    let before = review(&rig, &project, ProposalOrigin::Goal, false);
    let request = with(edit(&project, "source", Some("[\"src/**\"]")), false, true);
    let message = done(rig.profiles.request(request).await);
    assert_eq!(
        message,
        "proposed: source = [\"src/**\"]; saved in the proposal"
    );
    let after = rig.on_disk(&project).unwrap();
    assert_eq!((after.state, after.edit), (ProposalState::Ready, None));
    let profile = after.profile.unwrap();
    assert_eq!(profile.source, ["src/**"]);
    assert_eq!(profile.check, before.profile.unwrap().check);
    assert_eq!(after.verification, before.verification);
    assert!(stored(&rig, &project).is_none());
}

#[tokio::test(flavor = "multi_thread")]
async fn an_edit_on_a_proposal_needs_a_ready_review_proposal() {
    let rig = Rig::new();
    let project = repo(rig.dir.path(), "app");
    let request = || with(edit(&project, "check", Some("true && true")), false, true);
    let none = format!(
        "no proposal to edit for {}; run anthrex profile detect",
        project.display()
    );
    assert_eq!(refused(rig.profiles.request(request()).await), none);
    // An edit of the stored profile is not a review proposal.
    let mut edit_record = review(&rig, &project, ProposalOrigin::Detect, false);
    edit_record.origin = ProposalOrigin::Edit {
        keys: vec!["check".into()],
    };
    put(&rig, &edit_record);
    assert_eq!(refused(rig.profiles.request(request()).await), none);
    // A review proposal not ready yet: `ready_profile`'s text.
    put(
        &rig,
        &record(&project, ProposalState::Scouting, 1_790_000_000),
    );
    assert_eq!(
        refused(rig.profiles.request(request()).await),
        format!(
            "the proposal for {} is not ready yet (state scouting); see anthrex profile status",
            project.display()
        )
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn use_this_waits_for_a_proposal_edit_being_checked() {
    let rig = Rig::new();
    let project = repo(rig.dir.path(), "app");
    review(&rig, &project, ProposalOrigin::Detect, false);
    // A check that takes 2 s, so the confirm below lands while it runs.
    let request = with(
        edit(&project, "check", Some("sleep 2 && true")),
        false,
        true,
    );
    done(rig.profiles.request(request).await);
    let confirm = ProfileRequest::Confirm {
        dir: project.clone(),
        shown: None,
    };
    // Fix round M2 (the controller's ruling over the brief's `already_running` text).
    assert_eq!(
        refused(rig.profiles.request(confirm.clone()).await),
        "the edit of check is still being checked; wait for it"
    );
    // Nor does a `--yes` goal's store take it (decision 9's `use_ready`).
    assert_eq!(
        rig.profiles.use_ready(&project).await,
        Err("the edit of check is still being checked; wait for it".to_string())
    );
    wait("the check to end", || edit_state(&rig, &project).is_none()).await;
    work_ended(&rig, &project).await;
    done(rig.profiles.request(confirm).await);
    assert_eq!(
        stored_check(&rig, &project).as_deref(),
        Some("sleep 2 && true")
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn an_unset_is_an_edit() {
    let rig = Rig::new();
    let project = repo(rig.dir.path(), "app");
    let mut ready = review(&rig, &project, ProposalOrigin::Detect, false);
    if let Some(profile) = ready.profile.as_mut() {
        profile.single_test = Some("sh t.sh {test}".into());
    }
    put(&rig, &ready);
    let request = with(edit(&project, "single_test", None), false, true);
    let message = done(rig.profiles.request(request).await);
    assert_eq!(
        message,
        "proposed: single_test = (unset); checking it in the proposal (anthrex profile status)"
    );
    wait("the unset to be saved", || {
        rig.on_disk(&project)
            .is_some_and(|r| r.edit.is_none() && r.profile.unwrap().single_test.is_none())
    })
    .await;
}

#[tokio::test(flavor = "multi_thread")]
async fn an_edited_proposal_with_a_yes_goal_still_needs_use_this() {
    let rig = Rig::new();
    let seen = recorder(&rig.profiles, Ok("run-1"));
    let project = repo(rig.dir.path(), "app");
    review(&rig, &project, ProposalOrigin::Detect, false);
    rig.profiles
        .queue_goal(goal(&project, "add a flag", true))
        .await
        .unwrap()
        .expect("queued");
    let request = with(edit(&project, "check", Some("true && true")), false, true);
    done(rig.profiles.request(request).await);
    wait("the edit to be saved in the proposal", || {
        rig.on_disk(&project).is_some_and(|r| {
            r.edit.is_none() && r.profile.unwrap().check.as_deref() == Some("true && true")
        })
    })
    .await;
    let after = rig.on_disk(&project).unwrap();
    assert_eq!(after.state, ProposalState::Ready);
    assert!(stored(&rig, &project).is_none(), "it waits for Use this");
    assert!(crate::lock(&seen).is_empty(), "no goal started");
    assert_eq!(rig.profiles.queued_goals().len(), 1);
}

/// A row edit the restart interrupted is failed, so it can be saved anyway, reverted or
/// edited again, never shown `checking…` forever.
#[tokio::test(flavor = "multi_thread")]
async fn restore_fails_a_row_edit_the_restart_interrupted() {
    let rig = Rig::new();
    let project = repo(rig.dir.path(), "app");
    let mut checking = review(&rig, &project, ProposalOrigin::Detect, false);
    checking.edit = Some(RowEdit {
        key: "check".into(),
        value: Some("true && true".into()),
        state: RowEditState::Verifying,
    });
    put(&rig, &checking);
    let profiles = service(rig.dir.path());
    profiles.restore().await;
    let restored = rig.on_disk(&project).unwrap();
    assert_eq!(restored.state, ProposalState::Ready);
    assert_eq!(
        restored.edit.map(|e| e.state),
        Some(RowEditState::Failed {
            reason: super::service::EDIT_RESTART_REASON.to_string(),
            tail: String::new(),
            secs: 0,
        })
    );
}
