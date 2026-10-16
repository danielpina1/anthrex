use super::hook_command::{FLAGS, LIMIT};
use crate::support;

use std::time::{Duration, Instant};

use proto::{ClientMsg, DaemonMsg};
use serde_json::json;
use support::hook_wire::{accept, hello, welcome};
use support::{
    RunningCommand, assert_silent_success, isolated_command, runtime, tempdir, warm_anthrex,
};
use tokio::net::UnixListener;

/// Wave-2 review ruling: an object `tool_response` must never be stringified, because
/// `ToolResult.ok` is a plain `bool` with no honest third "unknown" state to fall back on —
/// so the `error`/`success` predicate must always be evaluable instead of sometimes lost.
/// This is the case the whole finding exists for: an object whose *structure* (not its
/// string content) is what pushes it over budget — thousands of short numeric keys, with no
/// string leaves worth cutting — used to fall through to the wave-1 stringify fallback and
/// lose `error`. It must now survive as an object with `error` intact, dropping the bulk
/// keys instead.
#[test]
fn an_over_limit_object_with_bulk_keys_keeps_error_intact() {
    let dir = tempdir();
    let rt = runtime();
    rt.block_on(async {
        let listener = UnixListener::bind(dir.path().join("daemon.sock")).unwrap();
        // ~5000 short numeric-valued keys: no string leaf anywhere in them, so the wave-1
        // shrink-a-string-leaf mechanism has nothing to cut and would fall through to
        // stringify. `error` is a short, ordinary string that always fits on its own.
        let mut bulky: serde_json::Map<String, serde_json::Value> = (0..5000)
            .map(|i| (format!("k{i:04}"), serde_json::Value::from(i)))
            .collect();
        bulky.insert("error".to_string(), serde_json::Value::from("boom"));
        let payload = json!({"hook_event_name":"PostToolUse",
            "tool_response": serde_json::Value::Object(bulky), "session_id":"sess-error-bulk"});
        let child =
            RunningCommand::start(isolated_command(dir.path(), FLAGS).arg(payload.to_string()));
        let mut stream = accept(&listener).await;
        hello(&mut stream).await;
        welcome(&mut stream).await;
        let event = tokio::time::timeout(
            Duration::from_secs(1),
            proto::read_frame::<_, ClientMsg>(&mut stream),
        )
        .await
        .unwrap()
        .unwrap()
        .unwrap();
        let ClientMsg::HookEvent {
            payload: forwarded, ..
        } = event
        else {
            panic!("expected a HookEvent");
        };
        let object = forwarded.as_object().unwrap();
        let tool_response = object.get("tool_response").unwrap();
        let response_object = tool_response
            .as_object()
            .expect("tool_response must still be a JSON object, never a stringified blob");
        assert_eq!(
            response_object.get("error").unwrap(),
            &json!("boom"),
            "the error key must survive, unshrunk, since it fits on its own"
        );
        assert!(
            response_object.len() < 5001,
            "some bulk keys must have been dropped to make room"
        );
        let encoded_len = serde_json::to_string(tool_response).unwrap().len();
        assert!(
            encoded_len <= 4096,
            "object still over budget: {encoded_len} bytes"
        );
        assert_eq!(object.get("tool_result_truncated").unwrap(), &json!(true));
        assert_eq!(
            object.get("tool_result_stringified").unwrap(),
            &json!(false),
            "an object tool_response is never stringified"
        );
        proto::write_frame(
            &mut stream,
            &DaemonMsg::Ack {
                request: "hook".into(),
            },
        )
        .await
        .unwrap();
        assert_silent_success(&child.finish(Duration::from_millis(500)));
    });
}

/// The symmetric case for the `success` key, with every value distinct from the `error`
/// fixture above (different key prefix, different value type for the bulk keys, different
/// priority key and value) so a bug that kept the wrong key, or dropped the wrong set of
/// bulk keys, would be caught rather than passing by resemblance.
#[test]
fn an_over_limit_object_with_bulk_keys_keeps_success_intact() {
    let dir = tempdir();
    let rt = runtime();
    rt.block_on(async {
        let listener = UnixListener::bind(dir.path().join("daemon.sock")).unwrap();
        let mut bulky: serde_json::Map<String, serde_json::Value> = (0..5000)
            .map(|i| (format!("j{i:04}"), serde_json::Value::from(format!("s{i}"))))
            .collect();
        bulky.insert("success".to_string(), serde_json::Value::from(false));
        let payload = json!({"hook_event_name":"PostToolUse",
            "tool_response": serde_json::Value::Object(bulky), "session_id":"sess-success-bulk"});
        let child =
            RunningCommand::start(isolated_command(dir.path(), FLAGS).arg(payload.to_string()));
        let mut stream = accept(&listener).await;
        hello(&mut stream).await;
        welcome(&mut stream).await;
        let event = tokio::time::timeout(
            Duration::from_secs(1),
            proto::read_frame::<_, ClientMsg>(&mut stream),
        )
        .await
        .unwrap()
        .unwrap()
        .unwrap();
        let ClientMsg::HookEvent {
            payload: forwarded, ..
        } = event
        else {
            panic!("expected a HookEvent");
        };
        let object = forwarded.as_object().unwrap();
        let tool_response = object.get("tool_response").unwrap();
        let response_object = tool_response
            .as_object()
            .expect("tool_response must still be a JSON object, never a stringified blob");
        assert_eq!(
            response_object.get("success").unwrap(),
            &json!(false),
            "the success key must survive, unshrunk, since it fits on its own"
        );
        assert!(
            response_object.len() < 5001,
            "some bulk keys must have been dropped to make room"
        );
        let encoded_len = serde_json::to_string(tool_response).unwrap().len();
        assert!(
            encoded_len <= 4096,
            "object still over budget: {encoded_len} bytes"
        );
        assert_eq!(object.get("tool_result_truncated").unwrap(), &json!(true));
        assert_eq!(
            object.get("tool_result_stringified").unwrap(),
            &json!(false),
            "an object tool_response is never stringified"
        );
        proto::write_frame(
            &mut stream,
            &DaemonMsg::Ack {
                request: "hook".into(),
            },
        )
        .await
        .unwrap();
        assert_silent_success(&child.finish(Duration::from_millis(500)));
    });
}

/// A gap the two fixtures above don't reach: in both of them, `error`/`success` are tiny and
/// always fit on their own, so the case where a priority key's *own* value has no string
/// leaf to shrink (an array of numbers, say) and would otherwise have to be dropped like any
/// other key is never exercised — confirmed by mutation (forcing `error`/`success` through
/// the same droppable path as every other key still passes both fixtures above; only this
/// one catches it). `error` here is a 2000-element array of numbers, large enough on its own
/// to exceed `TOOL_RESULT_SUMMARY_MAX` with no string content anywhere in it. `error` must
/// still be present and non-null (some truncated rendering of itself) rather than dropped —
/// `bound_object`'s `insert_shrinking(..., allow_drop: false, ...)` renders a leaf-less
/// priority value to its own JSON text before shrinking it, precisely for this case.
#[test]
fn an_over_limit_error_value_with_no_string_leaf_still_survives_non_null() {
    let dir = tempdir();
    let rt = runtime();
    rt.block_on(async {
        let listener = UnixListener::bind(dir.path().join("daemon.sock")).unwrap();
        let big_array: Vec<i64> = (0..2000).collect();
        let payload = json!({"hook_event_name":"PostToolUse",
            "tool_response": {"error": big_array}, "session_id":"sess-leafless-error"});
        let child =
            RunningCommand::start(isolated_command(dir.path(), FLAGS).arg(payload.to_string()));
        let mut stream = accept(&listener).await;
        hello(&mut stream).await;
        welcome(&mut stream).await;
        let event = tokio::time::timeout(
            Duration::from_secs(1),
            proto::read_frame::<_, ClientMsg>(&mut stream),
        )
        .await
        .unwrap()
        .unwrap()
        .unwrap();
        let ClientMsg::HookEvent {
            payload: forwarded, ..
        } = event
        else {
            panic!("expected a HookEvent");
        };
        let object = forwarded.as_object().unwrap();
        let tool_response = object.get("tool_response").unwrap();
        let response_object = tool_response
            .as_object()
            .expect("tool_response must still be a JSON object");
        let error = response_object
            .get("error")
            .expect("error must not be dropped even though it has no string leaf to shrink");
        assert!(!error.is_null(), "error must remain non-null");
        let encoded_len = serde_json::to_string(tool_response).unwrap().len();
        assert!(
            encoded_len <= 4096,
            "object still over budget: {encoded_len} bytes"
        );
        assert_eq!(object.get("tool_result_truncated").unwrap(), &json!(true));
        assert_eq!(
            object.get("tool_result_stringified").unwrap(),
            &json!(false),
            "an object tool_response is never stringified, even when one field within it had \
             to be rendered to text"
        );
        proto::write_frame(
            &mut stream,
            &DaemonMsg::Ack {
                request: "hook".into(),
            },
        )
        .await
        .unwrap();
        assert_silent_success(&child.finish(Duration::from_millis(500)));
    });
}

/// Wave-2 review ruling: `tool_result_stringified` is now only ever `true` for a type with
/// no `ok` predicate to protect (array, number, bool, null) — never for an object. A large
/// array has to be converted to a truncated string to fit at all (there is no way to keep
/// an array's *shape* the way an object's discriminating keys are kept), and M6.5.5's
/// documented default `ok == true` for a non-object `tool_response` is already correct for
/// it, so the flag here is honest documentation, not a signal to branch on.
#[test]
fn an_over_limit_array_tool_response_is_stringified_and_flagged() {
    let dir = tempdir();
    let rt = runtime();
    rt.block_on(async {
        let listener = UnixListener::bind(dir.path().join("daemon.sock")).unwrap();
        let big_array: Vec<i64> = (0..2000).collect();
        let payload = json!({"hook_event_name":"PostToolUse",
            "tool_response": big_array, "session_id":"sess-array"});
        let child =
            RunningCommand::start(isolated_command(dir.path(), FLAGS).arg(payload.to_string()));
        let mut stream = accept(&listener).await;
        hello(&mut stream).await;
        welcome(&mut stream).await;
        let event = tokio::time::timeout(
            Duration::from_secs(1),
            proto::read_frame::<_, ClientMsg>(&mut stream),
        )
        .await
        .unwrap()
        .unwrap()
        .unwrap();
        let ClientMsg::HookEvent {
            payload: forwarded, ..
        } = event
        else {
            panic!("expected a HookEvent");
        };
        let object = forwarded.as_object().unwrap();
        let tool_response = object.get("tool_response").unwrap();
        assert!(
            tool_response.is_string(),
            "an over-limit array must be converted to a string"
        );
        assert!(tool_response.as_str().unwrap().len() <= 4096);
        assert_eq!(object.get("tool_result_truncated").unwrap(), &json!(true));
        assert_eq!(object.get("tool_result_stringified").unwrap(), &json!(true));
        proto::write_frame(
            &mut stream,
            &DaemonMsg::Ack {
                request: "hook".into(),
            },
        )
        .await
        .unwrap();
        assert_silent_success(&child.finish(Duration::from_millis(500)));
    });
}

/// Wave-1 review finding F8: `tool_result_truncated`/`tool_result_stringified` must be
/// overwritten, not merged with, whatever the runtime happened to send under those same
/// keys — untested before this. A payload that already carries a (nonsensical)
/// `tool_result_truncated` string, with a `tool_response` that needs no truncation at all,
/// must still come through with the computed boolean values.
#[test]
fn tool_result_truncated_and_stringified_overwrite_whatever_the_runtime_sent() {
    let dir = tempdir();
    let rt = runtime();
    rt.block_on(async {
        let listener = UnixListener::bind(dir.path().join("daemon.sock")).unwrap();
        let payload = json!({"hook_event_name":"PostToolUse", "tool_response":"short",
            "tool_result_truncated": "nonsense", "tool_result_stringified": "also-nonsense",
            "session_id":"sess-overwrite"});
        let child =
            RunningCommand::start(isolated_command(dir.path(), FLAGS).arg(payload.to_string()));
        let mut stream = accept(&listener).await;
        hello(&mut stream).await;
        welcome(&mut stream).await;
        let event = tokio::time::timeout(
            Duration::from_secs(1),
            proto::read_frame::<_, ClientMsg>(&mut stream),
        )
        .await
        .unwrap()
        .unwrap()
        .unwrap();
        let ClientMsg::HookEvent {
            payload: forwarded, ..
        } = event
        else {
            panic!("expected a HookEvent");
        };
        let object = forwarded.as_object().unwrap();
        assert_eq!(object.get("tool_result_truncated").unwrap(), &json!(false));
        assert_eq!(
            object.get("tool_result_stringified").unwrap(),
            &json!(false)
        );
        proto::write_frame(
            &mut stream,
            &DaemonMsg::Ack {
                request: "hook".into(),
            },
        )
        .await
        .unwrap();
        assert_silent_success(&child.finish(Duration::from_millis(500)));
    });
}

/// Pins the *outcome* — decision 5a's bounded summary did not widen `HOOK_PAYLOAD_MAX`, a
/// payload whose raw bytes exceed it is still dropped before it is ever parsed or forwarded
/// — but not `HOOK_PAYLOAD_MAX`'s own check in isolation: deleting the explicit `bytes.len()
/// > HOOK_PAYLOAD_MAX` check still passes this test, because the stdin reader's
/// `.take(HOOK_PAYLOAD_MAX + 1)` already clamps what can be read, and the resulting
/// truncated-mid-JSON prefix fails to parse (`serde_json::from_slice` errors, `forward()`
/// returns `None`). A real over-limit payload delivered as a single CLI argument is in any
/// case unreachable on macOS — `ARG_MAX` is far below 8 MiB — so this test exercises the
/// code's actual defense-in-depth (the stdin cap plus the parse failure), not a standalone
/// proof of the explicit guard. Delivered over stdin, not as a command-line argument, to
/// stay clear of that same `ARG_MAX` limit.
#[test]
fn a_payload_over_the_size_limit_is_still_dropped_whole() {
    warm_anthrex();
    let dir = tempdir();
    let rt = runtime();
    rt.block_on(async {
        let listener = UnixListener::bind(dir.path().join("daemon.sock")).unwrap();
        // HOOK_PAYLOAD_MAX is 8 MiB; this payload's raw bytes exceed it on their own,
        // before any JSON parsing.
        let oversized = json!({"hook_event_name":"PostToolUse",
            "tool_response":"x".repeat(9 * 1024 * 1024), "session_id":"sess-huge"})
        .to_string()
        .into_bytes();
        let start = Instant::now();
        let mut child = RunningCommand::start(&mut isolated_command(dir.path(), FLAGS));
        child.input(oversized);
        let output = child.finish(LIMIT);
        assert_silent_success(&output);
        assert!(start.elapsed() < LIMIT);
        // The listener never receives a connection at all: the payload is dropped before
        // the hook ever dials the daemon.
        let accept_result =
            tokio::time::timeout(Duration::from_millis(200), listener.accept()).await;
        assert!(
            accept_result.is_err(),
            "hook connected to the daemon with an oversized payload"
        );
    });
}
