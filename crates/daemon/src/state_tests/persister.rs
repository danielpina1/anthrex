//! `spawn_persister`'s own risk cases: decision 9's debounce loop is a classic place to
//! lose the last write, and the failure mode that matters is a user's final change never
//! reaching disk. `crates/daemon/tests/manager.rs`'s `persister_writes_changes_within_a_second`
//! already covers the acceptance case (a burst reaches disk as the final state, within a
//! second); the tests below target the specific ways a *naive* debouncer breaks that a
//! single burst-and-settle test cannot distinguish from a correct one: a debounce that
//! blocks cancellation, and a debounce that writes on every change instead of skipping a
//! no-op one.
//!
//! Not tested here: "a change arriving during the write". `save`'s own blocking write is
//! a handful of syscalls, far too fast to land a real change inside deterministically
//! without a testing hook this module's interface does not have. It is structurally safe
//! regardless — `spawn_persister`'s loop does not `select!` on `changes.changed()` while
//! awaiting the save's `spawn_blocking` task, so any change the `watch::Receiver` sees
//! during that await stays marked and is picked up by the very next `changes.changed()`
//! call once the write returns — and the same tight, sleep-free rename loop
//! `persister_writes_changes_within_a_second` already drives is, in practice, exactly the
//! kind of burst that lands changes while a write is in flight.

use super::*;
use crate::manager::{ManagerConfig, WindowManager};

/// A manager with no real windows; the event channel is drained so `handle_event`
/// calls (none, in these tests) would not need anywhere to go, matching the pattern
/// every other daemon test harness uses.
fn test_manager() -> Arc<WindowManager> {
    let (m, mut events) = WindowManager::new(ManagerConfig::new(
        "/tmp/anthrex-persister-test.sock".into(),
        "/bin/sh".into(),
    ));
    tokio::spawn(async move { while events.recv().await.is_some() {} });
    m
}

/// One dormant window, restored rather than spawned: `spawn_persister`'s subject is
/// the debounce loop, not a real PTY, and `restore` is the cheapest, fastest way to
/// get a window whose name `rename` can change to trigger `watch()` publications.
fn seed_one_window(m: &WindowManager, id: u32, name: &str) {
    m.restore(StateFile {
        version: STATE_VERSION,
        next_id: id + 1,
        windows: vec![WindowRecord {
            id,
            name: name.to_string(),
            runtime: Runtime::Shell,
            cwd: PathBuf::from("/tmp"),
            project: None,
            worktree: None,
            managed: None,
            model: None,
            initial_prompt: None,
            session_id: None,
            created_at: 1,
            status: Status::Exited,
            run: None,
            kind: Default::default(),
        }],
        runs: Vec::new(),
    });
}

/// A single change followed at once by cancellation must not hold shutdown up for
/// the debounce window: decision 9's wait is meant to collect a burst, not to give a
/// lone change priority over the daemon actually stopping. A naive debouncer that
/// merely `sleep`s for `SAVE_DEBOUNCE` before checking for cancellation would fail
/// this by taking (at least) that long to return.
#[tokio::test(start_paused = true)]
async fn a_single_change_then_immediate_cancellation_stops_promptly() {
    let m = test_manager();
    seed_one_window(&m, 1, "before");
    let dir = tempfile::tempdir().unwrap();
    let path = state_path(&dir);
    let shutdown = CancellationToken::new();
    let handle = spawn_persister(m.clone(), path, shutdown.clone());

    // Let the persister actually subscribe before the change, then fire one change
    // and cancel with no gap at all — not even a yield.
    tokio::task::yield_now().await;
    m.rename(1, "after".into()).unwrap();
    shutdown.cancel();

    tokio::time::timeout(Duration::from_millis(20), handle)
        .await
        .expect("persister did not stop promptly after a single change")
        .unwrap();
}

/// A burst that never goes quiet on its own must still release the persister the
/// instant it is cancelled — decision 9's collection window must never turn into an
/// unbounded wait for quiet — and, separately, `state.json` must not go arbitrarily
/// stale while that burst keeps running: [`SAVE_MAX_DELAY`] (fix wave 4, item 2)
/// bounds how long a sustained stream of changes can hold the write back. Before that
/// bound existed, this test built exactly this never-quiet input and asserted only
/// the cancellation half — the right scenario, checking the wrong property. A naive
/// debouncer whose collection window restarts on every change (only the outer,
/// pre-debounce `changed()` wait had no restart problem) passes the cancellation
/// assertion below while starving `state.json` for as long as the burst continues.
#[tokio::test(start_paused = true)]
async fn cancellation_during_a_never_quiet_burst_is_not_delayed() {
    let m = test_manager();
    seed_one_window(&m, 1, "before");
    let dir = tempfile::tempdir().unwrap();
    let path = state_path(&dir);
    let shutdown = CancellationToken::new();
    let handle = spawn_persister(m.clone(), path.clone(), shutdown.clone());

    let burst_stop = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let burst_manager = m.clone();
    let burst_stop_flag = burst_stop.clone();
    let burst = tokio::spawn(async move {
        let mut n: u64 = 0;
        while !burst_stop_flag.load(std::sync::atomic::Ordering::Relaxed) {
            n += 1;
            burst_manager.rename(1, format!("burst-{n}")).unwrap();
            tokio::time::sleep(Duration::from_millis(1)).await;
        }
    });

    // The burst never goes quiet for longer than SAVE_DEBOUNCE, so only
    // SAVE_MAX_DELAY can make a write land. Wait past that bound, with the burst
    // still running, and confirm state.json moved on from the pre-burst "before" —
    // to some "burst-*" name — despite the stream never settling.
    tokio::time::sleep(SAVE_MAX_DELAY + SAVE_DEBOUNCE).await;
    let read_path = path.clone();
    let contents = tokio::task::spawn_blocking(move || std::fs::read_to_string(&read_path))
        .await
        .unwrap()
        .unwrap_or_default();
    assert!(
        contents.contains("burst-"),
        "state.json did not become current within SAVE_MAX_DELAY while the burst \
         kept running (a debounce window that only ever restarts starves the write \
         indefinitely): {contents:?}"
    );

    // Run well past SAVE_DEBOUNCE while the burst keeps the debounce window
    // perpetually restarting, then cancel with the burst still going.
    tokio::time::sleep(SAVE_DEBOUNCE * 5).await;
    shutdown.cancel();
    tokio::time::timeout(Duration::from_millis(20), handle)
        .await
        .expect("persister did not stop promptly during a never-quiet burst")
        .unwrap();

    burst_stop.store(true, std::sync::atomic::Ordering::Relaxed);
    burst.await.unwrap();
}

/// Decision 9: "it skips the write when the serialized bytes equal the last ones it
/// wrote." A rename to a window's *own current name* still publishes on `watch()`
/// (`WindowManager::rename` does not special-case a no-op), so it is exactly the case
/// that would make a debouncer that writes unconditionally on every change perform a
/// write whose bytes are indistinguishable from the one already on disk. Checked by
/// mtime, the only externally observable trace of a write `save`'s own atomic-rename
/// implementation leaves: content equality alone cannot tell "skipped" from "wrote
/// the same bytes again".
#[tokio::test]
async fn a_no_op_change_is_not_written_but_the_next_real_one_is() {
    let m = test_manager();
    seed_one_window(&m, 1, "same-name");
    let dir = tempfile::tempdir().unwrap();
    let path = state_path(&dir);
    let shutdown = CancellationToken::new();
    let handle = spawn_persister(m.clone(), path.clone(), shutdown.clone());
    // `spawn_persister` only subscribes to `watch()` once its task actually runs;
    // without this, `rename` below can race ahead of that subscription and publish a
    // change the persister was never listening for yet.
    tokio::task::yield_now().await;

    // Wait for the first write (seeding this window is itself a call this test made
    // before the persister subscribed, so nothing has published yet — `rename` to the
    // window's own name is both the trigger and, deliberately, a no-op).
    m.rename(1, "same-name".into()).unwrap();
    wait_for_content(&path, |s| s.contains("same-name"), Duration::from_secs(2)).await;
    let after_first_write = std::fs::metadata(&path).unwrap().modified().unwrap();

    // A second no-op change: same name again. If this were written, the mtime would
    // advance even though the bytes are identical to what is already on disk.
    m.rename(1, "same-name".into()).unwrap();
    tokio::time::sleep(SAVE_DEBOUNCE * 3).await;
    let after_no_op = std::fs::metadata(&path).unwrap().modified().unwrap();
    assert_eq!(
        after_first_write, after_no_op,
        "a change whose bytes match the last write must not cause a second write"
    );

    // A real change must still reach disk.
    m.rename(1, "actually-different".into()).unwrap();
    wait_for_content(
        &path,
        |s| s.contains("actually-different"),
        Duration::from_secs(2),
    )
    .await;

    shutdown.cancel();
    handle.await.unwrap();
}

async fn wait_for_content(path: &Path, mut pred: impl FnMut(&str) -> bool, timeout: Duration) {
    let deadline = std::time::Instant::now() + timeout;
    loop {
        if let Ok(bytes) = std::fs::read(path)
            && let Ok(text) = String::from_utf8(bytes)
            && pred(&text)
        {
            return;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "timed out waiting for state.json to reflect the expected content"
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}
