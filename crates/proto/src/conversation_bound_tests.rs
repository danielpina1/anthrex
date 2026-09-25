//! The tool-result bounding M8a.7 moved here from `crates/cli/src/hook.rs`, now shared by
//! `anthrex hook` and the daemon's headless sessions.

use super::*;
use serde_json::json;

#[test]
fn truncation_backs_off_to_a_char_boundary() {
    // 4096 is not a multiple of 3: the cut must back off one byte.
    let wide = truncate_to_char_boundary(&"世".repeat(3000), TOOL_RESULT_SUMMARY_MAX);
    assert_eq!((wide.len(), wide.chars().count()), (4095, 1365));
    // 1 + 4 * 1023 = 4093: a 4-byte character straddles 4096 and is dropped whole.
    let mixed = format!("a{}", "😀".repeat(2000));
    assert_eq!(
        truncate_to_char_boundary(&mixed, TOOL_RESULT_SUMMARY_MAX).len(),
        4093
    );
    assert_eq!(
        truncate_to_char_boundary("short", TOOL_RESULT_SUMMARY_MAX),
        "short"
    );
}

#[test]
fn a_string_response_is_cut_on_its_text_and_flagged() {
    let (value, truncated, stringified) = bound_tool_response_value(json!("世".repeat(3000)));
    assert_eq!(value.as_str().map(str::len), Some(4095));
    assert_eq!((truncated, stringified), (true, false));
    let at_limit = "a".repeat(TOOL_RESULT_SUMMARY_MAX);
    assert_eq!(
        bound_tool_response_value(json!(at_limit)),
        (json!(at_limit), false, false)
    );
}

#[test]
fn an_over_limit_object_keeps_its_error_key_and_fits() {
    let (value, truncated, stringified) = bound_tool_response_value(json!({
        "error": "x".repeat(6000),
        "zzz": "y".repeat(100),
    }));
    assert!(truncated && !stringified);
    assert!(value["error"].as_str().is_some_and(|e| !e.is_empty()));
    assert!(serde_json::to_string(&value).unwrap().len() <= TOOL_RESULT_SUMMARY_MAX);
}

#[test]
fn an_over_limit_array_is_stringified() {
    let (value, truncated, stringified) = bound_tool_response_value(json!(vec![1u32; 3000]));
    assert!(value.is_string() && truncated && stringified);
}

#[test]
fn bound_tool_response_rewrites_only_the_top_level_field() {
    let mut object = json!({"tool_response": "ok", "tool_result_truncated": true});
    let map = object.as_object_mut().unwrap();
    bound_tool_response(map);
    assert_eq!(
        object,
        json!({"tool_response": "ok", "tool_result_truncated": false, "tool_result_stringified": false})
    );
    let mut none = json!({"other": 1});
    bound_tool_response(none.as_object_mut().unwrap());
    assert_eq!(none, json!({"other": 1}));
}
