use daemon::manager::WindowManager;
use proto::{Runtime, Status, WindowInfo, WindowSpec};
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
    let (m, mut events) = WindowManager::new("/tmp/unused.sock".into(), "/bin/sh".into());
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

#[tokio::test]
async fn create_lists_the_window_and_notifies_watchers() {
    let m = manager();
    let mut rx = m.watch();
    let info = m.create(spec("one"), 80, 24).unwrap();
    assert_eq!(info.id, 1);
    assert_eq!(info.name, "one");
    assert_eq!(info.status, Status::Starting);
    rx.changed().await.unwrap();
    assert_eq!(rx.borrow().len(), 1);
    let second = m.create(spec("two"), 80, 24).unwrap();
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
    let info = m.create(unnamed, 80, 24).unwrap();
    assert_eq!(info.name, "shell-1");
    let err = m.create(spec("shell-1"), 80, 24).unwrap_err();
    assert!(err.to_string().contains("already exists"));
    let mut bad_dir = spec("y");
    bad_dir.cwd = "/definitely/missing/dir".into();
    assert!(
        m.create(bad_dir, 80, 24)
            .unwrap_err()
            .to_string()
            .contains("does not exist")
    );
}

#[tokio::test]
async fn input_reaches_the_shell_and_output_drives_status() {
    let m = manager();
    let id = m.create(spec("io"), 80, 24).unwrap().id;
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

/// I6: worktrees are a later milestone. Accepting the branch would show it in the title
/// bar with no worktree behind it, so the user would believe the agent was isolated.
#[tokio::test]
async fn a_spec_asking_for_a_worktree_is_rejected() {
    let m = manager();
    let mut with_branch = spec("wt");
    with_branch.worktree_branch = Some("feat/x".into());
    let err = m.create(with_branch, 80, 24).unwrap_err().to_string();
    assert!(err.contains("--worktree is not implemented yet"), "{err}");
    assert!(m.list().is_empty(), "no window may be created");
    // Without the branch the same spec is fine.
    m.create(spec("wt"), 80, 24).unwrap();
}

#[tokio::test]
async fn attach_then_resize_reports_new_size() {
    let m = manager();
    let id = m.create(spec("size"), 80, 24).unwrap().id;
    m.resize(id, 120, 40).unwrap();
    let att = m.attach(id).unwrap();
    assert_eq!((att.cols, att.rows), (120, 40));
    assert!(m.attach(99).is_err());
}

#[tokio::test]
async fn kill_terminates_and_remove_forgets() {
    let m = manager();
    let id = m.create(spec("victim"), 80, 24).unwrap().id;
    wait_until("shell started", || find(&m, id).status != Status::Starting).await;
    m.kill(id).unwrap();
    wait_until("exited", || find(&m, id).status == Status::Exited).await;
    let info = find(&m, id);
    assert!(info.exit.is_some());
    m.remove(id).unwrap();
    assert!(m.list().is_empty());
    assert!(m.remove(id).is_err());
}

/// C1: a program that does not read its stdin must never block the manager.
///
/// `stty raw -echo; sleep 5` is the shape every full-screen agent (claude, codex, vim)
/// has: no canonical line discipline to drain the input queue and no echo, so the PTY
/// master write stalls after about 1 KB on macOS. Enqueueing input must stay
/// non-blocking (Ok, or a queue-full Err), and a concurrent `list()` must not wait
/// behind it.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_program_that_ignores_stdin_never_blocks_write_input_or_list() {
    let m = manager();
    let id = m.create(spec("blocked"), 80, 24).unwrap().id;
    wait_until("prompt output", || find(&m, id).status == Status::Working).await;
    m.write_input(id, b"stty raw -echo; sleep 5\n").unwrap();
    tokio::time::sleep(Duration::from_millis(600)).await;

    // A second thread hammers list() for as long as the writes run.
    let stop = Arc::new(AtomicBool::new(false));
    let lister = {
        let m = m.clone();
        let stop = stop.clone();
        std::thread::spawn(move || {
            let mut worst = Duration::ZERO;
            while !stop.load(Ordering::Relaxed) {
                let started = Instant::now();
                let _ = m.list();
                worst = worst.max(started.elapsed());
                std::thread::sleep(Duration::from_millis(5));
            }
            worst
        })
    };

    let chunk = vec![b'x'; 4096];
    for i in 0..16 {
        let started = Instant::now();
        // Ok or the queue-full Err; what matters is that it returns promptly.
        let _ = m.write_input(id, &chunk);
        let elapsed = started.elapsed();
        assert!(
            elapsed < Duration::from_millis(100),
            "write_input #{i} took {elapsed:?}"
        );
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
    m.remove(id).unwrap();
}

#[tokio::test]
async fn rename_rejects_duplicates() {
    let m = manager();
    let a = m.create(spec("a"), 80, 24).unwrap().id;
    let _b = m.create(spec("b"), 80, 24).unwrap();
    assert!(m.rename(a, "b".into()).is_err());
    m.rename(a, "c".into()).unwrap();
    assert_eq!(find(&m, a).name, "c");
}

#[tokio::test]
async fn shutdown_ends_every_window() {
    let m = manager();
    for n in ["s1", "s2"] {
        m.create(spec(n), 80, 24).unwrap();
    }
    wait_until("both started", || {
        m.list().iter().all(|w| w.status != Status::Starting)
    })
    .await;
    m.shutdown().await;
    wait_until("both exited", || {
        m.list().iter().all(|w| w.status == Status::Exited)
    })
    .await;
}
