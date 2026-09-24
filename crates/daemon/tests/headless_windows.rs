//! M8a.17: headless agent sessions as manager windows with no terminal (decisions 27,
//! 28 and 49), against a real manager and real `/bin/sh` stand-ins for the CLIs.
//!
//! Split by seam (AGENTS.md rule 8): `headless_windows/hooks.rs` holds the real-hook and
//! conversation tests, `headless_windows/socket.rs` the client refusals over a socket.

mod support;

#[path = "headless_windows/hooks.rs"]
mod hooks;
#[path = "headless_windows/socket.rs"]
mod socket;

use daemon::headless::SessionEvent;
use daemon::launch::LaunchGate;
use daemon::manager::{WindowSignal, WindowSignalKind};
use daemon::state::WindowRecord;
use proto::{AgentRole, Block, Role, Runtime, Status, WindowKind};
use std::time::Duration;
use support::headless::*;

/// The session id of `claude-2.1.278-stream.jsonl`'s `system/init`.
const CLAUDE_SESSION: &str = "00000000-0000-4000-8000-000000000003";
/// The `thread_id` of the first run in `codex-0.155.0-exec.jsonl`.
const CODEX_SESSION: &str = "00000000-0000-4000-8000-000000000157";

fn is_exit(signal: &WindowSignal) -> bool {
    matches!(
        signal.kind,
        WindowSignalKind::Session(SessionEvent::ProcessExited { .. })
    )
}

#[tokio::test]
async fn create_headless_registers_a_headless_window() {
    let dir = tempfile::tempdir().unwrap();
    let claude = script(
        dir.path(),
        "claude",
        &format!(
            "cat '{}'; exec sleep 30",
            fixture("claude-2.1.278-stream.jsonl").display()
        ),
    );
    let m = manager(&claude, &claude, |_| {});
    let info = create(&m, "w1", spec(Runtime::Claude, dir.path()), "hello").await;
    assert_eq!(info.kind, WindowKind::Headless);
    let run = info.run.clone().expect("a run window names its run");
    assert_eq!(run.role, AgentRole::Worker);
    assert_eq!(run, run_ref());
    assert_eq!(info.runtime, Runtime::Claude);
    wait_until("Init's session id", || {
        find(&m, info.id).session_id.as_deref() == Some(CLAUDE_SESSION)
    })
    .await;
    assert_eq!(
        m.headless_spec(info.id),
        Some(spec(Runtime::Claude, dir.path()))
    );
    m.remove(info.id).unwrap();
}

#[tokio::test]
async fn session_events_drive_status_and_the_conversation() {
    let dir = tempfile::tempdir().unwrap();
    let exec = fixture("codex-0.155.0-exec.jsonl");
    let codex = script(
        dir.path(),
        "codex",
        &format!(
            "{}\nsed -n 1,2p '{exec}'\n{}\nsed -n 3,8p '{exec}'",
            gate(dir.path(), "go1"),
            gate(dir.path(), "go2"),
            exec = exec.display()
        ),
    );
    let m = manager(&codex, &codex, |_| {});
    let mut feed = m.signals();
    let prompt = "Create a.txt";
    let info = create(&m, "c1", spec(Runtime::Codex, dir.path()), prompt).await;
    assert_eq!(find(&m, info.id).status, Status::Starting);

    open_gate(dir.path(), "go1");
    wait_until("Working", || find(&m, info.id).status == Status::Working).await;
    assert_eq!(find(&m, info.id).session_id.as_deref(), Some(CODEX_SESSION));
    open_gate(dir.path(), "go2");
    next_signal(&mut feed, "the exit", is_exit).await;
    // Codex runs one process per turn: its exit after a completed turn does not end
    // the session, so the window stays `Idle`, not `Exited`.
    assert_eq!(find(&m, info.id).status, Status::Idle);

    let conversation = m
        .conversation_snapshot(info.id, None)
        .expect("a conversation");
    assert_eq!(conversation.degraded, None);
    let roles: Vec<Role> = conversation.turns.iter().map(|t| t.role).collect();
    assert_eq!(roles, [Role::User, Role::Assistant]);
    assert_eq!(
        conversation.turns[0].blocks,
        [Block::Text {
            text: prompt.into()
        }]
    );
    assert!(
        conversation.turns[1]
            .blocks
            .iter()
            .any(|b| matches!(b, Block::ToolCall { name, .. } if name == "Bash")),
        "{:?}",
        conversation.turns[1].blocks
    );
}

#[tokio::test]
async fn every_session_event_reaches_the_feed_with_its_pid_in_order() {
    let dir = tempfile::tempdir().unwrap();
    let exec = fixture("codex-0.155.0-exec.jsonl");
    let codex = script(
        dir.path(),
        "codex",
        &format!("sed -n 1,8p '{}'", exec.display()),
    );
    let m = manager(&codex, &codex, |_| {});
    let mut feed = m.signals();
    let info = create(&m, "c2", spec(Runtime::Codex, dir.path()), "go").await;
    let mut got = Vec::new();
    loop {
        let signal = next_signal(&mut feed, "a session event", |_| true).await;
        assert_eq!(signal.window_id, info.id);
        let done = is_exit(&signal);
        got.push(signal);
        if done {
            break;
        }
    }
    let WindowSignalKind::Session(SessionEvent::ProcessStarted { pid }) = got[0].kind else {
        panic!("ProcessStarted comes first: {got:?}");
    };
    assert!(got.iter().all(|s| s.pid == Some(pid)), "{got:?}");
    let events: Vec<SessionEvent> = got
        .into_iter()
        .skip(1)
        .map(|s| match s.kind {
            WindowSignalKind::Session(event) => event,
            other => panic!("{other:?}"),
        })
        .collect();
    let text = std::fs::read_to_string(&exec).unwrap();
    let mut expected: Vec<SessionEvent> = text
        .lines()
        .take(8)
        .flat_map(daemon::headless::codex_stream::parse_line)
        .collect();
    expected.push(SessionEvent::ProcessExited {
        code: Some(0),
        signal: None,
    });
    assert_eq!(events, expected);
}

#[tokio::test]
async fn headless_windows_persist_and_restore_as_ended() {
    let dir = tempfile::tempdir().unwrap();
    let claude = script(
        dir.path(),
        "claude",
        &format!(
            "cat '{}'; exec sleep 30",
            fixture("claude-2.1.278-stream.jsonl").display()
        ),
    );
    let m = manager(&claude, &claude, |_| {});
    let info = create(&m, "w1", spec(Runtime::Claude, dir.path()), "hello").await;
    wait_until("the session id", || find(&m, info.id).session_id.is_some()).await;
    let state = m.state_snapshot();
    m.remove(info.id).unwrap();
    let record = state.windows.iter().find(|r| r.id == info.id).unwrap();
    assert_eq!(record.kind, WindowKind::Headless);
    assert!(record.run.is_some());

    let restored = manager(&claude, &claude, |_| {});
    restored.restore(state.clone());
    let window = find(&restored, info.id);
    assert_eq!(window.kind, WindowKind::Headless);
    assert_eq!(window.status, Status::Exited);
    assert_eq!(window.session_id.as_deref(), Some(CLAUDE_SESSION));
    assert_eq!(window.run, Some(run_ref()));
    assert_eq!(
        restored.headless_spec(info.id),
        Some(spec(Runtime::Claude, dir.path()))
    );
    assert_eq!(restored.child_pid(info.id).unwrap(), None);

    // An unparseable `run` loads as an exited PTY record (with a warning in the log).
    let mut broken = state;
    let record: &mut WindowRecord = broken.windows.iter_mut().find(|r| r.id == info.id).unwrap();
    record.run = Some(serde_json::json!({"runtime": "not a runtime"}));
    let fallback = manager(&claude, &claude, |_| {});
    fallback.restore(broken);
    let window = find(&fallback, info.id);
    assert_eq!(window.kind, WindowKind::Pty);
    assert_eq!(window.status, Status::Exited);
    assert_eq!(window.run, None);
    assert_eq!(fallback.headless_spec(info.id), None);
}

#[tokio::test]
async fn create_headless_waits_for_the_launch_gate() {
    let dir = tempfile::tempdir().unwrap();
    let args = dir.path().join("args");
    let claude = script(
        dir.path(),
        "claude",
        &format!("echo \"$@\" > '{}'; exec sleep 30", args.display()),
    );
    let gate = LaunchGate::closed();
    let m = manager(&claude, &claude, |c| c.launch_gate = gate.clone());
    let creating = {
        let m = m.clone();
        let spec = spec(Runtime::Claude, dir.path());
        tokio::spawn(async move { create(&m, "gated", spec, "hi").await })
    };
    // An absence: nothing may spawn while the gate is closed. The positive half below
    // proves the same path does spawn once it opens.
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert!(!args.exists(), "spawned before the launch gate opened");
    assert!(
        m.list().is_empty(),
        "the manager still answers, with nothing created"
    );
    gate.open();
    let info = tokio::time::timeout(DEADLINE, creating)
        .await
        .expect("create_headless returns once the gate opens")
        .unwrap();
    wait_until("the args file", || args.exists()).await;
    m.remove(info.id).unwrap();
}

#[tokio::test]
async fn tick_leaves_a_headless_windows_status_alone() {
    let dir = tempfile::tempdir().unwrap();
    let stream = fixture("claude-2.1.278-stream.jsonl");
    let init = std::fs::read_to_string(&stream)
        .unwrap()
        .lines()
        .find(|l| l.contains("\"subtype\":\"init\""))
        .unwrap()
        .to_string();
    let first = dir.path().join("init.jsonl");
    std::fs::write(&first, format!("{init}\n")).unwrap();
    let claude = script(
        dir.path(),
        "claude",
        &format!("cat '{}'; exec sleep 30", first.display()),
    );
    let m = manager(&claude, &claude, |_| {});
    let info = create(&m, "busy", spec(Runtime::Claude, dir.path()), "go").await;
    let shell = m
        .create(
            proto::WindowSpec {
                name: Some("pty".into()),
                runtime: Runtime::Shell,
                cwd: std::env::temp_dir(),
                worktree_branch: None,
                model: None,
                initial_prompt: None,
            },
            std::env::temp_dir(),
            None,
            80,
            24,
        )
        .await
        .unwrap();
    wait_until("both Working", || {
        find(&m, info.id).status == Status::Working && find(&m, shell.id).status == Status::Working
    })
    .await;
    // Past the PTY rule's quiet time, with no stream event since the `Init`.
    tokio::time::sleep(daemon::manager::QUIET_AFTER + Duration::from_millis(300)).await;
    m.tick();
    assert_eq!(
        find(&m, shell.id).status,
        Status::Idle,
        "the PTY rule still runs"
    );
    assert_eq!(find(&m, info.id).status, Status::Working);
    m.remove(info.id).unwrap();
    m.remove(shell.id).unwrap();
}
