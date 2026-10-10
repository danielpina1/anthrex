//! Decision 9 (M9.11.9, A12) and fix round 1's D9-I1: a forwarder that lagged gives
//! the engine every signal the feed still retained, each once, then the exits and last
//! turn ends the lag hid. Against a real manager and `/bin/sh` stand-ins (never an
//! agent), each run once first so macOS's first-exec stall is paid before any bound.

use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};

use tokio::sync::{broadcast, mpsc};
use tokio_util::sync::CancellationToken;

use super::super::{GitRoots, Msg, RunService};
use crate::manager::{ManagerConfig, WindowManager, WindowSignal};
use crate::run::engine::{AgentSignal, EventKind};

/// The workspace's allowance for a loaded machine (`docs/timing-budgets.md`).
const WALL_CLOCK_SLACK: Duration = Duration::from_secs(5);
/// A warmed `/bin/sh` stand-in printing one file, well under `WALL_CLOCK_SLACK`.
const LAUNCH_WAIT: Duration = WALL_CLOCK_SLACK;
/// The forwarder's catch-up is one pass over the old receiver and the records: memory
/// only, so the bound only has to exceed a scheduler's delay.
const FORWARD_WAIT: Duration = WALL_CLOCK_SLACK;

struct NoRoots;
impl GitRoots for NoRoots {
    fn register(&self, _: std::path::PathBuf) {}
    fn unregister(&self, _: &Path) {}
}

/// One Claude turn's stream as the headless reader parses it: `system/init`, `texts`
/// lines of prose, a success `result` (a `TurnEnded`).
fn claude_turn(session: &str, texts: usize) -> String {
    let mut lines = vec![format!(
        r#"{{"type":"system","subtype":"init","session_id":"{session}","model":"m"}}"#
    )];
    for n in 0..texts {
        let line = serde_json::json!({"type": "assistant", "parent_tool_use_id": null,
            "message": {"content": [{"type": "text", "text": format!("line {n}")}]}});
        lines.push(line.to_string());
    }
    lines.push(r#"{"type":"result","subtype":"success","is_error":false}"#.into());
    lines.join("\n") + "\n"
}

fn headless_spec(cwd: &Path, role: &str) -> crate::headless::HeadlessSpec {
    crate::headless::HeadlessSpec {
        runtime: proto::Runtime::Claude,
        model: "model-x".into(),
        effort: proto::Effort::MEDIUM,
        cwd: cwd.to_path_buf(),
        instructions: "the contract".into(),
        mcp: None,
        allowed_tools: vec!["Read".into()],
        claude_permission_mode: Some("acceptEdits".into()),
        claude_disallowed_tools: Vec::new(),
        claude_sandbox: None,
        codex_sandbox: "workspace-write".into(),
        codex_writable_roots: Vec::new(),
        codex_read_only: Vec::new(),
        codex_grant_dialect: None,
        env: vec![("STAND_IN_ROLE".into(), role.into())],
        claude_auth: config::ClaudeAuth::Login,
        api_key_helper: None,
        run_ref: None,
        codex_config_guard: None,
        output_filter: None,
    }
}

async fn wait_listed(m: &WindowManager, id: u32, status: proto::Status) {
    let deadline = Instant::now() + LAUNCH_WAIT;
    loop {
        let now = m.list().into_iter().find(|w| w.id == id).map(|w| w.status);
        if now == Some(status) {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "window {id} is {now:?}, not {status:?}"
        );
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

/// Kills and removes the windows a test made, on every path out of it (D9-M2).
struct Windows(Arc<WindowManager>, Vec<u32>);

impl Drop for Windows {
    fn drop(&mut self) {
        for id in &self.1 {
            let _ = self.0.headless_kill(*id);
            let _ = self.0.remove(*id);
        }
    }
}

/// The rig: a feed of `capacity`; window `a` ends one turn of `a_texts` lines and exits;
/// then window `b` ends one turn of `b_texts` lines and stays. The forwarder's receiver
/// is taken before `a` starts and is handed back unread, with the engine's queue: the
/// forwarder is spawned on it only after both windows are settled, so its first receive
/// is `Lagged` whenever more than `capacity` signals were sent (the injected lag).
struct Lagged {
    s: Arc<RunService>,
    engine: mpsc::UnboundedReceiver<Msg>,
    feed: broadcast::Receiver<WindowSignal>,
    a: u32,
    b: u32,
    _windows: Windows,
    _dir: tempfile::TempDir,
}

async fn lagged(capacity: usize, a_texts: usize, b_texts: usize) -> Lagged {
    let dir = tempfile::tempdir().unwrap();
    let a_turn = dir.path().join("a.jsonl");
    std::fs::write(&a_turn, claude_turn("sess-a", a_texts)).unwrap();
    let b_turn = dir.path().join("b.jsonl");
    std::fs::write(&b_turn, claude_turn("sess-b", b_texts)).unwrap();
    let claude = testexec::write_executable(
        dir.path().join("claude"),
        format!(
            "#!/bin/sh\ncase \"$STAND_IN_ROLE\" in\n  a) cat '{}' ;;\n  b) cat '{}'; exec sleep 30 ;;\nesac\nexit 0\n",
            a_turn.display(),
            b_turn.display()
        ),
    );
    let warmed = std::process::Command::new(&claude)
        .env("STAND_IN_ROLE", "warm")
        .stdin(std::process::Stdio::null())
        .status()
        .expect("the stand-in runs");
    assert!(warmed.success());

    let mut config = ManagerConfig::for_tests(dir.path().join("d.sock"), "/bin/sh".into());
    config.claude_bin = claude.to_str().unwrap().into();
    config.codex_bin = "/nonexistent/anthrex-test/codex".into();
    config.decider_bin = Some("/nonexistent/anthrex-test/decider".into());
    config.signal_capacity = capacity;
    let (manager, mut events) = WindowManager::new(config);
    let pump = manager.clone();
    tokio::spawn(async move {
        while let Some((id, event)) = events.recv().await {
            pump.handle_event(id, event);
        }
    });
    let s = RunService::for_manager(&manager, dir.path().join("data"), Arc::new(NoRoots));
    let engine = crate::lock(&s.rx).take().expect("the engine's queue");
    let feed = manager.signals();
    let mut windows = Windows(manager.clone(), Vec::new());
    let create = async |name: &str, uuid: &str| {
        let info = manager
            .create_headless(
                name.into(),
                headless_spec(dir.path(), name),
                crate::headless::SessionArg::New {
                    uuid: Some(uuid.into()),
                },
                "go".into(),
                dir.path().to_path_buf(),
                dir.path().to_path_buf(),
            )
            .await
            .expect("create_headless");
        info.id
    };
    let a = create("a", "00000000-0000-4000-8000-00000000000a").await;
    windows.1.push(a);
    wait_listed(&manager, a, proto::Status::Exited).await;
    let b = create("b", "00000000-0000-4000-8000-00000000000b").await;
    windows.1.push(b);
    wait_listed(&manager, b, proto::Status::Idle).await;
    Lagged {
        s,
        engine,
        feed,
        a,
        b,
        _windows: windows,
        _dir: dir,
    }
}

/// Every signal the forwarder sends until `until` holds of the ones sent so far, then
/// whatever else it had already queued.
async fn forwarded(
    rig: Lagged,
    mut until: impl FnMut(&[(u32, AgentSignal)]) -> bool,
) -> (Vec<(u32, AgentSignal)>, Lagged) {
    let mut rig = rig;
    let shutdown = CancellationToken::new();
    let feed = rig.feed.resubscribe();
    let feed = std::mem::replace(&mut rig.feed, feed);
    tokio::spawn(rig.s.clone().forward(feed, shutdown.clone()));
    let mut seen = Vec::new();
    let deadline = Instant::now() + FORWARD_WAIT;
    while !until(&seen) {
        let left = deadline.saturating_duration_since(Instant::now());
        let Ok(Some(msg)) = tokio::time::timeout(left, rig.engine.recv()).await else {
            panic!("the engine never had what the test waits for: {seen:?}");
        };
        if let Msg::Event(EventKind::Signal { window_id, signal }) = msg {
            seen.push((window_id, signal));
        }
    }
    // The forwarder sends one lag's signals in one pass, so the rest is queued already.
    tokio::task::yield_now().await;
    while let Ok(msg) = rig.engine.try_recv() {
        if let Msg::Event(EventKind::Signal { window_id, signal }) = msg {
            seen.push((window_id, signal));
        }
    }
    shutdown.cancel();
    (seen, rig)
}

fn count(seen: &[(u32, AgentSignal)], id: u32, what: fn(&AgentSignal) -> bool) -> usize {
    seen.iter().filter(|(w, s)| *w == id && what(s)).count()
}

fn exited(s: &AgentSignal) -> bool {
    matches!(s, AgentSignal::ProcessExited { .. })
}

fn turn_ended(s: &AgentSignal) -> bool {
    matches!(s, AgentSignal::TurnEnded { .. })
}

/// Decision 9: `a`'s exit and turn end are gone from a 4-signal feed (`b` sent an init,
/// 8 lines and a turn end after them), and still reach the engine.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_lagged_feed_still_delivers_the_exit() {
    let rig = lagged(4, 1, 8).await;
    let a = rig.a;
    let (seen, _rig) = forwarded(rig, |seen| {
        count(seen, a, exited) > 0 && count(seen, a, turn_ended) > 0
    })
    .await;
    assert_eq!(count(&seen, a, exited), 1, "{seen:?}");
    assert_eq!(count(&seen, a, turn_ended), 1, "{seen:?}");
}

/// D9-I1: a lag must not drop what the feed still retained. `a` sends more than the
/// 8-signal feed holds (an init, 10 lines, a turn end, its exit), then `b` sends its
/// start, its init and a turn end: those are retained at the lag, and `b`'s
/// `ProcessStarted` (the only source of a round's pid) and `Init` (of its session id)
/// must reach the engine, once each, with `b`'s turn end once (retained, so never
/// caught up again), and `a`'s exit (overflowed, so caught up) once.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_lag_still_delivers_the_retained_signals() {
    let rig = lagged(8, 10, 0).await;
    let (a, b) = (rig.a, rig.b);
    let (seen, _rig) = forwarded(rig, |seen| count(seen, a, exited) > 0).await;
    let started = |s: &AgentSignal| matches!(s, AgentSignal::ProcessStarted { .. });
    let init = |s: &AgentSignal| matches!(s, AgentSignal::Init { .. });
    assert_eq!(count(&seen, b, started), 1, "{seen:?}");
    assert_eq!(count(&seen, b, init), 1, "{seen:?}");
    assert_eq!(count(&seen, b, turn_ended), 1, "{seen:?}");
    assert_eq!(count(&seen, a, exited), 1, "{seen:?}");
}

/// Decision 9: an exit forwarded before a lag is not forwarded again by the catch-up,
/// and one the lag hid is forwarded once, however often the manager is asked; turn ends
/// the lag hid are coalesced into the process's last.
#[test]
fn a_lag_never_forwards_an_exit_twice() {
    use super::Forwarded;
    use crate::headless::{SessionEvent, TurnOutcome};
    use crate::manager::{TerminalRecord, WindowSignal, WindowSignalKind};
    let turn_end = |text: &str| SessionEvent::TurnEnded {
        outcome: TurnOutcome::Completed,
        usage: None,
        denials: vec![text.to_string()],
    };
    let exit = |code| SessionEvent::ProcessExited {
        code: Some(code),
        signal: None,
    };
    let live = |window_id, pid, event: &SessionEvent| WindowSignal {
        window_id,
        pid: Some(pid),
        kind: WindowSignalKind::Session(event.clone()),
    };
    let mut forwarded = Forwarded::default();
    // Window 3's process 70 ended a turn and exited, both forwarded before the lag.
    forwarded.note(&live(3, 70, &turn_end("seventy")));
    forwarded.note(&live(3, 70, &exit(0)));
    // Window 4's process 80 ended one turn before the lag; two more and its exit after.
    forwarded.note(&live(4, 80, &turn_end("eighty one")));
    let records = vec![
        TerminalRecord {
            window_id: 3,
            pid: 70,
            turn_ends: 1,
            last_turn_end: Some(turn_end("seventy")),
            exited: Some(exit(0)),
        },
        TerminalRecord {
            window_id: 4,
            pid: 80,
            turn_ends: 3,
            last_turn_end: Some(turn_end("eighty three")),
            exited: Some(exit(9)),
        },
    ];
    let caught = forwarded.catch_up(Vec::new(), &records);
    assert_eq!(
        caught,
        vec![
            live(4, 80, &turn_end("eighty three")),
            live(4, 80, &exit(9))
        ]
    );
    assert!(
        forwarded.catch_up(Vec::new(), &records).is_empty(),
        "nothing twice"
    );

    // D9-M1: the live path. Process 81's exit is forwarded as it arrives; a later lag's
    // catch-up does not send it again.
    forwarded.note(&live(4, 81, &exit(1)));
    let later = vec![TerminalRecord {
        window_id: 4,
        pid: 81,
        turn_ends: 0,
        last_turn_end: None,
        exited: Some(exit(1)),
    }];
    assert!(forwarded.catch_up(Vec::new(), &later).is_empty());

    // D9-I1: process 90 ended two turns; the lag lost the first and the feed retained the
    // second, which is its newest. It is sent once, as retained, and never caught up.
    let retained = vec![live(5, 90, &turn_end("ninety two"))];
    let records = vec![TerminalRecord {
        window_id: 5,
        pid: 90,
        turn_ends: 2,
        last_turn_end: Some(turn_end("ninety two")),
        exited: None,
    }];
    assert_eq!(forwarded.catch_up(retained.clone(), &records), retained);
    assert!(forwarded.catch_up(Vec::new(), &records).is_empty());
}
