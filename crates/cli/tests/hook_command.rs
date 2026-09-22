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
                "tool_result_truncated": false, "session_id":"sess-x",
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

/// The 4096-byte cutoff must land on a UTF-8 character boundary. 3000 repetitions of the
/// two-byte character `é` is 6000 bytes, well over the limit, and its 4096th byte falls
/// inside a character (4096 is even, but the point of this fixture is that truncation must
/// not simply cut at a fixed byte count without checking — proven by round-tripping the
/// whole frame through `serde_json` rather than by reasoning about the code).
#[test]
fn a_multibyte_tool_response_truncates_on_a_char_boundary() {
    let dir = tempdir();
    let rt = runtime();
    rt.block_on(async {
        let listener = UnixListener::bind(dir.path().join("daemon.sock")).unwrap();
        let payload = json!({"hook_event_name":"PostToolUse", "tool_response":"é".repeat(3000),
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
        assert_eq!(tool_response.len() % 2, 0, "cut a multi-byte é in half");
        assert!(tool_response.len() <= 4096);
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

/// Pins that decision 5a's bounded summary did not widen `HOOK_PAYLOAD_MAX`: a payload
/// whose raw bytes exceed it is still dropped before it is ever parsed or forwarded.
/// Delivered over stdin, not as a command-line argument, to stay clear of the OS's own
/// `ARG_MAX` limit on a single process argument.
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
