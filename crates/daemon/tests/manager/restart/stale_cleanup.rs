//! Fix wave 5 review, Critical 1: `finish_restart` left the old child's cleanup record
//! under `id` in `Inner.cleanups`, so `Inner::start_cleanup` for the id's *new* process
//! short-circuited on the stale record and signalled nothing. Four probes on the same
//! stale-record bug: a plain `kill` after one restart, a second restart's own phase-B
//! kill, the `child_alive` stale-event race it reopens, and `shutdown`. A submodule of
//! `restart.rs` (AGENTS.md rule 8) sharing its `group_alive` helper via `use super::*`.

use super::*;

/// Critical 1 (fix wave 5 review): `finish_restart` left the old child's cleanup record
/// under `id` in `Inner.cleanups`, so `Inner::start_cleanup` for the id's *new* process
/// short-circuited on the stale record and signalled nothing. Probe 1: after one restart,
/// `kill(id)` on the window that came out of it must still actually end it, not sit
/// forever as a silent no-op — before the fix this timed out `wait_until`'s 8s deadline
/// every time, deterministically.
#[tokio::test]
async fn kill_after_a_restart_still_ends_the_window() {
    let m = manager();
    let id = create_id(&m, spec("kill-after-restart"), std::env::temp_dir(), 80, 24).await;
    wait_until("shell started", || find(&m, id).status != Status::Starting).await;

    m.restart(id).await.unwrap();
    wait_until("restarted window leaves Exited", || {
        find(&m, id).status != Status::Exited
    })
    .await;

    m.kill(id).unwrap();
    wait_until("the restarted window's own kill takes effect", || {
        find(&m, id).status == Status::Exited
    })
    .await;
}

/// Critical 1, probe 2: the same stale `cleanups` record that breaks probe 1's `kill`
/// also breaks `restart`'s own phase B kill for every restart after the first, since
/// phase B's kill is exactly a `start_cleanup` call. A *second* restart must actually
/// signal the process the *first* restart put in place — checked on the process group
/// directly, not just on a status transition, because a status of Exited can be reported
/// while the child is still alive (this module's own doc comment). Before the fix this
/// left the first restart's shell running as an orphan.
#[tokio::test]
async fn a_second_restart_actually_kills_the_first_restarts_process() {
    let m = manager();
    let id = create_id(&m, spec("double-restart"), std::env::temp_dir(), 80, 24).await;
    wait_until("shell started", || find(&m, id).status != Status::Starting).await;

    m.restart(id).await.unwrap();
    wait_until("first restart leaves Exited", || {
        find(&m, id).status != Status::Exited
    })
    .await;
    let first_restart_pid = m
        .child_pid(id)
        .unwrap()
        .expect("restarted window has a child");

    m.restart(id).await.unwrap();

    // `restart` only returns `Ok` once phase B's wait has confirmed `child_alive` false
    // (this module's doc comment on `wait_for_exit`), which is only ever set once
    // `WindowEvent::Exited` has been processed — and that event is sent only after
    // `child.wait()` has already reaped the pid (`window.rs`'s `pty-wait-{id}` thread).
    // So this needs no `wait_until`: by the time the second `restart` above returns, the
    // first restart's process is either already reaped or was never signalled at all.
    assert!(
        !group_alive(first_restart_pid),
        "the second restart must actually kill the process the first restart spawned, \
         not leave it running as an orphan"
    );
}

/// Critical 1, probe 3: the exact race the `child_alive` deviation exists to close
/// (`manager/restart.rs`'s module doc), reopened by the stale `cleanups` record. Without
/// the fix, the second restart's phase B kill signals nothing, so the first restart's
/// child eventually exits on its own real time and its `Exited` event lands — unfiltered
/// by id — on the *third* process now running under the same id, marking a live window
/// Exited.
#[tokio::test]
async fn no_stale_exit_reaches_a_twice_restarted_window() {
    let m = manager();
    let id = create_id(&m, spec("twice-restarted"), std::env::temp_dir(), 80, 24).await;
    wait_until("shell started", || find(&m, id).status != Status::Starting).await;

    m.restart(id).await.unwrap();
    wait_until("first restart leaves Exited", || {
        find(&m, id).status != Status::Exited
    })
    .await;
    m.restart(id).await.unwrap();
    wait_until("second restart leaves Exited", || {
        find(&m, id).status != Status::Exited
    })
    .await;

    // Give a stale `Exited` every chance to arrive before asserting it never did: with
    // the bug present this reproduced on the first try, deterministically, well inside
    // this margin.
    tokio::time::sleep(Duration::from_secs(2)).await;
    assert_ne!(
        find(&m, id).status,
        Status::Exited,
        "a stale Exited from an un-killed old child landed on the restarted window"
    );
}

/// Critical 1, probe 4: `shutdown`'s own `start_cleanup` call is exactly as vulnerable to
/// the stale record as `kill`'s — before the fix, a window that had been restarted once
/// left its live process behind when the daemon shut down, because `start_cleanup`
/// believed cleanup was already in hand for that id and signalled nothing.
#[tokio::test]
async fn shutdown_after_a_restart_ends_the_restarted_child() {
    let m = manager();
    let id = create_id(
        &m,
        spec("shutdown-after-restart"),
        std::env::temp_dir(),
        80,
        24,
    )
    .await;
    wait_until("shell started", || find(&m, id).status != Status::Starting).await;

    m.restart(id).await.unwrap();
    wait_until("restart leaves Exited", || {
        find(&m, id).status != Status::Exited
    })
    .await;
    let restarted_pid = m
        .child_pid(id)
        .unwrap()
        .expect("restarted window has a child");

    m.shutdown().await;

    assert!(
        !group_alive(restarted_pid),
        "shutdown must reach the process a restart put in this window's place, not just \
         the one `create` originally spawned"
    );
}
