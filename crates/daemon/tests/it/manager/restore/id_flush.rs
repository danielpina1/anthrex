//! A window's id reaches `state.json` before anything can learn it. After a crash the
//! next daemon rebuilds `next_id` from that file alone, so an id published but not yet
//! saved (the persister's debounce, `state::SAVE_DEBOUNCE` up to `SAVE_MAX_DELAY`) would
//! be handed to another window, and a reconnecting client that kept focus by id would
//! type into it. Real shells in real PTYs, a real persister and real files. A submodule
//! of `restore.rs` (AGENTS.md rule 8), sharing its fixtures via `use super::*`.

use super::*;
use tokio_util::sync::CancellationToken;

/// The production wiring: `spawn_persister` keeps the file, and `create` waits for it to
/// record the id it spent. The file is read the instant `create` returns, well inside
/// the debounce, and a fresh manager restored from it (no final flush: a crash) must not
/// hand the same id out again.
#[tokio::test]
async fn a_window_id_is_on_disk_before_its_create_returns() {
    let m = manager();
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("state.json");
    let shutdown = CancellationToken::new();
    let persister = state::spawn_persister(m.clone(), path.clone(), shutdown.clone());

    let info = create(&m, spec("flushed"), std::env::temp_dir(), 80, 24)
        .await
        .unwrap();
    let (on_disk, _problems) = state::load(&path);
    assert!(
        on_disk.next_id > info.id,
        "window {} was published before state.json recorded its id (next_id {})",
        info.id,
        on_disk.next_id
    );

    // The crash: the persister stops with no final flush.
    shutdown.cancel();
    persister.await.unwrap();
    m.remove(info.id).unwrap();

    let reborn = manager();
    reborn.restore(state::load(&path).0);
    let next = create_id(&reborn, spec("after-crash"), std::env::temp_dir(), 80, 24).await;
    reborn.remove(next).unwrap();
    assert_ne!(
        next, info.id,
        "the restarted daemon reissued an id a client may still be focused on"
    );
}

/// A persister that cannot finish a write must not hang `create`: the create fails
/// within `ManagerConfig::state_flush_timeout`, says why, opens nothing, and the manager
/// lock stays free the whole time. The seam is a real one: a FIFO at `state.json.tmp`,
/// so the persister's `save` blocks in `open` until a reader appears.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_create_fails_within_its_bound_when_the_persister_is_stuck() {
    const BOUND: Duration = Duration::from_millis(300);
    let mut config = ManagerConfig::for_tests("/tmp/unused.sock".into(), "/bin/sh".into());
    config.state_flush_timeout = BOUND;
    let (m, mut events) = WindowManager::new(config);
    tokio::spawn(async move { while events.recv().await.is_some() {} });
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("state.json");
    let fifo = dir.path().join("state.json.tmp");
    let c_fifo = std::ffi::CString::new(fifo.to_str().unwrap()).unwrap();
    // SAFETY: a valid NUL-terminated path; mkfifo only creates a file.
    assert_eq!(unsafe { libc::mkfifo(c_fifo.as_ptr(), 0o600) }, 0);
    // Lets the stuck write go on every exit, a failed assertion included: a blocking
    // task still in `open` would hold the runtime's shutdown, and the test, forever.
    let release = Release(fifo.clone());
    let shutdown = CancellationToken::new();
    let persister = state::spawn_persister(m.clone(), path.clone(), shutdown.clone());

    let started = Instant::now();
    let creating = {
        let m = m.clone();
        tokio::spawn(async move { create(&m, spec("stuck"), std::env::temp_dir(), 80, 24).await })
    };
    // While the create waits, the manager answers: no lock is held across the wait.
    let listed = {
        let m = m.clone();
        tokio::time::timeout(
            Duration::from_secs(5),
            tokio::task::spawn_blocking(move || m.list().len()),
        )
    };
    assert_eq!(listed.await.expect("the manager lock was held").unwrap(), 0);
    let outcome = tokio::time::timeout(BOUND + Duration::from_secs(5), creating)
        .await
        .expect("create hung on a stuck persister")
        .unwrap();
    let elapsed = started.elapsed();

    drop(release);
    let error = outcome.expect_err("a create whose id could not be saved must fail");
    assert!(
        error.to_string().contains("not opened") && error.to_string().contains("state file"),
        "{error}"
    );
    assert!(elapsed >= BOUND, "failed before its bound: {elapsed:?}");
    assert!(m.list().is_empty(), "a refused create opened a window");
    shutdown.cancel();
    tokio::time::timeout(Duration::from_secs(5), persister)
        .await
        .expect("the persister stopped once released")
        .unwrap();
}

/// Opens the FIFO for reading without blocking when dropped, which lets a writer stuck
/// in `open` return and finish its write into the pipe. The reader is kept open on a
/// thread of its own for a while, so the writer never meets a FIFO with no reader.
struct Release(PathBuf);

impl Drop for Release {
    fn drop(&mut self) {
        use std::os::unix::fs::OpenOptionsExt;
        let reader = std::fs::OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NONBLOCK)
            .open(&self.0);
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_secs(10));
            drop(reader);
        });
    }
}

/// The daemon starts its persister right after `restore`, before the runs are restored,
/// so windows opened during that restore get their ids saved too. Until the restore
/// finishes, the table is half-restored: here a record renamed and another removed, the
/// way the run restore's own steps change it. None of that may reach the file, because a
/// second crash would make the next daemon trust it; only `next_id` may move. Then a
/// second restore from what was written gets the original windows back and a fresh id,
/// and once the restore is finished the full table is written again.
#[tokio::test]
async fn a_crash_mid_restore_leaves_only_the_new_next_id_on_disk() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("state.json");
    let cwd = std::env::temp_dir();
    let original = StateFile {
        version: state::STATE_VERSION,
        next_id: 3,
        windows: vec![
            window_record(1, "kept", cwd.clone(), None),
            window_record(2, "stale-run-window", cwd.clone(), None),
        ],
        runs: Vec::new(),
    };
    state::save(&path, &original).unwrap();
    let (loaded, problems) = state::load(&path);
    assert!(problems.is_empty(), "{problems:?}");

    let m = manager();
    m.restore(loaded.clone());
    let shutdown = CancellationToken::new();
    let (persister, restoring) =
        state::spawn_persister_during_restore(m.clone(), path.clone(), shutdown.clone(), loaded);

    // The half-restored table, then a window opened during the restore. Its create
    // returns only after a write that saw every change before it.
    m.rename(1, "half-renamed".into()).unwrap();
    m.remove(2).unwrap();
    let opened = create_id(&m, spec("opened-in-restore"), cwd.clone(), 80, 24).await;
    assert_eq!(opened, 3);

    let (on_disk, _problems) = state::load(&path);
    let names: Vec<(u32, &str)> = (on_disk.windows.iter())
        .map(|w| (w.id, w.name.as_str()))
        .collect();
    assert_eq!(
        names,
        [(1, "kept"), (2, "stale-run-window")],
        "a half-restored table reached state.json"
    );
    assert!(on_disk.next_id > opened, "next_id {}", on_disk.next_id);

    // The second crash, and a second restore from what was written.
    let reborn = manager();
    reborn.restore(on_disk);
    let listed: Vec<(u32, String)> = reborn.list().into_iter().map(|w| (w.id, w.name)).collect();
    assert_eq!(
        listed,
        [(1, "kept".to_string()), (2, "stale-run-window".to_string())]
    );
    let fresh = create_id(&reborn, spec("after-second-crash"), cwd.clone(), 80, 24).await;
    reborn.remove(fresh).unwrap();
    assert_ne!(fresh, opened, "an id spent during the restore was reissued");

    // The restore is done: the next write is the whole table as it stood when that
    // create's id was saved, before its own window was listed.
    restoring.finish();
    let after = create_id(&m, spec("after-restore"), cwd, 80, 24).await;
    let (on_disk, _problems) = state::load(&path);
    let names: Vec<(u32, &str)> = (on_disk.windows.iter())
        .map(|w| (w.id, w.name.as_str()))
        .collect();
    assert_eq!(names, [(1, "half-renamed"), (3, "opened-in-restore")]);
    assert!(on_disk.next_id > after, "next_id {}", on_disk.next_id);
    m.remove(after).unwrap();
    m.remove(opened).unwrap();
    shutdown.cancel();
    persister.await.unwrap();
}
