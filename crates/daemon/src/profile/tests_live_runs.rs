//! Milestone 9.0.6 decision 37: a profile edit waits for the runs live in its project;
//! confirm and detect do not (a goal's onboarding confirms during its own run). A real
//! service and a real git repository (`tests_ready.rs`'s rig); no agent runs.

use std::path::Path;
use std::sync::Arc;

use proto::{ProfileReply, ProfileRequest, ProposalOrigin, ProposalState, RowEdit, RowEditState};

use super::service_requests::LIVE_RUN;
use super::store;
use super::tests_ready::{Rig, record, repo};

/// A `LiveRuns` that names `r-0001` for `project` and nothing elsewhere.
fn one_live_run(rig: &Rig, project: &Path) {
    let project = project.to_path_buf();
    rig.profiles.set_live_runs(Arc::new(move |p: &Path| {
        if p == project {
            vec!["r-0001".to_string()]
        } else {
            Vec::new()
        }
    }));
}

fn edit_of(project: &Path) -> ProfileRequest {
    ProfileRequest::Edit {
        dir: project.to_path_buf(),
        key: "modules".into(),
        value: Some("[\"src/*\"]".into()),
        yes: false,
        unconfined_checks: false,
        anyway: false,
        on_proposal: false,
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn an_edit_during_a_live_run_is_refused() {
    let rig = Rig::new();
    let project = repo(rig.dir.path(), "app");
    rig.store_profile(&project);
    one_live_run(&rig, &project);
    // Milestone 9.10 decision 20: one text, whatever the edit's target.
    let expected = "finish or cancel the run in this repo to change its profile";
    assert_eq!(LIVE_RUN, expected);
    let reply = rig.profiles.request(edit_of(&project)).await;
    assert_eq!(
        reply,
        ProfileReply::Refused {
            message: expected.to_string()
        }
    );
    assert!(
        rig.on_disk(&project).is_none(),
        "a refused edit wrote a proposal"
    );

    // With no live run the edit proceeds as before.
    rig.profiles.set_live_runs(Arc::new(|_: &Path| Vec::new()));
    rig.edit(&project).await;
    assert!(rig.on_disk(&project).is_some());
}

/// Decision 20: an edit of a review proposal, `anyway` too, and a revert of the stored
/// profile's failed edit wait for the live run as well.
#[tokio::test(flavor = "multi_thread")]
async fn every_row_edit_and_a_stored_revert_wait_for_a_live_run() {
    let rig = Rig::new();
    let project = repo(rig.dir.path(), "app");
    let expected = ProfileReply::Refused {
        message: LIVE_RUN.to_string(),
    };
    one_live_run(&rig, &project);
    let dir = rig.repo_dir(&project);
    std::fs::create_dir_all(&dir).unwrap();
    let review = record(&project, ProposalState::Ready, 1_790_000_000);
    store::save_proposal(&dir, &review).unwrap();
    for anyway in [false, true] {
        let mut request = edit_of(&project);
        if let ProfileRequest::Edit {
            anyway: a,
            on_proposal,
            ..
        } = &mut request
        {
            (*a, *on_proposal) = (anyway, true);
        }
        assert_eq!(rig.profiles.request(request).await, expected);
    }
    assert_eq!(rig.on_disk(&project), Some(review));

    let mut failed = record(&project, ProposalState::Ready, 1_790_000_000);
    failed.origin = ProposalOrigin::Edit {
        keys: vec!["check".into()],
    };
    failed.edit = Some(RowEdit {
        key: "check".into(),
        value: Some("false".into()),
        state: RowEditState::Failed {
            reason: "exit 1 after 0s".into(),
            tail: String::new(),
            secs: 0,
        },
    });
    store::save_proposal(&dir, &failed).unwrap();
    let revert = ProfileRequest::RevertEdit {
        dir: project.clone(),
    };
    assert_eq!(rig.profiles.request(revert).await, expected);
    assert_eq!(rig.on_disk(&project), Some(failed));
}

/// Pinning: only `Edit` waits for live runs.
#[tokio::test(flavor = "multi_thread")]
async fn confirm_and_detect_ignore_live_runs() {
    let rig = Rig::new();
    let project = repo(rig.dir.path(), "app");
    rig.store_profile(&project);
    rig.edit(&project).await;
    one_live_run(&rig, &project);
    let confirm = ProfileRequest::Confirm {
        dir: project.clone(),
        shown: None,
    };
    let reply = rig.profiles.request(confirm).await;
    assert!(matches!(reply, ProfileReply::Done { .. }), "{reply:?}");

    let detect = ProfileRequest::Detect {
        dir: project.clone(),
        trust_project: false,
        unconfined_checks: true,
    };
    let reply = rig.profiles.request(detect).await;
    if let ProfileReply::Refused { message } = &reply {
        assert!(!message.contains("is live in"), "{message}");
    }
    // Whatever the detection started (its agent does not exist), stop it.
    let _ = rig
        .profiles
        .request(ProfileRequest::Reject { dir: project })
        .await;
}
