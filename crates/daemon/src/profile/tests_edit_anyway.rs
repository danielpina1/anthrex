//! Milestone 9.10 final review fixes (daemon) M4: a **Save anyway** of the stored
//! profile keeps the edit's ✗, and `verified_at` stays the time the stored profile was
//! last verified, never the save's. A real service, real verification commands and a
//! real git repository; no agent runs.

use std::collections::BTreeMap;

use proto::{
    CommandCheck, ProfileMeta, ProfileVerification, ProposalOrigin, ProposalState, RepoProfile,
    RowEdit, RowEditState,
};

use super::service_run::confirm_record;
use super::store::{self, Stored};
use super::tests_edit::{edit, failed_check_edit, stored, with};
use super::tests_queue_service::{done, status};
use super::tests_ready::{Rig, record, repo};

/// The check ran and failed: the stored ✗ is the run's, and `verified_at` is the
/// stored profile's last verification (`Rig::store_profile`'s `confirmed_at`, 1).
#[tokio::test(flavor = "multi_thread")]
async fn save_anyway_keeps_the_last_verified_time() {
    let rig = Rig::new();
    let project = repo(rig.dir.path(), "app");
    failed_check_edit(&rig, &project).await;
    let anyway = with(edit(&project, "check", Some("false")), true, false);
    done(rig.profiles.request(anyway).await);

    let (_, meta) = stored(&rig, &project).unwrap();
    let check = meta.verification.and_then(|v| v.check).expect("its ✗ kept");
    assert_eq!((check.command.as_str(), check.ok), ("false", false));
    assert_eq!(status(&rig.profiles, &project).await.verified_at, Some(1));
}

/// Verification could not run (`Stop::Failed`), so the row edit's record has no
/// verification: the previous one is kept, with the edited key's ✗ in its slot, and
/// its time.
#[test]
fn a_save_anyway_with_no_verification_keeps_the_previous_one() {
    let dir = tempfile::tempdir().unwrap();
    let project = dir.path().join("app");
    let previous = ProfileVerification {
        at: 7,
        confined: true,
        setup: None,
        check: Some(CommandCheck {
            command: "true".into(),
            ok: true,
            code: Some(0),
            timed_out: false,
            secs: 1,
            tail: String::new(),
        }),
        single_test: None,
        build_check: None,
        module_graph: None,
        module_test: None,
        module_tests: None,
        toolchain_id: None,
    };
    let meta = ProfileMeta {
        confirmed_at: 5,
        report: None,
        verification: Some(previous),
        fingerprint: BTreeMap::new(),
        edited_keys: Vec::new(),
        project: Some(project.clone()),
    };
    let old = RepoProfile {
        check: Some("true".into()),
        ..RepoProfile::default()
    };
    store::save(dir.path(), &old, &meta).unwrap();
    let mut saved = record(&project, ProposalState::Ready, 9);
    saved.origin = ProposalOrigin::Edit {
        keys: vec!["check".into()],
    };
    saved.profile = Some(RepoProfile {
        check: Some("false".into()),
        ..RepoProfile::default()
    });
    saved.verification = None;
    saved.edit = Some(RowEdit {
        key: "check".into(),
        value: Some("false".into()),
        state: RowEditState::Failed {
            reason: "verification could not run".into(),
            tail: "no checkout".into(),
            secs: 0,
        },
    });

    confirm_record(dir.path(), &project, &saved).unwrap();
    let Stored::Found { meta, .. } = store::load(dir.path()) else {
        panic!("not stored");
    };
    let v = meta.verification.expect("a verification is kept");
    assert_eq!(v.at, 7);
    assert!(v.confined);
    let check = v.check.expect("the edited key's ✗");
    assert_eq!(
        (check.command.as_str(), check.ok, check.tail.as_str()),
        ("false", false, "no checkout")
    );
}
