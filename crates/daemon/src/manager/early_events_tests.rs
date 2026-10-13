//! A window's events that arrive between `Window::spawn` (phase B) and `insert` (phase C).
//!
//! `pty-read-{id}` and `pty-wait-{id}` start inside phase B and send on the manager's
//! event channel at once, while the entry is only inserted in phase C, after
//! `spawn_blocking` returns and its future is polled again. These tests construct that
//! ordering directly: they keep the event receiver, pump it by hand through
//! `handle_event` until the event under test has passed, and only then run phase C. No
//! timing luck is involved: the event is handled before `insert` every time.

use super::create::{Admitted, spawn_window};
use super::{ManagerConfig, WindowEvent, WindowManager};
use proto::{Runtime, Status, WindowInfo, WindowSpec};
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::mpsc;

/// How long the stand-in may take to start and send the event under test: a PTY spawn
/// and a `/bin/sh` start, about 0.02 s warmed and 0.44 s for a fresh file's first exec
/// on macOS (`docs/timing-budgets.md`, "Measured primitive costs"); ten seconds is a
/// tenfold margin over that on a loaded host. The wait ends as soon as the event comes.
const EVENT_WAIT: Duration = Duration::from_secs(10);

fn spec(dir: &Path, runtime: Runtime) -> WindowSpec {
    WindowSpec {
        name: Some("early".into()),
        runtime,
        cwd: dir.to_path_buf(),
        worktree_branch: None,
        model: None,
        initial_prompt: None,
    }
}

/// Phases A and B of `create`, then every event the window sends handed to
/// `handle_event` until `until` matches one, then phase C. The events are handled
/// before the entry exists, which is the ordering under test.
async fn create_after_early_events(
    manager: &Arc<WindowManager>,
    events: &mut mpsc::UnboundedReceiver<(u32, WindowEvent)>,
    spec: WindowSpec,
    until: impl Fn(&WindowEvent) -> bool,
) -> WindowInfo {
    let project = spec.cwd.clone();
    let (id, reservation) = manager.admit(&spec, &project).unwrap();
    let name = reservation.name.clone();
    let config = manager.config.clone();
    let sender = manager.events.clone();
    let roots = crate::project::DetectedRoots {
        project: project.clone(),
        worktree: None,
        detection_failed: false,
    };
    let phase_b_name = name.clone();
    let spawned = tokio::task::spawn_blocking(move || {
        let admitted = Admitted {
            id,
            name: &phase_b_name,
            cols: 80,
            rows: 24,
        };
        spawn_window(&config, sender, spec, &roots, &admitted, None)
    })
    .await
    .unwrap()
    .unwrap();
    let mut seen = Vec::new();
    loop {
        let (event_id, event) = tokio::time::timeout(EVENT_WAIT, events.recv())
            .await
            .unwrap_or_else(|_| panic!("the stand-in never sent the event; saw {seen:?}"))
            .expect("the event channel stays open while the manager lives");
        assert_eq!(event_id, id);
        let done = until(&event);
        seen.push(format!("{event:?}"));
        manager.handle_event(event_id, event);
        if done {
            break;
        }
    }
    let info = manager
        .insert(id, name, project, None, spawned, reservation)
        .unwrap();
    // Every later event is pumped as the daemon does.
    let pump = manager.clone();
    let mut rest = std::mem::replace(events, mpsc::unbounded_channel().1);
    tokio::spawn(async move {
        while let Some((id, event)) = rest.recv().await {
            pump.handle_event(id, event);
        }
    });
    info
}

fn listed(manager: &WindowManager, id: u32) -> WindowInfo {
    manager.list().into_iter().find(|w| w.id == id).unwrap()
}

/// A program that fails at once: its `Exited` is sent before phase C inserts the
/// entry. Dropping it left the window `Starting`, with a live child, for good.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_exit_before_insert_is_applied_at_insert() {
    let dir = tempfile::tempdir().unwrap();
    let shell = dir.path().join("fails");
    testexec::write_executable(&shell, "#!/bin/sh\nexit 3\n");
    let config = ManagerConfig::for_tests(
        dir.path().join("unused.sock"),
        shell.to_str().unwrap().into(),
    );
    let (manager, mut events) = WindowManager::new(config);

    let info = create_after_early_events(
        &manager,
        &mut events,
        spec(dir.path(), Runtime::Shell),
        |event| matches!(event, WindowEvent::Exited { .. }),
    )
    .await;

    let window = listed(&manager, info.id);
    assert_eq!(window.status, Status::Exited, "{window:?}");
    assert_eq!(
        window.exit.as_ref().and_then(|e| e.code),
        Some(3),
        "{window:?}"
    );
}

/// A one-shot startup title, as a Codex session with no hooks gives: sent once, before
/// phase C. Dropping it left the window `Starting` with no signal seen.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_title_before_insert_is_applied_at_insert() {
    let dir = tempfile::tempdir().unwrap();
    let codex = dir.path().join("codex");
    // Waits on its input after the title, so it is still running at the assertion; the
    // PTY's close ends it when the manager is dropped.
    testexec::write_executable(
        &codex,
        "#!/bin/sh\nstty -echo 2>/dev/null\nprintf '\\033]0;Ready\\007'\nexec cat >/dev/null\n",
    );
    let mut config = ManagerConfig::for_tests(dir.path().join("unused.sock"), "/bin/sh".into());
    config.codex_bin = codex.to_str().unwrap().into();
    let (manager, mut events) = WindowManager::new(config);

    let info = create_after_early_events(
        &manager,
        &mut events,
        spec(dir.path(), Runtime::Codex),
        |event| matches!(event, WindowEvent::Title(_)),
    )
    .await;

    let window = listed(&manager, info.id);
    assert!(window.signals_seen, "{window:?}");
    assert_ne!(window.status, Status::Starting, "{window:?}");
    let _ = manager.kill(info.id);
}

/// A create that ends before phase C (an error, a panic, a dropped future) gives its
/// slot back with its name, so a later event for the spent id is dropped as before and
/// nothing is kept for an id no entry will ever have.
#[test]
fn a_create_that_never_inserts_gives_its_slot_back() {
    let dir = tempfile::tempdir().unwrap();
    let config = ManagerConfig::for_tests(dir.path().join("unused.sock"), "/bin/sh".into());
    let (manager, _events) = WindowManager::new(config);
    let (id, reservation) = manager
        .admit(&spec(dir.path(), Runtime::Shell), dir.path())
        .unwrap();
    manager.handle_event(id, WindowEvent::Output);
    assert_eq!(crate::lock(&manager.inner).early_events[&id].len(), 1);
    drop(reservation);
    manager.handle_event(id, WindowEvent::Output);
    assert!(crate::lock(&manager.inner).early_events.is_empty());
}
