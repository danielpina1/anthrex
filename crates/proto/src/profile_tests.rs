//! Milestone 9.10: the simple repository profile's wire types (queued goals, row
//! edits, set-up progress).

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
