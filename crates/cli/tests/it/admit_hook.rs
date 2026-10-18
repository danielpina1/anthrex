//! Milestone 9.14 (M9.14.8, M9.14.9): `anthrex admit`, the worker's admission hook, and
//! the hook events that release its holds, against a real daemon whose `[testing]
//! test_slots` is 1 (or 2), its agents pinned to `fake-agent`. Each `anthrex admit` gets
//! its payload on stdin, and stdin closed.

use crate::support;

use std::path::Path;
use std::process::Output;
use std::time::{Duration, Instant};

use daemon::admit::{ADMIT_CLIENT_GRACE, ADMIT_SWEEP};
use proto::{ClientKind, ClientMsg, HANDSHAKE_TIMEOUT, Runtime};
use serde_json::{Value, json};
use support::{
    RunningCommand, TestDaemon, assert_silent_success, isolated_command, tempdir, warm_anthrex,
};

/// As `hook_command.rs`'s `LIMIT`: no daemon constant is on this path, only an
/// `anthrex` spawn (≤ 85 ms loaded, `docs/timing-budgets.md`) and an immediate
/// `ECONNREFUSED` or an exit before connecting; 3 s is clear of both by a wide margin.
const LIMIT: Duration = Duration::from_secs(3);
/// An ask that reaches the daemon and is answered at once (a free slot, a held key, an
/// unknown window), and a wait for the daemon's log line about such an ask: an
/// `anthrex` spawn, the `Hello`/`Welcome` handshake (whose legal worst case is the
/// client's `HANDSHAKE_TIMEOUT`, 5 s) and then one log write through the non-blocking
/// appender (milliseconds). `LIMIT` on top of the handshake covers the spawn and the
/// flush (M9.14.10's timing rows; task 8 review M5, task 9 review Minor 7).
const ASK_LIMIT: Duration = Duration::from_secs(HANDSHAKE_TIMEOUT.as_secs() + LIMIT.as_secs());
/// Room for a loaded machine on top of a bound's legal worst case.
const WALL_CLOCK_SLACK: Duration = Duration::from_secs(10);

/// A daemon with one test slot; its config and the config's directory live as long as it.
struct Rig {
    daemon: TestDaemon,
    _config: tempfile::TempDir,
}

impl Rig {
    fn start() -> Rig {
        Rig::with_slots(1)
    }

    fn with_slots(slots: u32) -> Rig {
        let config = tempdir();
        let path = config.path().join("config.toml");
        // Ruling C3: this config replaces `TestDaemon`'s, so it keeps lessons off too,
        // and 9.16's C19 the quota brake.
        let lessons = crate::support::lessons::LESSONS_OFF;
        let brakes = crate::support::brakes::BRAKES_OFF;
        std::fs::write(
            &path,
            format!("{lessons}\n[testing]\ntest_slots = {slots}\n\n{brakes}"),
        )
        .unwrap();
        let daemon = TestDaemon::start_configured(&[json!({"read_line": true})], |command| {
            command
                .env("ANTHREX_CONFIG", &path)
                .env("ANTHREX_LOG", "info");
        });
        Rig {
            daemon,
            _config: config,
        }
    }

    /// A `fake-agent` Claude window.
    fn window(&self) -> u32 {
        self.named("agent")
    }

    /// A `fake-agent` Claude window named `name` (names are unique per daemon).
    fn named(&self, name: &str) -> u32 {
        self.daemon.client().create(Runtime::Claude, name)
    }

    /// `anthrex admit --window <window> --wait-secs <wait>` with `payload` on stdin.
    fn admit(&self, window: u32, wait: u64, payload: &Value) -> RunningCommand {
        let (window, wait) = (window.to_string(), wait.to_string());
        let args = ["admit", "--window", &window, "--wait-secs", &wait];
        let mut child = RunningCommand::start(&mut self.daemon.command(&args));
        child.input(payload.to_string().into_bytes());
        child
    }

    /// `anthrex hook --window <window> --source claude <payload>`, which returns once the
    /// daemon has acknowledged the event (or at `HOOK_DEADLINE`, 1 s).
    fn hook(&self, window: u32, payload: &Value) {
        let window = window.to_string();
        let mut command = self
            .daemon
            .command(&["hook", "--window", &window, "--source", "claude"]);
        command.arg(payload.to_string());
        finish(RunningCommand::start(&mut command), LIMIT);
    }

    fn log(&self) -> String {
        std::fs::read_to_string(self.daemon.data_dir().join("daemon.log")).unwrap_or_default()
    }

    /// The daemon log's lines with `message` for `tool_use_id`.
    fn lines(&self, message: &str, tool_use_id: &str) -> usize {
        let id = format!("tool_use_id=\"{tool_use_id}\"");
        self.log()
            .lines()
            .filter(|line| line.contains(message) && line.contains(&id))
            .count()
    }

    /// Waits (deadline loop) until the log has `message` for `tool_use_id`.
    fn wait_for(&self, message: &str, tool_use_id: &str) {
        self.wait_for_within(message, tool_use_id, ASK_LIMIT);
    }

    fn wait_for_within(&self, message: &str, tool_use_id: &str, limit: Duration) {
        let deadline = Instant::now() + limit;
        while self.lines(message, tool_use_id) == 0 {
            assert!(
                Instant::now() < deadline,
                "no {message:?} for {tool_use_id}: {}",
                self.log()
            );
            std::thread::sleep(Duration::from_millis(10));
        }
    }
}

fn bash(tool_use_id: Option<&str>, command: &str) -> Value {
    let mut payload = json!({"hook_event_name": "PreToolUse", "tool_name": "Bash",
        "tool_input": {"command": command}});
    if let Some(id) = tool_use_id {
        payload["tool_use_id"] = json!(id);
    }
    payload
}

fn finish(child: RunningCommand, limit: Duration) -> Output {
    let output = child.finish(limit);
    assert_silent_success(&output);
    output
}

#[test]
fn without_a_daemon_admit_exits_zero_silently_and_fast() {
    // Before the clock, as in `hook_command.rs`'s twin: the first exec of a freshly built
    // `anthrex` (macOS's assessment, 1.6-4.7 s measured) is not what `LIMIT` bounds, and
    // `isolated_command` would otherwise pay it inside it (flake diagnosis, 2026-10-18).
    warm_anthrex();
    let dir = tempdir();
    let args = ["admit", "--window", "1", "--wait-secs", "60"];
    let start = Instant::now();
    let mut child = RunningCommand::start(&mut isolated_command(dir.path(), &args));
    child.input(bash(Some("a"), "cargo test").to_string().into_bytes());
    finish(child, LIMIT);
    assert!(start.elapsed() < LIMIT, "{:?}", start.elapsed());
    assert!(!Path::new(&dir.path().join("daemon.sock")).exists());
}

#[test]
fn a_light_command_or_no_tool_use_id_never_connects() {
    let rig = Rig::start();
    let window = rig.window();
    finish(rig.admit(window, 60, &bash(Some("a"), "ls -la")), LIMIT);
    finish(rig.admit(window, 60, &bash(None, "cargo test")), LIMIT);
    let log = rig.log();
    assert!(!log.contains("admit asked"), "{log}");
}

/// Derivation: the daemon answers `AdmitTimedOut` at W = 1 s after its `Admit`
/// arrives. Before that the client pays an `anthrex` spawn (warmed by
/// `isolated_command`, so not macOS's first exec) and a handshake bounded by
/// `HANDSHAKE_TIMEOUT` (5 s): 1 + 5 + spawn < `ADMIT_CLIENT_GRACE` (11 s from the
/// client's start, `W + ADMIT_CLIENT_GRACE`), which `daemon/src/admit.rs`'s `const _`
/// keeps true. The client's own deadline would end it at ≥ 11 s, so finishing under
/// 11 s shows the daemon's answer ended it. The loop around the exit is capped at
/// `W + ADMIT_CLIENT_GRACE + WALL_CLOCK_SLACK`.
#[test]
fn admit_returns_after_the_wait_budget_and_exits_zero() {
    let rig = Rig::start();
    let window = rig.window();
    finish(
        rig.admit(window, 60, &bash(Some("a"), "cargo test")),
        ASK_LIMIT,
    );
    rig.wait_for("admit granted", "a");

    let wait = Duration::from_secs(1);
    let own_deadline = wait + ADMIT_CLIENT_GRACE;
    assert!(wait + HANDSHAKE_TIMEOUT < own_deadline);
    let start = Instant::now();
    let child = rig.admit(window, 1, &bash(Some("b"), "cargo build"));
    finish(child, own_deadline + WALL_CLOCK_SLACK);
    let elapsed = start.elapsed();
    assert!(elapsed >= wait, "{elapsed:?}");
    assert!(elapsed < own_deadline, "{elapsed:?}");
    rig.wait_for("admit timed out", "b");
}

/// The second ask for a held tool use is answered at once: had it waited, it would have
/// waited its full W (60 s) behind its own hold. The bound is W.
#[test]
fn a_held_tool_use_asked_again_is_admitted_at_once() {
    let rig = Rig::start();
    let window = rig.window();
    let payload = bash(Some("a"), "cargo test");
    finish(rig.admit(window, 60, &payload), ASK_LIMIT);
    rig.wait_for("admit granted", "a");
    let wait = Duration::from_secs(60);
    let start = Instant::now();
    finish(rig.admit(window, 60, &payload), wait);
    assert!(start.elapsed() < wait, "{:?}", start.elapsed());
    rig.wait_for("admit already held", "a");
    assert_eq!(rig.lines("admit granted", "a"), 1, "{}", rig.log());
    assert_eq!(rig.lines("admit already held", "a"), 1, "{}", rig.log());
}

/// Task 8 review M6 (pinning): an ask for a window the daemon does not have is answered
/// with an `Error` at once, and the client exits 0, silently, without waiting.
#[test]
fn an_ask_for_an_unknown_window_exits_zero_at_once() {
    let rig = Rig::start();
    rig.window();
    let start = Instant::now();
    finish(rig.admit(99, 60, &bash(Some("z"), "cargo test")), ASK_LIMIT);
    assert!(start.elapsed() < ASK_LIMIT, "{:?}", start.elapsed());
    rig.wait_for("admit asked", "z");
    assert_eq!(rig.lines("admit granted", "z"), 0, "{}", rig.log());
}

/// A Claude hook event `name` for `tool_use_id`, as the status hook forwards it.
fn event(name: &str, tool_use_id: Option<&str>) -> Value {
    let mut payload = json!({"hook_event_name": name, "session_id": "s", "tool_name": "Bash",
        "tool_input": {"command": "cargo test"}});
    if let Some(id) = tool_use_id {
        payload["tool_use_id"] = json!(id);
    }
    payload
}

/// B's own W: the bound every release test below asserts against.
const W: Duration = Duration::from_secs(60);

/// A (window `a`, tool use `a`) holds the only slot and B (window `b`, tool use `b`)
/// waits behind it with W; returns B and when it started.
fn held_and_waiting(rig: &Rig, a: u32, b: u32) -> (RunningCommand, Instant) {
    finish(rig.admit(a, 60, &bash(Some("a"), "cargo test")), ASK_LIMIT);
    rig.wait_for("admit granted", "a");
    let start = Instant::now();
    let waiting = rig.admit(b, W.as_secs(), &bash(Some("b"), "cargo build"));
    rig.wait_for("admit asked", "b");
    (waiting, start)
}

/// Decision 22: A's `PostToolUse` releases its hold, so B is granted. Bound: B's own W
/// (60 s). The release path's legal worst case is `HOOK_DEADLINE` (1 s, `anthrex hook`)
/// plus a grant made inside the release call (`slots/mod.rs`), so the margin is ~59 s;
/// the loop is capped at W.
#[test]
fn a_post_tool_use_releases_the_hold() {
    let rig = Rig::start();
    let (a, b) = (rig.named("a"), rig.named("b"));
    let (waiting, start) = held_and_waiting(&rig, a, b);
    rig.hook(a, &event("PostToolUse", Some("a")));
    finish(waiting, W);
    assert!(start.elapsed() < W, "{:?}", start.elapsed());
    rig.wait_for("admit released", "a");
    rig.wait_for("admit granted", "b");
}

/// Fact F4: a failed `Bash` fires `PostToolUseFailure` instead of `PostToolUse`, and it
/// releases the hold the same way. Bound: as `a_post_tool_use_releases_the_hold`.
#[test]
fn a_post_tool_use_failure_releases_the_hold() {
    let rig = Rig::start();
    let (a, b) = (rig.named("a"), rig.named("b"));
    let (waiting, start) = held_and_waiting(&rig, a, b);
    let mut failure = event("PostToolUseFailure", Some("a"));
    failure["error"] = json!("exit status 101");
    failure["is_interrupt"] = json!(false);
    failure["duration_ms"] = json!(1200);
    rig.hook(a, &failure);
    finish(waiting, W);
    assert!(start.elapsed() < W, "{:?}", start.elapsed());
    rig.wait_for("admit released", "a");
}

/// Decision 22: the window's `Stop` releases every hold it has. A1 and A2 (window 1, each
/// `Want::Half` of 2 slots, so 1) hold both slots; B (window 2) waits. Bound: as
/// `a_post_tool_use_releases_the_hold`.
#[test]
fn stop_releases_every_hold_of_the_window() {
    let rig = Rig::with_slots(2);
    let (a, b) = (rig.named("a"), rig.named("b"));
    finish(rig.admit(a, 60, &bash(Some("a1"), "cargo test")), ASK_LIMIT);
    finish(rig.admit(a, 60, &bash(Some("a2"), "cargo test")), ASK_LIMIT);
    rig.wait_for("admit granted", "a1");
    rig.wait_for("admit granted", "a2");
    let start = Instant::now();
    let waiting = rig.admit(b, W.as_secs(), &bash(Some("b"), "cargo build"));
    rig.wait_for("admit asked", "b");
    rig.hook(a, &event("Stop", None));
    finish(waiting, W);
    assert!(start.elapsed() < W, "{:?}", start.elapsed());
    rig.wait_for("admit released", "a1");
    rig.wait_for("admit released", "a2");
}

/// Decision 23 (C2, C17): B, a hand-rolled socket client (never a process, so nothing is
/// signalled), asks and then goes away by dropping its stream; C asks after it. A's
/// `PostToolUse` must reach C: had B's abandoned ask kept its place, B would be granted
/// first and C would wait its full W. Bound: as `a_post_tool_use_releases_the_hold`,
/// after its log waits.
#[test]
fn a_client_that_goes_away_gives_up_its_place() {
    let rig = Rig::start();
    let (a, b) = (rig.named("a"), rig.named("b"));
    finish(rig.admit(a, 60, &bash(Some("a"), "cargo test")), ASK_LIMIT);
    rig.wait_for("admit granted", "a");

    let rt = support::runtime();
    let socket = rig.daemon.socket();
    let gone = rt.block_on(async {
        let mut stream = tokio::net::UnixStream::connect(&socket).await.unwrap();
        let hello = ClientMsg::Hello {
            proto_version: proto::PROTO_VERSION,
            client: ClientKind::Hook,
        };
        proto::write_frame(&mut stream, &hello).await.unwrap();
        let admit = ClientMsg::Admit {
            window_id: b,
            tool_use_id: "b".into(),
            slots: None,
            wait_secs: W.as_secs(),
            label: "agent w2 cargo test".into(),
        };
        proto::write_frame(&mut stream, &admit).await.unwrap();
        stream
    });
    rig.wait_for("admit asked", "b");

    let start = Instant::now();
    let waiting = rig.admit(b, W.as_secs(), &bash(Some("c"), "cargo build"));
    rig.wait_for("admit asked", "c");
    rt.block_on(async move { drop(gone) });
    rig.wait_for("admit abandoned", "b");

    rig.hook(a, &event("PostToolUse", Some("a")));
    finish(waiting, W);
    assert!(start.elapsed() < W, "{:?}", start.elapsed());
    rig.wait_for("admit granted", "c");
    assert_eq!(rig.lines("admit granted", "b"), 0, "{}", rig.log());
}

/// Decision 22 (C8 (b)): the sweep releases the holds of a window that is no longer
/// listed. Bound: B's W (60 s); the release path's legal worst case is the removal
/// itself plus one `ADMIT_SWEEP` (10 s) after it; the loop is capped at W.
#[test]
fn removing_the_window_releases_its_holds() {
    assert!(ADMIT_SWEEP * 2 < W);
    let rig = Rig::start();
    let (a, b) = (rig.named("a"), rig.named("b"));
    let (waiting, start) = held_and_waiting(&rig, a, b);
    let removed = rig.daemon.anthrex(&["rm", &a.to_string()]);
    assert!(removed.status.success(), "{removed:?}");
    finish(waiting, W);
    assert!(start.elapsed() < W, "{:?}", start.elapsed());
    rig.wait_for_within("admit released", "a", ADMIT_SWEEP + WALL_CLOCK_SLACK);
}
