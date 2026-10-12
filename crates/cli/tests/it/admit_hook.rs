//! Milestone 9.14 (M9.14.8): `anthrex admit`, the worker's admission hook, against a
//! real daemon whose `[testing] test_slots = 1`, its agents pinned to `fake-agent`. Each
//! `anthrex admit` gets its payload on stdin, and stdin closed.

use crate::support;

use std::path::Path;
use std::process::Output;
use std::time::{Duration, Instant};

use daemon::admit::ADMIT_CLIENT_GRACE;
use proto::{HANDSHAKE_TIMEOUT, Runtime};
use serde_json::{Value, json};
use support::{RunningCommand, TestDaemon, assert_silent_success, isolated_command, tempdir};

/// As `hook_command.rs`'s `LIMIT`: no daemon constant is on this path, only an
/// `anthrex` spawn (≤ 85 ms loaded, `docs/timing-budgets.md`) and an immediate
/// `ECONNREFUSED` or an exit before connecting; 3 s is clear of both by a wide margin.
const LIMIT: Duration = Duration::from_secs(3);
/// Room for a loaded machine on top of a bound's legal worst case.
const WALL_CLOCK_SLACK: Duration = Duration::from_secs(10);

/// A daemon with one test slot; its config and the config's directory live as long as it.
struct Rig {
    daemon: TestDaemon,
    _config: tempfile::TempDir,
}

impl Rig {
    fn start() -> Rig {
        let config = tempdir();
        let path = config.path().join("config.toml");
        std::fs::write(&path, "[testing]\ntest_slots = 1\n").unwrap();
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
        self.daemon.client().create(Runtime::Claude, "agent")
    }

    /// `anthrex admit --window <window> --wait-secs <wait>` with `payload` on stdin.
    fn admit(&self, window: u32, wait: u64, payload: &Value) -> RunningCommand {
        let (window, wait) = (window.to_string(), wait.to_string());
        let args = ["admit", "--window", &window, "--wait-secs", &wait];
        let mut child = RunningCommand::start(&mut self.daemon.command(&args));
        child.input(payload.to_string().into_bytes());
        child
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
        let deadline = Instant::now() + LIMIT;
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
    finish(rig.admit(window, 60, &bash(Some("a"), "cargo test")), LIMIT);
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
    finish(rig.admit(window, 60, &payload), LIMIT);
    rig.wait_for("admit granted", "a");
    let wait = Duration::from_secs(60);
    let start = Instant::now();
    finish(rig.admit(window, 60, &payload), wait);
    assert!(start.elapsed() < wait, "{:?}", start.elapsed());
    rig.wait_for("admit already held", "a");
    assert_eq!(rig.lines("admit granted", "a"), 1, "{}", rig.log());
    assert_eq!(rig.lines("admit already held", "a"), 1, "{}", rig.log());
}
