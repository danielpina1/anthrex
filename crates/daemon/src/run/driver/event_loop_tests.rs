//! Decision 8 (M9.11.8, B1): a panic in an event's handling after its step, in the
//! effects (`execute`) or in the tick's own work (`on_tick`), costs that event only.
//! The event loop keeps receiving, a waiting caller is answered, and ticks go on.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use tokio_util::sync::CancellationToken;

use super::super::effects::Ready;
use super::super::guard::EFFECTS_PANICKED;
use super::super::{GitRoots, RunService};
use crate::manager::{ManagerConfig, WindowManager};
use crate::run::engine::{AgentSignal, EventKind};
use crate::run::test_support::{PROFILE, plan_with, run_ok, task_toml};

/// The driver's ticker period (`RunService::spawn`'s `interval`).
const TICK: Duration = Duration::from_secs(1);
/// The workspace's allowance for a loaded machine (`docs/timing-budgets.md`).
const WALL_CLOCK_SLACK: Duration = Duration::from_secs(5);
/// Every wait below: an answered request costs one step in memory and at most one
/// `run.json` save on a blocking thread, and a tick is due at most one `TICK` after the
/// last; the failure these tests catch is a hang (no answer, no tick, ever), so the
/// bound only has to exceed that legal worst case.
const REQUEST_WAIT: Duration = Duration::from_secs(TICK.as_secs() + WALL_CLOCK_SLACK.as_secs());

struct NoRoots;
impl GitRoots for NoRoots {
    fn register(&self, _: std::path::PathBuf) {}
    fn unregister(&self, _: &std::path::Path) {}
}

fn service(data: &std::path::Path) -> Arc<RunService> {
    let mut config = ManagerConfig::for_tests(data.join("d.sock"), "/bin/sh".into());
    config.claude_bin = "/nonexistent/anthrex-test/claude".into();
    config.codex_bin = "/nonexistent/anthrex-test/codex".into();
    config.decider_bin = Some("/nonexistent/anthrex-test/decider".into());
    let (manager, _events) = WindowManager::new(config);
    RunService::for_manager(&manager, data.join("data"), Arc::new(NoRoots))
}

/// Counts the ticks that reach `check_orchestrators` (its `between_reads` hook), and
/// panics on the first when `panic_first` is set.
fn count_ticks(s: &RunService, panic_first: bool) -> Arc<AtomicUsize> {
    let ticks = Arc::new(AtomicUsize::new(0));
    let seen = ticks.clone();
    *crate::lock(&s.wakes.between_reads) = Some(Box::new(move || {
        if seen.fetch_add(1, Ordering::SeqCst) == 0 && panic_first {
            panic!("injected: the tick's work panicked");
        }
    }));
    ticks
}

async fn wait_for_more_ticks(ticks: &AtomicUsize, than: usize) {
    let deadline = Instant::now() + REQUEST_WAIT;
    while ticks.load(Ordering::SeqCst) <= than {
        assert!(Instant::now() < deadline, "no tick ran after the panic");
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

/// A request that goes through the loop: cancelling a run that does not exist answers
/// with a refusal, but only once the loop has stepped it.
async fn loop_answers(s: &Arc<RunService>) {
    let answer = tokio::time::timeout(
        REQUEST_WAIT,
        s.ask(|reply| EventKind::Cancel {
            reply,
            run_id: "no-such-run".into(),
        }),
    )
    .await
    .expect("the loop answers a request after the panic");
    assert!(answer.is_err(), "{answer:?}");
    assert_ne!(answer.unwrap_err(), "the daemon is shutting down");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_panic_in_execute_keeps_the_loop() {
    let dir = tempfile::tempdir().unwrap();
    let s = service(dir.path());
    let ticks = count_ticks(&s, false);
    let fired = Arc::new(AtomicBool::new(false));
    let once = fired.clone();
    *crate::lock(&s.execute_hook) = Some(Box::new(move |item: &Ready| {
        let panicky = match item {
            Ready::Save(run) => run.id == "panicky",
            Ready::Dirty(id) => id == "panicky",
            _ => false,
        };
        if panicky && !once.swap(true, Ordering::SeqCst) {
            panic!("injected: the first Persist of run panicky");
        }
    }));
    let shutdown = CancellationToken::new();
    s.spawn(shutdown.clone());

    let mut run = run_ok(&plan_with(
        PROFILE,
        &[task_toml("t1", "S", "[\"crates/a/**\"]", "")],
    ));
    run.id = "panicky".into();
    run.data_dir = dir.path().join("runs").join("panicky");
    std::fs::create_dir_all(&run.data_dir).unwrap();
    let started = tokio::time::timeout(
        REQUEST_WAIT,
        s.ask(|reply| EventKind::Start {
            reply,
            run: Box::new(run),
        }),
    )
    .await
    .expect("the start's caller is answered, not left waiting");
    assert!(fired.load(Ordering::SeqCst), "the hook never fired");
    assert_eq!(started, Err(EFFECTS_PANICKED.to_string()));

    // The step was committed before its effects panicked: the list shows the run.
    let listed = s.current();
    assert!(
        listed.runs.iter().any(|r| r.run_id == "panicky"),
        "{listed:?}"
    );
    loop_answers(&s).await;
    let now = ticks.load(Ordering::SeqCst);
    wait_for_more_ticks(&ticks, now).await;
    s.stop().await;
    shutdown.cancel();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_panic_in_on_tick_keeps_the_loop() {
    let dir = tempfile::tempdir().unwrap();
    let s = service(dir.path());
    let ticks = count_ticks(&s, true);
    let shutdown = CancellationToken::new();
    s.spawn(shutdown.clone());
    // The first tick panics inside `on_tick`.
    wait_for_more_ticks(&ticks, 0).await;
    loop_answers(&s).await;
    wait_for_more_ticks(&ticks, 1).await;
    s.stop().await;
    shutdown.cancel();
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

fn headless_spec(cwd: &std::path::Path, role: &str) -> crate::headless::HeadlessSpec {
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

async fn wait_listed(m: &crate::manager::WindowManager, id: u32, status: proto::Status) {
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

/// A stand-in script's run, from its spawn to its last line read: the scripts below are
/// run once first (`warm`), so macOS's first exec of a fresh file
/// (`docs/timing-budgets.md`, "Measured primitive costs") is paid before this starts;
/// the rest is a `/bin/sh` printing a file, well under `WALL_CLOCK_SLACK`.
const LAUNCH_WAIT: Duration = WALL_CLOCK_SLACK;

fn warm(script: &std::path::Path) {
    let status = std::process::Command::new(script)
        .env("STAND_IN_ROLE", "warm")
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .status()
        .expect("the stand-in runs");
    assert!(status.success());
}

/// Decision 9 (M9.11.9, A12): a forwarder that lagged still gives the engine a
/// session's exit and its last turn end. The feed holds 4 signals; the forwarder's
/// receiver is taken before window `a` starts, and the forwarder is spawned only after
/// `a` has ended its turn and exited and window `b` has then sent more than 4 signals
/// of its own (an init, 8 lines, a turn end), so `a`'s are gone from the feed and the
/// first receive is `Lagged`: the injected lag.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_lagged_feed_still_delivers_the_exit() {
    let dir = tempfile::tempdir().unwrap();
    let a_turn = dir.path().join("a.jsonl");
    std::fs::write(&a_turn, claude_turn("sess-a", 1)).unwrap();
    let b_turn = dir.path().join("b.jsonl");
    std::fs::write(&b_turn, claude_turn("sess-b", 8)).unwrap();
    // `a` ends one turn and exits; `b` ends one turn of 8 lines and stays; `warm` exits.
    let claude = testexec::write_executable(
        dir.path().join("claude"),
        format!(
            "#!/bin/sh\ncase \"$STAND_IN_ROLE\" in\n  a) cat '{}' ;;\n  b) cat '{}'; exec sleep 30 ;;\nesac\nexit 0\n",
            a_turn.display(),
            b_turn.display()
        ),
    );
    warm(&claude);

    let mut config = ManagerConfig::for_tests(dir.path().join("d.sock"), "/bin/sh".into());
    config.claude_bin = claude.to_str().unwrap().into();
    config.codex_bin = "/nonexistent/anthrex-test/codex".into();
    config.decider_bin = Some("/nonexistent/anthrex-test/decider".into());
    config.signal_capacity = 4;
    let (manager, mut events) = WindowManager::new(config);
    let pump = manager.clone();
    tokio::spawn(async move {
        while let Some((id, event)) = events.recv().await {
            pump.handle_event(id, event);
        }
    });
    let s = RunService::for_manager(&manager, dir.path().join("data"), Arc::new(NoRoots));
    let mut engine = crate::lock(&s.rx).take().expect("the engine's queue");
    let feed = manager.signals();

    let create = async |name: &str, uuid: &str| {
        manager
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
            .expect("create_headless")
    };
    let a = create("a", "00000000-0000-4000-8000-00000000000a").await;
    wait_listed(&manager, a.id, proto::Status::Exited).await;
    let b = create("b", "00000000-0000-4000-8000-00000000000b").await;
    wait_listed(&manager, b.id, proto::Status::Idle).await;

    let shutdown = CancellationToken::new();
    tokio::spawn(s.clone().forward(feed, shutdown.clone()));
    let (mut exited, mut turn_ended) = (0, 0);
    let deadline = Instant::now() + REQUEST_WAIT;
    while exited == 0 || turn_ended == 0 {
        let left = deadline.saturating_duration_since(Instant::now());
        let Ok(Some(msg)) = tokio::time::timeout(left, engine.recv()).await else {
            panic!("the engine never had a's exit ({exited}) and turn end ({turn_ended})");
        };
        let super::super::Msg::Event(EventKind::Signal { window_id, signal }) = msg else {
            continue;
        };
        if window_id != a.id {
            continue;
        }
        match signal {
            AgentSignal::ProcessExited { .. } => exited += 1,
            AgentSignal::TurnEnded { .. } => turn_ended += 1,
            _ => {}
        }
    }
    shutdown.cancel();
    let _ = manager.remove(b.id);
    let _ = manager.remove(a.id);
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
    let caught = forwarded.catch_up(&records);
    assert_eq!(
        caught,
        vec![
            live(4, 80, &turn_end("eighty three")),
            live(4, 80, &exit(9))
        ]
    );
    assert!(forwarded.catch_up(&records).is_empty(), "nothing twice");
}
