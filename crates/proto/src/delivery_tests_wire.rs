//! Moved from `delivery_tests.rs` (move only): the variant-order helpers other test
//! modules share (`crate::delivery::tests::{tagged_names, variant_names, variant_at}`)
//! and the appended-variants test.

use super::*;

/// The variants a tagged enum declares, in order, from serde's "expected one of" list.
pub(crate) fn tagged_names<T: DeserializeOwned + std::fmt::Debug>(tag: &str) -> Vec<String> {
    let error = serde_json::from_str::<T>(&format!(r#"{{"{tag}":"no_such_variant"}}"#))
        .unwrap_err()
        .to_string();
    let list = error
        .split_once("expected one of ")
        .map(|(_, rest)| rest)
        .unwrap_or_else(|| panic!("unexpected error: {error}"));
    list.split(", ")
        .map(|name| {
            name.trim_matches(|c: char| c == '`' || !c.is_ascii())
                .to_string()
        })
        .map(|name| name.split('`').next().unwrap_or(&name).to_string())
        .collect()
}

/// The variants an externally tagged enum declares, in order.
pub(crate) fn variant_names<T: DeserializeOwned + std::fmt::Debug>() -> Vec<String> {
    let error = serde_json::from_str::<T>(r#""NoSuchVariant""#)
        .unwrap_err()
        .to_string();
    let list = error
        .split_once("expected one of ")
        .map(|(_, rest)| rest)
        .unwrap_or_else(|| panic!("unexpected error: {error}"));
    list.split(", ")
        .map(|name| {
            let name = name.trim_start_matches('`');
            name.split('`').next().unwrap_or(name).to_string()
        })
        .collect()
}

/// `{index: payload}` in MessagePack: a struct variant addressed by its index.
pub(crate) fn variant_at<T: DeserializeOwned>(index: u8, payload: &serde_json::Value) -> Option<T> {
    assert!(index < 0x80, "a positive fixint");
    let mut bytes = vec![0x81, index];
    bytes.extend(rmp_serde::to_vec_named(payload).unwrap());
    rmp_serde::from_slice(&bytes).ok()
}

#[test]
fn appended_variants_keep_their_indices() {
    let names = tagged_names::<PlanEdit>("op");
    // Milestone 9.3 appends `iterate` (`rounds_tests.rs`).
    assert_eq!(
        names[names.len() - 4..],
        ["message", "refresh", "reply_comment", "iterate"],
        "{names:?}"
    );
    let names = tagged_names::<HoldKind>("kind");
    assert_eq!(names, ["promotion", "epic", "fix"]);
    let names = tagged_names::<HistoryLine>("type");
    assert_eq!(
        names,
        [
            "task",
            "run",
            "revert",
            "role_route",
            "tier",
            "flaky",
            "bisect",
            "stage",
            // Milestone 9.3 appends `round` (`rounds_tests.rs`).
            "round"
        ]
    );
    let names = variant_names::<RunRequest>();
    // Milestone 9.3 appends `Iterate` after `Watch`, and 9.5 `McpReady` after it.
    assert_eq!(
        names[names.len() - 6..names.len() - 1],
        ["TaskDetail", "Settings", "Deliver", "Watch", "Iterate"],
        "{names:?}"
    );
    let n = names.len() as u8 - 2;
    assert_eq!(
        variant_at::<RunRequest>(n - 2, &serde_json::json!({"run_id": "r1", "stage": 2})),
        Some(RunRequest::Deliver {
            run_id: "r1".into(),
            stage: 2,
        })
    );
    assert_eq!(
        variant_at::<RunRequest>(n - 1, &serde_json::json!({"run_id": "r1", "on": false})),
        Some(RunRequest::Watch {
            run_id: "r1".into(),
            on: false,
        })
    );
}
