//! M8a.22: a session the engine stops ends its window (decision 52). A retired or killed
//! Codex window between turns has no process to exit, so it is `Exited` at once (ruling
//! T18-N3 for the kill during a send's jitter); a retired Claude window exits on its
//! stdin's EOF.

use super::support::headless::*;
use super::{Cleanup, claude_recorder, codex_recorder, is_exit, is_turn_end};
use proto::{Runtime, Status};
use std::time::Duration;

fn status(m: &daemon::manager::WindowManager, id: u32) -> Status {
    m.list()
        .into_iter()
        .find(|w| w.id == id)
        .expect("the window is listed")
        .status
}

/// Ruling T18-N3: a kill while a Codex send waits out its launch jitter stops the send
/// and leaves the window `Exited`; before, it stayed at the ended turn's status.
#[tokio::test]
async fn a_kill_during_a_codex_send_jitter_leaves_the_window_exited() {
    let dir = tempfile::tempdir().unwrap();
    let codex = codex_recorder(dir.path(), "");
    let m = manager(&codex, &codex, |c| {
        c.codex_turn_jitter = Some(Duration::from_secs(60))
    });
    let _cleanup = Cleanup(m.clone());
    let mut feed = m.signals();
    let info = create(&m, "c", spec(Runtime::Codex, dir.path()), "first").await;
    let id = info.id;
    next_signal(&mut feed, "the first exit", is_exit).await;
    assert_ne!(status(&m, id), Status::Exited);

    let sending = m.clone();
    let send = tokio::spawn(async move { sending.headless_send(id, "second").await });
    // The send claims the window before its first wait (a current-thread runtime).
    for _ in 0..10 {
        tokio::task::yield_now().await;
    }
    m.headless_kill(id).unwrap();
    let result = tokio::time::timeout(DEADLINE, send)
        .await
        .expect("the kill ends the send's wait")
        .unwrap();
    assert!(result.is_err(), "{result:?}");
    assert_eq!(status(&m, id), Status::Exited);
}

/// Decision 52's retirement of a Codex window between turns: nothing is running, so the
/// window is `Exited` at once, for the watcher's `RETIRE_AFTER`.
#[tokio::test]
async fn a_retired_codex_window_between_turns_is_exited() {
    let dir = tempfile::tempdir().unwrap();
    let codex = codex_recorder(dir.path(), "");
    let m = manager(&codex, &codex, |_| {});
    let _cleanup = Cleanup(m.clone());
    let mut feed = m.signals();
    let info = create(&m, "c", spec(Runtime::Codex, dir.path()), "first").await;
    next_signal(&mut feed, "the first exit", is_exit).await;
    assert_ne!(status(&m, info.id), Status::Exited);
    m.headless_retire(info.id).unwrap();
    assert_eq!(status(&m, info.id), Status::Exited);
}

/// A retired Claude window's process ends on its stdin's EOF, and the window is
/// `Exited`.
#[tokio::test]
async fn a_retired_claude_window_exits_on_eof() {
    let dir = tempfile::tempdir().unwrap();
    let claude = claude_recorder(dir.path(), false);
    let m = manager(&claude, &claude, |_| {});
    let _cleanup = Cleanup(m.clone());
    let mut feed = m.signals();
    let info = create(&m, "w", spec(Runtime::Claude, dir.path()), "first").await;
    next_signal(&mut feed, "the first turn's end", is_turn_end).await;
    let pid = m.headless_pid(info.id).expect("a process was started");
    m.headless_retire(info.id).unwrap();
    next_signal(&mut feed, "the exit", is_exit).await;
    wait_until("the window to be exited", || {
        status(&m, info.id) == Status::Exited
    })
    .await;
    assert_eq!(m.headless_pid(info.id), Some(pid));
}
