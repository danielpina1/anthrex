use crate::support;

use std::time::{Duration, Instant};

use proto::{ClientMsg, DaemonMsg, HookSource, Runtime, Status};
use serde_json::json;
use support::hook_wire::{accept, hello, welcome};
use support::{
    RunningCommand, TestDaemon, assert_silent_success, isolated_command, runtime, tempdir,
    warm_anthrex,
};
use tokio::net::UnixListener;

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
///
/// The spawn term is a spawn of an `anthrex` macOS has already assessed. The first exec
/// of a freshly built binary is not (2.5-3 s measured, unbounded per "Measured primitive
/// costs"), and it lands in whichever test execs the binary first. Under libtest this
/// file's binary ran after `codex_version`'s, `daemon_*`'s and `decider_*`'s, which exec
/// `anthrex` first; under cargo-nextest (M9.12.4) the binaries' tests interleave and these
/// run alone, so each timing test below calls `warm_anthrex` before its clock starts and
/// pays that first exec outside `LIMIT`.
pub(super) const LIMIT: Duration = Duration::from_secs(3);
pub(super) const FLAGS: &[&str] = &["hook", "--window", "1", "--source", "claude"];

#[test]
fn without_a_daemon_it_exits_zero_silently_and_fast() {
    warm_anthrex();
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
    warm_anthrex();
    let dir = tempdir();
    let start = Instant::now();
    let mut child = RunningCommand::start(&mut isolated_command(dir.path(), FLAGS));
    let _held_open = child.stdin();
    assert_silent_success(&child.finish(LIMIT));
    assert!(start.elapsed() < LIMIT);
}

#[test]
fn a_silent_daemon_costs_at_most_a_second() {
    warm_anthrex();
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
fn one_deadline_covers_stdin_handshake_and_ack() {
    warm_anthrex();
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
