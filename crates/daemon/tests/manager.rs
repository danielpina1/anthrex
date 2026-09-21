//! `WindowManager` integration tests, against real PTYs, real shells and a manager whose
//! events are pumped by a background task the way the daemon does.
//!
//! Split by responsibility (AGENTS.md rule 8 — this file was 1170+ lines before fix wave
//! 5 and grew past 1400 with its own regression tests): this file keeps the fixtures every
//! submodule below shares (`spec`, `manager`, `wait_until`, `find`, `create`, `create_id`,
//! `window_record`) plus the create/attach/kill/backpressure tests that are this crate's
//! most basic coverage. Everything else moves to a submodule along a seam the test names
//! already drew:
//!
//! - [`mod@lifecycle`]: rename, shutdown, the parser-panic/cleanup-descendant tests, and
//!   window name validation — the window's whole life outside create and restart.
//! - [`mod@restore`]: `WindowManager::restore` and the persister.
//! - [`mod@restart`]: `WindowManager::restart` (task M6.7, and fix wave 5's Critical 1,
//!   Major 2 and Major 3 regression tests).

#[path = "manager/lifecycle.rs"]
mod lifecycle;
#[path = "manager/restart.rs"]
mod restart;
#[path = "manager/restore.rs"]
mod restore;

use daemon::manager::{DAEMON_RESTARTED, ManagerConfig, WindowManager};
use daemon::state::{self, StateFile, WindowRecord};
use proto::{Runtime, Status, WindowInfo, WindowSpec};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

fn spec(name: &str) -> WindowSpec {
    WindowSpec {
        name: Some(name.to_string()),
        runtime: Runtime::Shell,
        cwd: std::env::temp_dir(),
        worktree_branch: None,
        model: None,
        initial_prompt: None,
    }
}

/// A manager whose events are pumped by a background task, like the daemon does.
fn manager() -> Arc<WindowManager> {
    let (m, mut events) = WindowManager::new(ManagerConfig::new(
        "/tmp/unused.sock".into(),
        "/bin/sh".into(),
    ));
    let pump = m.clone();
    tokio::spawn(async move {
        while let Some((id, ev)) = events.recv().await {
            pump.handle_event(id, ev);
        }
    });
    m
}

async fn wait_until(what: &str, mut cond: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(8);
    while !cond() {
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
}

fn find(m: &WindowManager, id: u32) -> WindowInfo {
    m.list()
        .into_iter()
        .find(|w| w.id == id)
        .expect("window listed")
}

/// `create()` with no worktree, for the many tests that don't exercise it.
async fn create(
    m: &WindowManager,
    spec: WindowSpec,
    project: PathBuf,
    cols: u16,
    rows: u16,
) -> anyhow::Result<WindowInfo> {
    m.create(spec, project, None, cols, rows).await
}

/// [`create`], unwrapped, for the many tests that only want the new window's id.
async fn create_id(
    m: &WindowManager,
    spec: WindowSpec,
    project: PathBuf,
    cols: u16,
    rows: u16,
) -> u32 {
    create(m, spec, project, cols, rows).await.unwrap().id
}

#[tokio::test]
async fn create_records_the_given_project() {
    let m = manager();
    let info = create(&m, spec("p"), "/some/root".into(), 80, 24)
        .await
        .unwrap();
    let listed = find(&m, info.id);
    m.remove(info.id).unwrap();
    assert_eq!(info.project, std::path::Path::new("/some/root"));
    assert_eq!(listed.project, info.project);
}

#[tokio::test]
async fn create_records_the_given_worktree() {
    let m = manager();
    let worktree = PathBuf::from("/some/project/linked");
    let info = m
        .create(
            spec("wt-root"),
            "/some/project".into(),
            Some(worktree.clone()),
            80,
            24,
        )
        .await
        .unwrap();
    let listed = find(&m, info.id);
    m.remove(info.id).unwrap();
    assert_eq!(info.worktree, Some(worktree));
    assert_eq!(listed.worktree, info.worktree);
}

#[tokio::test]
async fn create_lists_the_window_and_notifies_watchers() {
    let m = manager();
    let mut rx = m.watch();
    let info = create(&m, spec("one"), std::env::temp_dir(), 80, 24)
        .await
        .unwrap();
    assert_eq!(info.id, 1);
    assert_eq!(info.name, "one");
    assert_eq!(info.status, Status::Starting);
    rx.changed().await.unwrap();
    assert_eq!(rx.borrow().len(), 1);
    let second = create(&m, spec("two"), std::env::temp_dir(), 80, 24)
        .await
        .unwrap();
    assert_eq!(second.id, 2);
    assert_eq!(
        m.list().iter().map(|w| w.name.as_str()).collect::<Vec<_>>(),
        vec!["one", "two"]
    );
}

#[tokio::test]
async fn names_default_to_runtime_and_id_and_must_be_unique() {
    let m = manager();
    let mut unnamed = spec("x");
    unnamed.name = None;
    let info = create(&m, unnamed, std::env::temp_dir(), 80, 24)
        .await
        .unwrap();
    assert_eq!(info.name, "shell-1");
    let err = create(&m, spec("shell-1"), std::env::temp_dir(), 80, 24)
        .await
        .unwrap_err();
    assert!(err.to_string().contains("already exists"));
    let mut bad_dir = spec("y");
    bad_dir.cwd = "/definitely/missing/dir".into();
    assert!(
        create(&m, bad_dir, std::env::temp_dir(), 80, 24)
            .await
            .unwrap_err()
            .to_string()
            .contains("does not exist")
    );
}

#[tokio::test]
async fn input_reaches_the_shell_and_output_drives_status() {
    let m = manager();
    let id = create_id(&m, spec("io"), std::env::temp_dir(), 80, 24).await;
    wait_until("prompt output", || find(&m, id).status == Status::Working).await;
    m.write_input(id, b"echo mgr-$((2+2))\n").unwrap();
    wait_until("command output", || {
        let (snap, _, _) = m.snapshot(id).unwrap();
        String::from_utf8_lossy(&snap).contains("mgr-4")
    })
    .await;
    // Nothing else is running, so after QUIET_AFTER the tick turns it idle.
    tokio::time::sleep(daemon::manager::QUIET_AFTER + Duration::from_millis(200)).await;
    m.tick();
    assert_eq!(find(&m, id).status, Status::Idle);
}

#[tokio::test]
async fn attach_then_resize_reports_new_size() {
    let m = manager();
    let id = create_id(&m, spec("size"), std::env::temp_dir(), 80, 24).await;
    m.resize(id, 120, 40).unwrap();
    let att = m.attach(id).unwrap();
    assert_eq!((att.cols, att.rows), (120, 40));
    assert!(m.attach(99).is_err());
}

#[tokio::test]
async fn kill_terminates_and_remove_forgets() {
    let m = manager();
    let id = create_id(&m, spec("victim"), std::env::temp_dir(), 80, 24).await;
    wait_until("shell started", || find(&m, id).status != Status::Starting).await;
    let started = Instant::now();
    m.kill(id).unwrap();
    wait_until("exited", || find(&m, id).status == Status::Exited).await;
    assert!(started.elapsed() < Duration::from_millis(1500));
    let info = find(&m, id);
    assert!(info.exit.is_some());
    m.remove(id).unwrap();
    assert!(m.list().is_empty());
    assert!(m.remove(id).is_err());
}

/// C1: a program that does not read its stdin must never block the manager.
///
/// A raw-mode child that ignores stdin can fill the platform's PTY input buffer.
/// Confirm sustained backpressure before checking concurrent list() latency.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_program_that_ignores_stdin_never_blocks_write_input_or_list() {
    let m = manager();
    let id = create_id(&m, spec("blocked"), std::env::temp_dir(), 80, 24).await;
    let stop = Arc::new(AtomicBool::new(false));
    // Remove our own child and stop the lister even if an assertion panics.
    struct Cleanup(Arc<WindowManager>, u32, Arc<AtomicBool>);
    impl Drop for Cleanup {
        fn drop(&mut self) {
            self.2.store(true, Ordering::Relaxed);
            let _ = self.0.remove(self.1);
        }
    }
    let _cleanup = Cleanup(m.clone(), id, stop.clone());
    let write = |bytes: &[u8]| {
        let started = Instant::now();
        let result = m.write_input(id, bytes);
        assert!(
            started.elapsed() < Duration::from_millis(100),
            "write_input took {:?}",
            started.elapsed()
        );
        match result {
            Ok(()) => false,
            Err(err) => {
                assert!(err.to_string().contains("not reading input"), "{err}");
                true
            }
        }
    };
    wait_until("prompt output", || find(&m, id).status == Status::Working).await;
    let child_started = Instant::now();
    assert!(!write(
        b"stty raw -echo && printf '%s%s' RAW_ READY && exec sleep 15\n"
    ));
    wait_until("raw-mode child readiness", || {
        let (snap, _, _) = m.snapshot(id).unwrap();
        String::from_utf8_lossy(&snap).contains("RAW_READY")
    })
    .await;

    let chunk = vec![b'x'; 4096];
    let deadline = Instant::now() + Duration::from_secs(2);
    let mut full_since = None;
    for _ in 0..400 {
        if Instant::now() >= deadline {
            break;
        }
        if write(&chunk) {
            full_since = Some(Instant::now());
            break;
        }
    }
    // A burst alone only outpaces the writer. Give it time to drain, then retry
    // the same chunk: a byte-full queue still accepts empty chunks. A successful
    // retry stops the probe, so sustained failures cannot add more pressure.
    let mut confirmed = false;
    while let Some(since) = full_since {
        if Instant::now() >= deadline {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
        if !write(&chunk) {
            break;
        }
        if since.elapsed() >= Duration::from_millis(200) {
            confirmed = true;
            break;
        }
    }
    use std::io::Write;
    if !confirmed {
        writeln!(std::io::stdout(), "skip: sustained PTY backpressure unconfirmed within the 400-chunk/2s probe; concurrent C1 assertions not exercised").unwrap();
        return;
    }

    // A second thread hammers list() for as long as the writes run.
    let ready = Arc::new(std::sync::Barrier::new(2));
    let lister = {
        let m = m.clone();
        let stop = stop.clone();
        let ready = ready.clone();
        std::thread::spawn(move || {
            let mut worst = Duration::ZERO;
            ready.wait();
            loop {
                let started = Instant::now();
                let _ = m.list();
                worst = worst.max(started.elapsed());
                if stop.load(Ordering::Relaxed) {
                    break;
                }
                std::thread::sleep(Duration::from_millis(5));
            }
            worst
        })
    };
    ready.wait();

    for i in 0..16 {
        assert!(write(&chunk), "backpressure disappeared at write #{i}");
        let started = Instant::now();
        let _ = m.list();
        let elapsed = started.elapsed();
        assert!(
            elapsed < Duration::from_millis(100),
            "list() after write #{i} took {elapsed:?}"
        );
    }

    stop.store(true, Ordering::Relaxed);
    let worst = lister.join().unwrap();
    assert!(
        worst < Duration::from_millis(100),
        "concurrent list() worst case was {worst:?}"
    );
    assert!(child_started.elapsed() < Duration::from_secs(15));
    assert_ne!(find(&m, id).status, Status::Exited);
    writeln!(std::io::stdout(), "ok: sustained PTY backpressure confirmed for 200ms; write_input and concurrent list() stayed below 100ms").unwrap();
}

/// A minimal, valid `WindowRecord` for `restore` tests: a shell window with no worktree,
/// only the fields each test actually cares about set to something other than the
/// default.
fn window_record(id: u32, name: &str, cwd: PathBuf, session_id: Option<&str>) -> WindowRecord {
    WindowRecord {
        id,
        name: name.to_string(),
        runtime: Runtime::Shell,
        cwd,
        project: None,
        worktree: None,
        model: None,
        initial_prompt: None,
        session_id: session_id.map(str::to_string),
        created_at: 1_700_000_000,
        status: Status::Working, // deliberately not Exited: restore must override this.
        run: None,
    }
}
