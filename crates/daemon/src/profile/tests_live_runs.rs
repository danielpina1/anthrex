//! Milestone 9.0.6 decision 37: a profile edit waits for the runs live in its project;
//! confirm and detect do not (a goal's onboarding confirms during its own run). A real
//! service and a real git repository (`tests_ready.rs`'s rig); no agent runs.

use std::path::Path;
use std::sync::Arc;

use proto::{ProfileReply, ProfileRequest};

use super::service_requests::live_run;
use super::tests_ready::{Rig, repo};

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
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn an_edit_during_a_live_run_is_refused() {
    let rig = Rig::new();
    let project = repo(rig.dir.path(), "app");
    rig.store_profile(&project);
    one_live_run(&rig, &project);
    let reply = rig.profiles.request(edit_of(&project)).await;
    let expected = format!(
        "run r-0001 is live in {}; edit the profile once it finishes (runs keep the profile they started with)",
        project.display()
    );
    assert_eq!(live_run("r-0001", &project), expected);
    assert_eq!(reply, ProfileReply::Refused { message: expected });
    assert!(
        rig.on_disk(&project).is_none(),
        "a refused edit wrote a proposal"
    );

    // With no live run the edit proceeds as before.
    rig.profiles.set_live_runs(Arc::new(|_: &Path| Vec::new()));
    rig.edit(&project).await;
    assert!(rig.on_disk(&project).is_some());
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
