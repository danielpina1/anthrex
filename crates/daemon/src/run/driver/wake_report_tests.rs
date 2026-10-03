//! Milestone 9.5 decisions 39 and 41 (task M9.5.5b), the driver's half: a quiet
//! orchestrator window that has sent no signal is reported as a start prompt, and why a
//! wake-up waits is logged once per reason. The rig of `wake_first_turn_tests.rs`: a real
//! manager and a real PTY window whose `claude` is a stand-in shell script (never an
//! agent); the engine's loop is not running, so what the driver sends is read off its
//! channel and, where the test needs the engine's answer, applied with `engine::step`.

use std::time::{Duration, Instant};

use proto::{Runtime, Status, WindowInfo, WindowKind};

use super::super::first_turn::tests::{Rig, SLACK, rig};
use super::super::*;
use crate::manager::QUIET_AFTER;
use crate::run::engine::Event;
use crate::run::orch::START_PROMPT;

/// Run `r1`, its live orchestrator in `window`, its wake-ups held for `quiet` seconds
/// of quiet; no first turn pending.
fn live_run(rig: &Rig, window: u32, quiet: u64) {
    let mut run = crate::run::orch::test_support::run_of(1);
    run.id = "r1".into();
    run.limits.orch.wake_quiet_secs = quiet;
    let mut o = crate::run::orch::test_support::orchestrator();
    o.window_id = Some(window);
    o.launch_op = None;
    o.live = true;
    o.launches = 1;
    run.orch.orchestrator = Some(o);
    crate::lock(&rig.runs.state).runs.insert("r1".into(), run);
}

/// Every `StartPrompt` and plain `OrchestratorWoken` the driver sent since the last
/// call, as `(kind, waiting)`.
fn sent(rig: &mut Rig) -> Vec<(&'static str, bool)> {
    let mut seen = Vec::new();
    while let Ok(msg) = rig.events.try_recv() {
        match msg {
            crate::run::driver::Msg::Event(EventKind::Orch(OrchEvent::StartPrompt {
                run_id,
                waiting,
            })) => {
                assert_eq!(run_id, "r1");
                seen.push(("start prompt", waiting));
            }
            crate::run::driver::Msg::Event(EventKind::Orch(OrchEvent::OrchestratorWoken {
                first_turn: false,
                ..
            })) => seen.push(("woken", true)),
            _ => {}
        }
    }
    seen
}

/// The engine's step for `waiting`, applied to the rig's state: run `r1`'s attention
/// lines afterwards.
fn applied(rig: &Rig, waiting: bool) -> Vec<String> {
    let state = crate::lock(&rig.runs.state).clone();
    let kind = EventKind::Orch(OrchEvent::StartPrompt {
        run_id: "r1".into(),
        waiting,
    });
    let (state, _) = crate::run::engine::step(state, Event { now: 2_000, kind });
    let lines = crate::run::snapshot::attention(&state.runs["r1"], 2_000);
    *crate::lock(&rig.runs.state) = state;
    lines
}

/// Waits, ticking the manager, until `window` is `Idle` with no signal: the stand-in
/// printed a line and went quiet, an agent at a prompt.
async fn idle_unsignalled(rig: &Rig, window: u32) {
    let deadline = Instant::now() + QUIET_AFTER + SLACK;
    while rig.status(window) != (Status::Idle, false) {
        assert!(Instant::now() < deadline, "{:?}", rig.status(window));
        rig.manager.tick();
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

fn wait_logs(rig: &Rig) -> Vec<String> {
    crate::lock(&rig.runs.wakes.waits.lines).clone()
}

/// Decision 39: a Claude orchestrator window `Idle` with no signal for the run's
/// `wake_quiet_secs` is reported once as a start prompt, which the snapshot shows as
/// `START_PROMPT` exactly; its first signal clears it.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_quiet_unsignalled_orchestrator_is_a_start_prompt() {
    let mut rig = rig();
    let window = rig.orchestrator_window("r1").await;
    idle_unsignalled(&rig, window).await;
    live_run(&rig, window, 1);
    let since = Instant::now();
    rig.runs.check_orchestrators();
    assert_eq!(sent(&mut rig), vec![], "reported before its quiet time");
    let mut reports = Vec::new();
    let deadline = since + Duration::from_secs(1) + SLACK;
    while reports.is_empty() {
        assert!(Instant::now() < deadline, "no start prompt reported");
        tokio::time::sleep(Duration::from_millis(50)).await;
        rig.runs.check_orchestrators();
        reports = sent(&mut rig);
    }
    assert!(
        since.elapsed() >= Duration::from_secs(1),
        "{:?}",
        since.elapsed()
    );
    for _ in 0..3 {
        rig.runs.check_orchestrators();
        reports.extend(sent(&mut rig));
    }
    assert_eq!(reports, vec![("start prompt", true)]);
    let lines = applied(&rig, true);
    assert_eq!(
        lines.iter().filter(|l| *l == START_PROMPT).count(),
        1,
        "{lines:?}"
    );
    assert!(lines.iter().any(|l| l == START_PROMPT), "{lines:?}");
    // Its first signal: no longer a start prompt.
    rig.hook(window, "SessionStart");
    assert_eq!(rig.status(window), (Status::Idle, true));
    rig.runs.check_orchestrators();
    rig.runs.check_orchestrators();
    assert_eq!(sent(&mut rig), vec![("start prompt", false)]);
    let lines = applied(&rig, false);
    assert!(!lines.iter().any(|l| l == START_PROMPT), "{lines:?}");
    let _ = rig.manager.kill(window);
}

/// Decision 41: a wake-up waiting on a `Working` window says so once however often it
/// is looked at, then once for the input that holds it next, then its delivery.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_waiting_wake_logs_why_once() {
    let mut rig = rig();
    let window = rig.orchestrator_window("r1").await;
    // The stand-in's line makes it `Working`; with no tick it stays so.
    let deadline = Instant::now() + SLACK;
    while rig.status(window).0 != Status::Working {
        assert!(Instant::now() < deadline, "{:?}", rig.status(window));
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    live_run(&rig, window, 4);
    rig.runs
        .queue_wake("r1".into(), window, "note".into(), (1, 1, None, false));
    rig.runs.check_orchestrators();
    rig.runs.check_orchestrators();
    assert_eq!(
        wait_logs(&rig),
        ["wake-up for run r1 waits: window working"]
    );
    // Its turn ends, then the user types: the quiet time holds it now.
    rig.hook(window, "Stop");
    assert_eq!(rig.status(window).0, Status::Done);
    rig.manager.write_client_input(window, b"x").unwrap();
    let typed = Instant::now();
    tokio::time::sleep_until((typed + Duration::from_secs(2)).into()).await;
    rig.runs.check_orchestrators();
    rig.runs.check_orchestrators();
    assert_eq!(
        wait_logs(&rig),
        [
            "wake-up for run r1 waits: window working",
            "wake-up for run r1 waits: input 2s ago",
        ]
    );
    assert_eq!(sent(&mut rig), vec![], "delivered inside the quiet time");
    // Past the quiet time it goes, and says so.
    let deadline = typed + Duration::from_secs(4) + SUBMIT_DELAY + SLACK;
    let mut woken = Vec::new();
    while woken.is_empty() {
        assert!(Instant::now() < deadline, "not delivered");
        tokio::time::sleep(Duration::from_millis(50)).await;
        rig.runs.check_orchestrators();
        woken = sent(&mut rig);
    }
    assert!(
        typed.elapsed() >= Duration::from_secs(4),
        "{:?}",
        typed.elapsed()
    );
    assert_eq!(
        wait_logs(&rig),
        [
            "wake-up for run r1 waits: window working".to_string(),
            "wake-up for run r1 waits: input 2s ago".to_string(),
            format!(
                "wake-up delivered to window {window} ({} bytes)",
                encode_paste("note").len()
            ),
        ]
    );
    let _ = rig.manager.kill(window);
}

fn window_info(status: Status) -> WindowInfo {
    WindowInfo {
        id: 1,
        name: "r1/orchestrator".into(),
        runtime: Runtime::Claude,
        cwd: "/tmp".into(),
        project: "/tmp".into(),
        worktree: None,
        branch: None,
        status,
        tool: None,
        since_secs: 0,
        last_output_secs: 0,
        session_id: None,
        model: None,
        subagents: Vec::new(),
        exit: None,
        kind: WindowKind::Pty,
        run: None,
        signals_seen: true,
        placeholder: false,
    }
}

/// Decision 41: `ready` says why a window does not take a paste, and each reason reads
/// as the log line's words.
#[test]
fn ready_reports_its_reason() {
    let quiet = Duration::from_secs(5);
    let ago = |secs| Instant::now().checked_sub(Duration::from_secs(secs));
    let idle = window_info(Status::Idle);
    assert_eq!(
        ready(&window_info(Status::Working), None, quiet, false),
        Err(WaitReason::Working)
    );
    assert_eq!(
        ready(&window_info(Status::Starting), None, quiet, false),
        Err(WaitReason::Starting)
    );
    assert_eq!(
        ready(&window_info(Status::Attention), None, quiet, false),
        Err(WaitReason::Attention)
    );
    assert_eq!(ready(&idle, None, quiet, true), Err(WaitReason::Attention));
    assert_eq!(
        ready(&idle, ago(2), quiet, false),
        Err(WaitReason::Input { secs: 2 })
    );
    assert_eq!(ready(&idle, ago(6), quiet, false), Ok(()));
    assert_eq!(
        ready(&window_info(Status::Done), None, quiet, false),
        Ok(())
    );
    for (reason, text) in [
        (WaitReason::Working, "window working"),
        (WaitReason::Starting, "window starting"),
        (WaitReason::Input { secs: 3 }, "input 3s ago"),
        (WaitReason::Attention, "attention open"),
        (WaitReason::NoSignal, "no hook signal yet"),
    ] {
        assert_eq!(reason.to_string(), text);
    }
}
