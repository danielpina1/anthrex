//! Milestone 9.10: the simple repository profile's wire types (queued goals, row
//! edits, set-up progress).

use std::path::PathBuf;

use serde::Serialize;
use serde::de::DeserializeOwned;

use crate::run_wire::{ProfileRequest, RunReply};
use crate::*;

fn both_ways<T>(value: &T)
where
    T: Serialize + DeserializeOwned + PartialEq + std::fmt::Debug,
{
    let packed = rmp_serde::to_vec_named(value).unwrap();
    let back: T = rmp_serde::from_slice(&packed).unwrap();
    assert_eq!(&back, value, "MessagePack");
    let json = serde_json::to_string(value).unwrap();
    let back: T = serde_json::from_str(&json).unwrap();
    assert_eq!(&back, value, "JSON");
}

fn queued(setup: SetupState) -> QueuedGoalInfo {
    QueuedGoalInfo {
        id: "q-1700000000000000000-1".into(),
        project: PathBuf::from("/work/app"),
        goal: "add a --json flag".into(),
        queued_at: 1_700_000_000,
        yes: true,
        trust_project: true,
        unconfined_checks: false,
        setup,
    }
}

fn record(edit: Option<RowEdit>) -> ProposalRecord {
    ProposalRecord {
        project: PathBuf::from("/work/app"),
        state: ProposalState::Ready,
        origin: ProposalOrigin::Goal,
        started_at: 1,
        updated_at: 2,
        base_sha: "abc".into(),
        scout_id: None,
        window_id: None,
        profile: None,
        verification: None,
        dropped: Vec::new(),
        proposed: None,
        trusted_project: Vec::new(),
        unconfined_checks: false,
        auto_confirm: false,
        edit,
    }
}

fn status(full: bool) -> ProfileStatus {
    ProfileStatus {
        project: PathBuf::from("/work/app"),
        repo_dir: PathBuf::from("/tmp/data/repos/app-1"),
        source: ProfileSource::None,
        confirmed_at: None,
        stale: Vec::new(),
        unparseable: None,
        proposal: None,
        scout: None,
        verify_confined: false,
        queued: if full {
            vec![queued(SetupState::Reading)]
        } else {
            Vec::new()
        },
        checking: full.then_some(CheckProgress { done: 2, total: 4 }),
        verified_at: full.then_some(1_700_000_100),
        unreadable_text: full.then(|| "test = [".to_string()),
        dropped_goals: if full {
            vec![DroppedGoal {
                goal: "g".into(),
                reason: "the repository is gone".into(),
                at: 5,
            }]
        } else {
            Vec::new()
        },
    }
}

fn snapshot(queued_goals: Vec<QueuedGoalInfo>) -> RunsSnapshot {
    RunsSnapshot {
        revision: 1,
        runs: Vec::new(),
        now: 0,
        proposals: Vec::new(),
        idle_orchestrators: Vec::new(),
        queued_goals,
    }
}

/// Decision 2: every new message, variant and field survives JSON and MessagePack.
#[test]
fn every_new_profile_type_round_trips() {
    for setup in [
        SetupState::Reading,
        SetupState::Checking { progress: None },
        SetupState::Checking {
            progress: Some(CheckProgress { done: 1, total: 3 }),
        },
        SetupState::NeedsReview,
        SetupState::Failed {
            reason: "no confinement".into(),
        },
    ] {
        both_ways(&queued(setup));
    }
    for state in [
        RowEditState::Verifying,
        RowEditState::Failed {
            reason: "exit 1".into(),
            tail: "boom".into(),
            secs: 3,
        },
    ] {
        both_ways(&RowEdit {
            key: "test".into(),
            value: Some("cargo test".into()),
            state: state.clone(),
        });
        both_ways(&RowEdit {
            key: "test".into(),
            value: None,
            state: state.clone(),
        });
        // Task 1-2 review minor 4: each state inside a `ProposalRecord` too.
        both_ways(&record(Some(RowEdit {
            key: "lint".into(),
            value: Some("x".into()),
            state,
        })));
    }
    both_ways(&DroppedGoal {
        goal: "g".into(),
        reason: "r".into(),
        at: 9,
    });
    both_ways(&status(true));
    both_ways(&status(false));
    for (anyway, on_proposal) in [(false, false), (true, false), (false, true), (true, true)] {
        both_ways(&ProfileRequest::Edit {
            dir: "/work/app".into(),
            key: "test".into(),
            value: Some("cargo test".into()),
            yes: false,
            unconfined_checks: false,
            anyway,
            on_proposal,
        });
    }
    both_ways(&ProfileRequest::RevertEdit {
        dir: "/work/app".into(),
    });
    both_ways(&RunReply::Queued {
        goal_id: "q-1".into(),
        project: "/work/app".into(),
        message: "waits for the profile".into(),
        request_id: Some(4),
    });
    both_ways(&snapshot(vec![queued(SetupState::NeedsReview)]));
}

/// A protocol-19/20 peer's JSON, without the new fields, still decodes.
#[test]
fn new_profile_fields_default_when_absent() {
    // Task 1-2 review minor 3: literal protocol-20 JSON, so the old field names are
    // pinned too, not only the new fields' defaults.
    let rec: ProposalRecord = serde_json::from_str(
        r#"{"project":"/work/app","state":{"state":"ready"},"origin":{"origin":"detect"},
            "started_at":1,"updated_at":2,"base_sha":"abc","scout_id":null,"window_id":null,
            "profile":null,"verification":null,"dropped":[],"proposed":null,
            "trusted_project":[],"unconfined_checks":false,"auto_confirm":false}"#,
    )
    .unwrap();
    let mut expected = record(None);
    expected.origin = ProposalOrigin::Detect;
    assert_eq!(rec, expected);

    let old: ProfileStatus = serde_json::from_str(
        r#"{"project":"/work/app","repo_dir":"/tmp/data/repos/app-1","source":"none",
            "confirmed_at":null,"stale":[],"unparseable":null,"proposal":null,"scout":null,
            "verify_confined":false}"#,
    )
    .unwrap();
    assert_eq!(old, status(false));

    let edit: ProfileRequest = serde_json::from_str(
        r#"{"Edit":{"dir":"/work/app","key":"test","value":null,"yes":false,"unconfined_checks":false}}"#,
    )
    .unwrap();
    assert_eq!(
        edit,
        ProfileRequest::Edit {
            dir: "/work/app".into(),
            key: "test".into(),
            value: None,
            yes: false,
            unconfined_checks: false,
            anyway: false,
            on_proposal: false,
        }
    );

    let snap: RunsSnapshot = serde_json::from_str(r#"{"revision":1,"runs":[]}"#).unwrap();
    assert!(snap.queued_goals.is_empty());
}

/// Empty, false or `None` new fields are left out; an `Edit` always says what it asks.
#[test]
fn empty_new_fields_are_left_out() {
    let json = serde_json::to_string(&record(None)).unwrap()
        + &serde_json::to_string(&status(false)).unwrap()
        + &serde_json::to_string(&snapshot(Vec::new())).unwrap();
    for field in [
        "edit",
        "queued",
        "checking",
        "verified_at",
        "unreadable_text",
        "dropped_goals",
        "queued_goals",
    ] {
        assert!(!json.contains(&format!("\"{field}\"")), "{field} in {json}");
    }
    let edit = serde_json::to_string(&ProfileRequest::Edit {
        dir: "/work/app".into(),
        key: "test".into(),
        value: None,
        yes: false,
        unconfined_checks: false,
        anyway: false,
        on_proposal: false,
    })
    .unwrap();
    assert!(edit.contains("\"anyway\":false"), "{edit}");
    assert!(edit.contains("\"on_proposal\":false"), "{edit}");
}

#[test]
fn setup_state_tags_are_snake_case() {
    assert_eq!(
        serde_json::to_string(&SetupState::NeedsReview).unwrap(),
        r#"{"state":"needs_review"}"#
    );
    assert_eq!(
        serde_json::to_string(&SetupState::Reading).unwrap(),
        r#"{"state":"reading"}"#
    );
    let failed = serde_json::to_string(&SetupState::Failed { reason: "x".into() }).unwrap();
    assert_eq!(failed, r#"{"state":"failed","reason":"x"}"#);
    assert_eq!(
        serde_json::to_string(&RowEditState::Verifying).unwrap(),
        r#"{"state":"verifying"}"#
    );
}

/// `tagged` and `request_id` each have the `Queued` arm.
#[test]
fn queued_has_its_request_id() {
    let reply = RunReply::Queued {
        goal_id: "q-1".into(),
        project: "/work/app".into(),
        message: "m".into(),
        request_id: None,
    };
    assert_eq!(reply.request_id(), None);
    assert_eq!(reply.clone().tagged(Some(7)).request_id(), Some(7));
    assert_eq!(reply.tagged(None).request_id(), None);
}

/// M8b.11 review (I4, m3): a meta written before `project` existed, and a `Confirm`
/// without `shown`, still read.
#[test]
fn meta_project_and_confirm_shown_default_when_absent() {
    let meta: crate::ProfileMeta = serde_json::from_str(
        r#"{"confirmed_at":1,"report":null,"verification":null,"fingerprint":{},"edited_keys":[]}"#,
    )
    .unwrap();
    assert_eq!(meta.project, None);
    let confirm: crate::ProfileRequest =
        serde_json::from_str(r#"{"Confirm":{"dir":"/work/app"}}"#).unwrap();
    assert_eq!(
        confirm,
        crate::ProfileRequest::Confirm {
            dir: "/work/app".into(),
            shown: None
        }
    );
}
