use super::hook_command::FLAGS;
use crate::support;

use std::time::{Duration, Instant};

use proto::{ClientMsg, DaemonMsg, HookSource};
use serde_json::json;
use support::hook_wire::{accept, hello, welcome};
use support::{RunningCommand, assert_silent_success, isolated_command, runtime, tempdir};
use tokio::net::UnixListener;

#[test]
fn waits_for_hook_ack_and_bounds_only_the_top_level_tool_response() {
    let dir = tempdir();
    let rt = runtime();
    rt.block_on(async {
        let listener = UnixListener::bind(dir.path().join("daemon.sock")).unwrap();
        let payload = json!({"hook_event_name":"PostToolUse", "tool_response":"outer-result",
            "session_id":"sess-x", "tool_input":{"tool_response":"inner-kept"}, "unknown":[1, true]});
        let mut child =
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
        assert_eq!(
            event,
            ClientMsg::HookEvent {
                window_id: 1,
                source: HookSource::Claude,
                payload: json!({"hook_event_name":"PostToolUse", "tool_response":"outer-result",
                "tool_result_truncated": false, "tool_result_stringified": false,
                "session_id":"sess-x",
                "tool_input":{"tool_response":"inner-kept"}, "unknown":[1, true]})
            }
        );
        proto::write_frame(
            &mut stream,
            &DaemonMsg::Ack {
                request: "unrelated".into(),
            },
        )
        .await
        .unwrap();
        let deadline = Instant::now() + Duration::from_millis(150);
        while Instant::now() < deadline {
            assert!(child.is_running(), "hook exited before its Ack");
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
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

/// `TOOL_RESULT_SUMMARY_MAX` (`crates/cli/src/hook.rs`) is 4096 bytes; a response well over
/// that must come through truncated to exactly that many bytes with the flag set.
#[test]
fn an_oversized_tool_response_is_truncated_and_flagged() {
    let dir = tempdir();
    let rt = runtime();
    rt.block_on(async {
        let listener = UnixListener::bind(dir.path().join("daemon.sock")).unwrap();
        let payload = json!({"hook_event_name":"PostToolUse", "tool_response":"x".repeat(10000),
            "session_id":"sess-oversized"});
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
        let tool_response = object.get("tool_response").unwrap().as_str().unwrap();
        assert_eq!(tool_response.len(), 4096);
        assert_eq!(object.get("tool_result_truncated").unwrap(), &json!(true));
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

/// The 4096-byte cutoff must land on a UTF-8 character boundary, and the fixture has to
/// actually exercise the back-off search rather than get lucky. Wave-1 review finding F1:
/// the original fixture here was 3000 repetitions of the *2-byte* `é` (6000 bytes, over the
/// limit) — but 3000 x 2 = 6000, and byte offset 4096 is itself even, so for a run of 2-byte
/// characters starting at offset 0, byte 4096 is *already* a char boundary. The back-off
/// loop never has to move for that fixture: deleting the loop entirely (`let mut end =
/// TOOL_RESULT_SUMMARY_MAX.min(...)`, no adjustment) still produces a valid 4096-byte cut
/// and passed every test in this file. On a run of *3-byte* characters, deleting that loop
/// is a live bug: `4096 % 3 != 0`, so slicing a `String` at byte 4096 panics (Rust panics on
/// a non-char-boundary index), the panic is swallowed by the no-op hook set in
/// `hook.rs::run`, the process still exits 0, and the daemon never receives a connection —
/// every tool result from a runtime whose output happens to straddle 4096 with a CJK
/// character or a wide emoji would be silently dropped. `世` is 3 bytes; 3000 repetitions is
/// 9000 bytes, and the largest multiple of 3 that is `<= 4096` is 4095, not 4096, so a
/// correct implementation must back off by exactly one byte.
#[test]
fn a_multibyte_tool_response_truncates_on_a_char_boundary() {
    let dir = tempdir();
    let rt = runtime();
    rt.block_on(async {
        let listener = UnixListener::bind(dir.path().join("daemon.sock")).unwrap();
        let payload = json!({"hook_event_name":"PostToolUse", "tool_response":"世".repeat(3000),
            "session_id":"sess-multibyte"});
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
        // Prove the whole frame is valid UTF-8 / valid JSON by re-parsing its serialised
        // bytes, rather than trusting that an in-memory `serde_json::Value` was built
        // without panicking.
        let reencoded = serde_json::to_vec(&forwarded).unwrap();
        let reparsed: serde_json::Value = serde_json::from_slice(&reencoded).unwrap();
        let tool_response = reparsed
            .as_object()
            .unwrap()
            .get("tool_response")
            .unwrap()
            .as_str()
            .unwrap();
        // Exact, not just "<= 4096 and a multiple of 3": pins that the back-off search
        // actually ran and landed one byte short of the naive (and wrong) 4096 cut.
        assert_eq!(
            tool_response.len(),
            4095,
            "expected the largest multiple of 3 <= 4096"
        );
        assert_eq!(
            reparsed
                .as_object()
                .unwrap()
                .get("tool_result_truncated")
                .unwrap(),
            &json!(true)
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

#[test]
fn an_object_tool_response_survives_under_the_limit() {
    let dir = tempdir();
    let rt = runtime();
    rt.block_on(async {
        let listener = UnixListener::bind(dir.path().join("daemon.sock")).unwrap();
        let payload = json!({"hook_event_name":"PostToolUse",
            "tool_response":{"stdout":"ok","exit":0}, "session_id":"sess-object"});
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
        assert_eq!(
            object.get("tool_response").unwrap(),
            &json!({"stdout":"ok","exit":0})
        );
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

/// Pins the `<=` boundary itself (wave-1 review finding F5: `<` in its place still passed
/// every other test in this file — nothing here previously exercised a value of *exactly*
/// `TOOL_RESULT_SUMMARY_MAX`). A value at exactly the limit needs no truncation and must be
/// forwarded byte-for-byte, with the flag `false`.
#[test]
fn a_tool_response_of_exactly_the_limit_is_kept_untruncated() {
    let dir = tempdir();
    let rt = runtime();
    rt.block_on(async {
        let listener = UnixListener::bind(dir.path().join("daemon.sock")).unwrap();
        let exact = "x".repeat(4096);
        let payload = json!({"hook_event_name":"PostToolUse", "tool_response":exact,
            "session_id":"sess-exact"});
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
        assert_eq!(
            object.get("tool_response").unwrap().as_str().unwrap(),
            exact
        );
        assert_eq!(object.get("tool_result_truncated").unwrap(), &json!(false));
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

/// Wave-1 review finding F2: an over-limit JSON *object* must not collapse into a plain
/// string, because M6.5.5's `ok` predicate reads an object's `error`/`success` keys to tell
/// a failed tool from a succeeded one — a stringified blob reads as `ok == true` regardless
/// of what it said, silently turning a failure into a success. Demonstrated with the
/// review's own fixture: `{"error": <5000 'x's>}`. The forwarded value must still be a JSON
/// object, still carry the `error` key (shrunk, not gone), fit under the limit, and be
/// flagged truncated but *not* stringified.
#[test]
fn an_over_limit_object_keeps_its_shape_by_shrinking_a_string_leaf() {
    let dir = tempdir();
    let rt = runtime();
    rt.block_on(async {
        let listener = UnixListener::bind(dir.path().join("daemon.sock")).unwrap();
        let payload = json!({"hook_event_name":"PostToolUse",
            "tool_response": {"error": "x".repeat(5000)}, "session_id":"sess-shrink-leaf"});
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
            .expect("tool_response must still be a JSON object, not a stringified blob");
        let error = response_object
            .get("error")
            .expect("the discriminating `error` key must survive")
            .as_str()
            .expect("error value must still be a string");
        assert!(error.len() < 5000, "the error string should have shrunk");
        let encoded_len = serde_json::to_string(tool_response).unwrap().len();
        assert!(
            encoded_len <= 4096,
            "shrunk object still over budget: {encoded_len} bytes"
        );
        assert_eq!(object.get("tool_result_truncated").unwrap(), &json!(true));
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

/// Task 11's review, M2: a `PostToolUseFailure`'s `error` is bounded as a string
/// `tool_response` is, on a character boundary, so a failed command's whole output never
/// reaches the daemon or is copied under its manager lock. Nothing else in the payload
/// changes, and no `tool_response` is added.
#[test]
fn an_oversized_failure_error_is_truncated_on_a_char_boundary() {
    let dir = tempdir();
    let rt = runtime();
    rt.block_on(async {
        let listener = UnixListener::bind(dir.path().join("daemon.sock")).unwrap();
        let error = "世".repeat(3000);
        let payload = json!({"hook_event_name":"PostToolUseFailure", "error": error,
            "is_interrupt": false, "tool_use_id": "toolu_9", "tool_name": "Bash",
            "tool_input": {"command": "false"}, "session_id": "sess-failure"});
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
        let mut expected = payload.clone();
        expected["error"] = json!("世".repeat(4095 / 3));
        assert_eq!(forwarded, expected);
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
