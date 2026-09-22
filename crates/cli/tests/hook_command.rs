mod support;

use std::time::{Duration, Instant};

use proto::{ClientKind, ClientMsg, DaemonMsg, HookSource, Runtime, Status};
use serde_json::json;
use support::{
    RunningCommand, TestDaemon, assert_silent_success, isolated_command, runtime, tempdir,
};
use tokio::net::{UnixListener, UnixStream};

/// `anthrex hook`'s own deadline (`HOOK_DEADLINE` in `crates/cli/src/hook.rs`) is 1s, and
/// every test here that deliberately drives the hook to that deadline needs to absorb a
/// real process spawn on top of it. `anthrex` is a binary-only crate (no `lib.rs`), so
/// these integration tests cannot import `HOOK_DEADLINE` to derive this value from it —
/// the two are coupled only by this comment, which is why the margin is generous rather
/// than tight: 3s is provably clear of `HOOK_DEADLINE + spawn` (measured 33-85ms loaded,
/// see docs/timing-budgets.md) by a wide margin, so an assertion here fails only on an
/// actual regression to the hook exceeding its own deadline, never on ordinary process
/// spawn variance (the investigation distilled into docs/timing-budgets.md found the
/// previous 1500ms bound left only 500ms for the spawn, producing a 58% failure rate
/// when the binary was looped).
const LIMIT: Duration = Duration::from_secs(3);
const FLAGS: &[&str] = &["hook", "--window", "1", "--source", "claude"];

#[test]
fn without_a_daemon_it_exits_zero_silently_and_fast() {
    let dir = tempdir();
    let start = Instant::now();
    let output = RunningCommand::start(isolated_command(dir.path(), FLAGS).arg("{}")).finish(LIMIT);
    assert_silent_success(&output);
    // Not `< 1s`: that bound *is* HOOK_DEADLINE, with a process spawn stacked on top, so
    // it coincided with the hook's own legal worst case rather than bounding a regression
    // (docs/timing-budgets.md's standing rule 1 is this exact shape). Nothing here actually waits close to a second —
    // there is no daemon to connect to, so `forward()` fails fast — so `LIMIT` still
    // catches a real hang while leaving room for spawn variance.
    assert!(start.elapsed() < LIMIT);
    assert!(
        !dir.path().join("daemon.sock").exists(),
        "hook started a daemon"
    );
}

#[test]
fn bad_arguments_exit_zero() {
    let dir = tempdir();
    for args in [
        vec!["hook", "--window", "nope"],
        vec!["hook"],
        vec!["hook", "--help"],
        vec!["hook", "--window", "1", "--source", "unknown"],
    ] {
        let output = RunningCommand::start(&mut isolated_command(dir.path(), &args)).finish(LIMIT);
        assert_silent_success(&output);
    }
}

#[test]
fn held_open_stdin_is_included_in_the_deadline() {
    let dir = tempdir();
    let start = Instant::now();
    let mut child = RunningCommand::start(&mut isolated_command(dir.path(), FLAGS));
    let _held_open = child.stdin();
    assert_silent_success(&child.finish(LIMIT));
    assert!(start.elapsed() < LIMIT);
}

async fn accept(listener: &UnixListener) -> UnixStream {
    let (stream, _) = tokio::time::timeout(Duration::from_millis(1300), listener.accept())
        .await
        .expect("hook never connected")
        .unwrap();
    stream
}

async fn hello(stream: &mut UnixStream) {
    let msg = tokio::time::timeout(
        Duration::from_secs(1),
        proto::read_frame::<_, ClientMsg>(stream),
    )
    .await
    .expect("hook never sent Hello")
    .unwrap();
    assert_eq!(
        msg,
        Some(ClientMsg::Hello {
            proto_version: proto::PROTO_VERSION,
            client: ClientKind::Hook
        })
    );
}

async fn welcome(stream: &mut UnixStream) {
    proto::write_frame(
        stream,
        &DaemonMsg::Welcome {
            daemon_version: "test".into(),
            windows: vec![],
        },
    )
    .await
    .unwrap();
}

#[test]
fn a_silent_daemon_costs_at_most_a_second() {
    let dir = tempdir();
    let rt = runtime();
    rt.block_on(async {
        let listener = UnixListener::bind(dir.path().join("daemon.sock")).unwrap();
        let start = Instant::now();
        let child = RunningCommand::start(isolated_command(dir.path(), FLAGS).arg("{}"));
        let mut stream = accept(&listener).await;
        hello(&mut stream).await;
        assert_silent_success(&child.finish(LIMIT));
        assert!(start.elapsed() < LIMIT);
    });
}

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

#[test]
fn one_deadline_covers_stdin_handshake_and_ack() {
    let dir = tempdir();
    let rt = runtime();
    rt.block_on(async {
        let listener = UnixListener::bind(dir.path().join("daemon.sock")).unwrap();
        let start = Instant::now();
        let mut child = RunningCommand::start(&mut isolated_command(dir.path(), FLAGS));
        // Intentionally spend part of the command's budget waiting for input.
        tokio::time::sleep(Duration::from_millis(350)).await;
        child.input(b"{}".to_vec());
        let mut stream = accept(&listener).await;
        hello(&mut stream).await;
        tokio::time::sleep(Duration::from_millis(350)).await;
        welcome(&mut stream).await;
        let event = tokio::time::timeout(
            Duration::from_millis(500),
            proto::read_frame::<_, ClientMsg>(&mut stream),
        )
        .await
        .unwrap()
        .unwrap();
        assert!(matches!(event, Some(ClientMsg::HookEvent { .. })));
        // Was `finish(Duration::from_millis(650))`: this child is designed to live to
        // HOOK_DEADLINE (~1s) measured from its own start, and by this point the test has
        // already spent 700ms of that budget on its own sleeps before ever calling
        // `finish` — leaving well under 300ms of real slack for process teardown, which
        // is exactly the tightest bound in this file and caused 5 of 7 failures when
        // this binary was looped (see docs/timing-budgets.md, the surviving distillate of
        // the investigation that measured it). `LIMIT` gives it real room:
        // the child still exits ~1s after `start` regardless, so `finish` returns long
        // before its own deadline and `start.elapsed()` below stays far under `LIMIT` too.
        assert_silent_success(&child.finish(LIMIT));
        assert!(start.elapsed() < LIMIT);
    });
}

#[test]
fn oversized_or_invalid_stdin_is_dropped_silently() {
    let daemon = TestDaemon::start(&[json!({"read_line":true})]);
    let id = daemon.client().create(Runtime::Claude, "payload-cap");
    assert_eq!(id, 1);
    let oversized = json!({"hook_event_name":"SessionStart", "session_id":"too-big",
        "tool_response":"x".repeat(8 * 1024 * 1024)})
    .to_string()
    .into_bytes();
    for payload in [b"not json".to_vec(), oversized] {
        let mut child = RunningCommand::start(&mut daemon.command(FLAGS));
        child.input(payload);
        assert_silent_success(&child.finish(LIMIT));
        let windows = daemon.client().windows();
        assert_eq!(
            windows[0].session_id, None,
            "oversized event reached the daemon"
        );
    }
}

#[test]
fn codex_sources_forward_the_last_payload_and_return_on_error() {
    let dir = tempdir();
    let rt = runtime();
    rt.block_on(async {
        let listener = UnixListener::bind(dir.path().join("daemon.sock")).unwrap();
        for (label, source) in [
            ("codex-notify", HookSource::CodexNotify),
            ("codex-hook", HookSource::CodexHook),
        ] {
            let payload = json!({"type":"agent-turn-complete", "thread-id":"codex-session"});
            let child = RunningCommand::start(&mut isolated_command(
                dir.path(),
                &[
                    "hook",
                    "--window",
                    "42",
                    "--source",
                    label,
                    "ignored-argument",
                    &payload.to_string(),
                ],
            ));
            let mut stream = accept(&listener).await;
            hello(&mut stream).await;
            welcome(&mut stream).await;
            let event = tokio::time::timeout(
                Duration::from_secs(1),
                proto::read_frame::<_, ClientMsg>(&mut stream),
            )
            .await
            .unwrap()
            .unwrap();
            assert_eq!(
                event,
                Some(ClientMsg::HookEvent {
                    window_id: 42,
                    source,
                    payload
                })
            );
            proto::write_frame(
                &mut stream,
                &DaemonMsg::Error {
                    request: "hook".into(),
                    message: "unknown window".into(),
                },
            )
            .await
            .unwrap();
            assert_silent_success(&child.finish(Duration::from_millis(500)));
        }
    });
}

#[test]
fn payload_from_stdin_and_from_the_last_argument() {
    let daemon = TestDaemon::start(&[json!({"read_line":true})]);
    let mut client = daemon.client();
    let id = client.create(Runtime::Claude, "hook-input");
    let id = id.to_string();
    let args = ["hook", "--window", &id, "--source", "claude"];
    let mut child = RunningCommand::start(&mut daemon.command(&args));
    child.input(
        json!({"hook_event_name":"SessionStart", "session_id":"x1"})
            .to_string()
            .into_bytes(),
    );
    assert_silent_success(&child.finish(LIMIT));
    client.wait_window(id.parse().unwrap(), "session id from stdin", |w| {
        w.session_id.as_deref() == Some("x1")
    });
    let payload = json!({"hook_event_name":"UserPromptSubmit"}).to_string();
    let output = RunningCommand::start(daemon.command(&args).arg(payload)).finish(LIMIT);
    assert_silent_success(&output);
    client.wait_window(id.parse().unwrap(), "working from argument", |w| {
        w.status == Status::Working
    });
}
