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

#[tokio::test]
async fn rename_rejects_duplicates() {
    let m = manager();
    let a = create_id(&m, spec("a"), std::env::temp_dir(), 80, 24).await;
    let _b = create(&m, spec("b"), std::env::temp_dir(), 80, 24)
        .await
        .unwrap();
    assert!(m.rename(a, "b".into()).is_err());
    m.rename(a, "c".into()).unwrap();
    assert_eq!(find(&m, a).name, "c");
}

#[tokio::test]
async fn shutdown_rejects_a_pending_creation_before_it_can_spawn() {
    let m = manager();
    let worker = m.clone();
    let (ready_tx, ready_rx) = tokio::sync::oneshot::channel();
    let (resume_tx, resume_rx) = tokio::sync::oneshot::channel();
    let pending = tokio::spawn(async move {
        let spec = spec("late");
        let roots = daemon::project::resolve_roots(spec.cwd.clone()).await;
        ready_tx.send(()).unwrap();
        // Reproduce a create task paused before manager admission, without sleeps.
        resume_rx.await.unwrap();
        worker
            .create(spec, roots.project, roots.worktree, 80, 24)
            .await
    });
    tokio::time::timeout(Duration::from_secs(2), ready_rx)
        .await
        .expect("creation did not resolve its project")
        .unwrap();
    m.shutdown().await;
    resume_tx.send(()).unwrap();
    let result = tokio::time::timeout(Duration::from_secs(2), pending)
        .await
        .expect("pending creation did not finish")
        .unwrap();
    let windows = m.list();
    // The pre-fix path creates a real shell; clean it up before asserting RED.
    for window in &windows {
        m.remove(window.id).unwrap();
    }
    let error = result.expect_err("a pending creation spawned after shutdown");
    assert!(error.to_string().contains("shutting down"));
    assert!(windows.is_empty(), "shutdown admitted a new window");
}

#[tokio::test]
async fn shutdown_ends_every_window() {
    let m = manager();
    for n in ["s1", "s2"] {
        create(&m, spec(n), std::env::temp_dir(), 80, 24)
            .await
            .unwrap();
    }
    wait_until("both started", || {
        m.list().iter().all(|w| w.status != Status::Starting)
    })
    .await;
    let started = Instant::now();
    m.shutdown().await;
    assert!(started.elapsed() < Duration::from_millis(1500));
    wait_until("both exited", || {
        m.list().iter().all(|w| w.status == Status::Exited)
    })
    .await;
}

#[tokio::test]
async fn kill_ends_an_interactive_shell_within_a_second() {
    let m = manager();
    let id = create_id(&m, spec("quick-kill"), std::env::temp_dir(), 80, 24).await;
    wait_until("shell started", || find(&m, id).status != Status::Starting).await;
    let started = Instant::now();
    m.kill(id).unwrap();
    wait_until("exited", || find(&m, id).status == Status::Exited).await;
    assert!(started.elapsed() < Duration::from_secs(1));
}

#[tokio::test]
async fn the_first_exit_reason_is_preserved() {
    use daemon::window::WindowEvent;
    let m = manager();
    let id = create_id(&m, spec("first-exit"), std::env::temp_dir(), 80, 24).await;
    m.handle_event(
        id,
        WindowEvent::Exited {
            code: Some(7),
            signal: None,
        },
    );
    m.handle_event(
        id,
        WindowEvent::Exited {
            code: Some(9),
            signal: None,
        },
    );
    let exit = find(&m, id).exit.unwrap();
    m.write_input(id, b"exit\n").unwrap();
    assert_eq!(exit.code, Some(7));
    assert_eq!(exit.reason, "exited with code 7");
}

#[tokio::test]
async fn a_parser_panic_ends_the_child_and_keeps_its_reason() {
    use daemon::window::WindowEvent;
    let m = manager();
    let id = create_id(&m, spec("parser-panic"), std::env::temp_dir(), 80, 24).await;
    wait_until("shell started", || find(&m, id).status != Status::Starting).await;
    let pid = m.child_pid(id).unwrap().unwrap() as libc::pid_t;
    let mut changed = m.watch();
    m.handle_event(id, WindowEvent::ParserPanicked("boom".into()));
    tokio::time::timeout(Duration::from_secs(1), changed.changed())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(find(&m, id).status, Status::Exited);
    assert_eq!(
        find(&m, id).exit.unwrap().reason,
        "screen parser panicked: boom"
    );
    let deadline = Instant::now() + Duration::from_secs(3);
    let gone = loop {
        // SAFETY: this PID belongs to the window created by this test.
        if unsafe { libc::kill(pid, 0) } == -1
            && std::io::Error::last_os_error().raw_os_error() == Some(libc::ESRCH)
        {
            break true;
        }
        if Instant::now() >= deadline {
            break false;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    };
    if !gone {
        // SAFETY: ensure a failed regression leaves no test-owned child behind.
        unsafe {
            libc::kill(pid, libc::SIGKILL);
        }
    }
    assert!(gone, "parser panic left its child alive");
    // Deliver a later wait result explicitly: the assertion does not depend on a
    // fixed sleep to infer whether the waiter has already reported its exit.
    m.handle_event(
        id,
        WindowEvent::Exited {
            code: Some(0),
            signal: None,
        },
    );
    assert_eq!(
        find(&m, id).exit.unwrap().reason,
        "screen parser panicked: boom"
    );
}

#[tokio::test]
async fn shutdown_waits_for_cleanup_after_the_group_leader_exits() {
    // Isolate Linux's subreaper setting from the other concurrently running tests.
    // Our test process adopts and reaps only the descendant it explicitly created.
    #[cfg(target_os = "linux")]
    {
        const CHILD_ENV: &str = "ANTHREX_TEST_GROUP_REAPER";
        if std::env::var_os(CHILD_ENV).is_none() {
            let output = tokio::task::spawn_blocking(|| {
                std::process::Command::new(std::env::current_exe().unwrap())
                    .args([
                        "--exact",
                        "shutdown_waits_for_cleanup_after_the_group_leader_exits",
                        "--nocapture",
                    ])
                    .env(CHILD_ENV, "1")
                    .output()
                    .unwrap()
            })
            .await
            .unwrap();
            assert!(
                output.status.success(),
                "{}\n{}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            );
            return;
        }
        // SAFETY: this isolated test process becomes a reaper for its own descendants.
        assert_eq!(
            unsafe { libc::prctl(libc::PR_SET_CHILD_SUBREAPER, 1, 0, 0, 0) },
            0
        );
    }

    let mut premature_returns = Vec::new();
    for parser_panic in [false, true] {
        let dir = tempfile::tempdir().unwrap();
        let pid_file = dir.path().join("descendant.pid");
        let term_file = dir.path().join("term");
        let descendant = dir.path().join("descendant.sh");
        let leader = dir.path().join("leader.sh");
        std::fs::write(&descendant, format!(
            "trap '' HUP\ntrap 'printf term > \"{}\"; exit 0' TERM\nprintf '%s' \"$$\" > '{}'\nwhile :; do :; done\n",
            term_file.display(), pid_file.display())).unwrap();
        std::fs::write(
            &leader,
            format!(
                "trap 'exit 0' HUP\n/bin/sh '{}' &\nwait\n",
                descendant.display()
            ),
        )
        .unwrap();
        let (m, mut events) = WindowManager::new(ManagerConfig::new(
            "/tmp/unused.sock".into(),
            "/bin/sh".into(),
        ));
        let id = create_id(&m, spec("cleanup-descendant"), std::env::temp_dir(), 80, 24).await;
        m.write_input(
            id,
            format!("exec /bin/sh '{}'\n", leader.display()).as_bytes(),
        )
        .unwrap();
        let mut descendant_pid = None;
        wait_until("descendant traps ready", || {
            descendant_pid = std::fs::read_to_string(&pid_file)
                .ok()
                .and_then(|value| value.parse::<libc::pid_t>().ok());
            descendant_pid.is_some()
        })
        .await;
        let descendant_pid = descendant_pid.unwrap();
        let reaper = tokio::spawn(async move {
            wait_until("owned descendant reaped", || {
                #[cfg(target_os = "linux")]
                // SAFETY: wait only for the descendant PID written by our fixture.
                if unsafe { libc::waitpid(descendant_pid, std::ptr::null_mut(), libc::WNOHANG) }
                    == descendant_pid
                {
                    return true;
                }
                // SAFETY: this only probes the descendant started by this test.
                (unsafe { libc::kill(descendant_pid, 0) }) == -1
                    && std::io::Error::last_os_error().raw_os_error() == Some(libc::ESRCH)
            })
            .await;
        });
        if parser_panic {
            m.handle_event(
                id,
                daemon::window::WindowEvent::ParserPanicked("boom".into()),
            );
        } else {
            m.kill(id).unwrap();
        }
        tokio::time::timeout(Duration::from_secs(3), async {
            loop {
                let (event_id, event) = events.recv().await.unwrap();
                let exited = matches!(event, daemon::window::WindowEvent::Exited { .. });
                m.handle_event(event_id, event);
                if exited {
                    break;
                }
            }
        })
        .await
        .expect("leader did not exit on HUP");
        m.shutdown().await;
        let term_delivered_before_return = term_file.exists();
        // Clean up before asserting even on the pre-fix path: its detached worker
        // still sends TERM while this test runtime remains alive.
        reaper.await.unwrap();
        if !term_delivered_before_return {
            premature_returns.push(parser_panic);
        }
    }
    assert!(
        premature_returns.is_empty(),
        "shutdown returned before descendant cleanup for parser_panic={premature_returns:?}"
    );
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

/// M6.5's headline acceptance test (decision 14): a restored window is listed as exited,
/// carries its saved identity, is viewable with a placeholder screen, refuses input,
/// tolerates `kill`, still guards its name against a duplicate `create`, and never lets a
/// later `create` reuse its id.
#[tokio::test]
async fn restored_windows_are_exited_and_viewable() {
    let m = manager();
    let state = StateFile {
        version: state::STATE_VERSION,
        next_id: 2,
        windows: vec![window_record(1, "kept", std::env::temp_dir(), Some("s-1"))],
        runs: Vec::new(),
    };
    m.restore(state);

    let info = find(&m, 1);
    assert_eq!(info.name, "kept");
    assert_eq!(info.status, Status::Exited);
    assert_eq!(info.session_id.as_deref(), Some("s-1"));
    let exit = info.exit.expect("a restored window carries an exit reason");
    assert_eq!(exit.reason, DAEMON_RESTARTED);
    assert_eq!(exit.code, None);

    let att = m.attach(1).unwrap();
    let mut parser = vt100::Parser::new(att.rows, att.cols, 0);
    parser.process(&att.snapshot);
    let screen = parser.screen().contents();
    assert!(
        screen.contains("this window stopped when the daemon restarted"),
        "{screen}"
    );
    assert!(screen.contains("resumes session s-1"), "{screen}");

    let err = m.write_input(1, b"echo hi\n").unwrap_err().to_string();
    assert!(err.contains("not running"), "{err}");

    m.kill(1).unwrap();

    let err = create(&m, spec("kept"), std::env::temp_dir(), 80, 24)
        .await
        .unwrap_err();
    assert!(err.to_string().contains("already exists"), "{err}");

    let fresh = create(&m, spec("fresh"), std::env::temp_dir(), 80, 24)
        .await
        .unwrap();
    assert!(
        fresh.id > 1,
        "the next created window must not collide with the restored id: {fresh:?}"
    );
}

/// Minor 1, M6.5 review: a restored record whose id matches a live window must not
/// silently replace it. Before this fix, `inner.entries.insert(id, entry)` just dropped
/// the returned live `Window` with no kill and no `start_cleanup`, orphaning its PTY
/// child. `restore` runs once on an empty manager today, so this could not happen from
/// the one caller that exists — but that "it cannot happen from the only current caller"
/// reasoning is exactly what let the same id-collision class through as two Criticals one
/// task earlier, and `restore` is a public entry point in its own right, not trusted to
/// get `next_id` right by its own doc comment.
#[tokio::test]
async fn restore_refuses_a_record_whose_id_collides_with_a_live_window() {
    let m = manager();
    let live = create(&m, spec("live"), std::env::temp_dir(), 80, 24)
        .await
        .unwrap();

    m.restore(StateFile {
        version: state::STATE_VERSION,
        next_id: live.id + 1,
        windows: vec![window_record(live.id, "ghost", std::env::temp_dir(), None)],
        runs: Vec::new(),
    });

    let info = find(&m, live.id);
    assert_eq!(
        info.name, "live",
        "the live window must survive a colliding restore, not be replaced by the ghost record: {info:?}"
    );
    assert_ne!(
        info.status,
        Status::Exited,
        "the live window must not be turned dormant by a colliding restore: {info:?}"
    );
}

/// The other half of Minor 1: a *name* collision must be refused too, even when the
/// colliding record's id is new. Two restores in a row is the shape the review's own
/// probe used — the second restore's record shares a name with an entry the first
/// restore already inserted.
#[tokio::test]
async fn restore_refuses_a_record_whose_name_collides_with_an_existing_entry() {
    let m = manager();
    m.restore(StateFile {
        version: state::STATE_VERSION,
        next_id: 2,
        windows: vec![window_record(1, "first", std::env::temp_dir(), None)],
        runs: Vec::new(),
    });

    m.restore(StateFile {
        version: state::STATE_VERSION,
        next_id: 3,
        windows: vec![window_record(2, "first", std::env::temp_dir(), None)],
        runs: Vec::new(),
    });

    let list = m.list();
    assert_eq!(
        list.len(),
        1,
        "the second restore's colliding-name record must be refused, not silently taken: {list:?}"
    );
    assert_eq!(list[0].id, 1, "{list:?}");
    assert_eq!(list[0].name, "first", "{list:?}");
}

/// Decision 9: `state_snapshot` mirrors the live window table exactly, with no I/O
/// (verified indirectly — this only checks the shape it comes back with; the persister
/// tests below check it never blocks a change from reaching disk).
#[tokio::test]
async fn state_snapshot_reflects_the_window_table() {
    let m = manager();
    // `project` is deliberately not `spec.cwd` (both are `PathBuf`s `state_snapshot`
    // could transpose without either test-fixture value making the swap visible): a
    // review of this exact task found that blind spot has already shipped three
    // same-typed-field-swap bugs elsewhere in this milestone.
    let project_one = PathBuf::from("/some/distinct/project/one");
    let a = create_id(&m, spec("one"), project_one.clone(), 80, 24).await;
    let _b = create_id(&m, spec("two"), std::env::temp_dir(), 80, 24).await;
    m.rename(a, "renamed".into()).unwrap();

    let snapshot = m.state_snapshot();

    assert_eq!(snapshot.version, state::STATE_VERSION);
    assert_eq!(snapshot.next_id, 3);
    assert!(snapshot.runs.is_empty());
    assert_eq!(snapshot.windows.len(), 2);
    let mut names: Vec<_> = snapshot.windows.iter().map(|w| w.name.as_str()).collect();
    names.sort();
    assert_eq!(names, vec!["renamed", "two"]);
    let renamed = snapshot
        .windows
        .iter()
        .find(|w| w.name == "renamed")
        .unwrap();
    assert_eq!(renamed.cwd, std::env::temp_dir());
    assert_eq!(renamed.project.as_deref(), Some(project_one.as_path()));
    for record in &snapshot.windows {
        assert_eq!(record.runtime, Runtime::Shell);
        assert!(record.created_at > 0, "{record:?}");
    }
}

/// A fixture with a distinct, recognisable value for every same-typed field pair
/// `restore` or `state_snapshot`'s hand-written mapping could silently swap: `cwd` vs
/// `project`, `worktree.repo_root` vs `worktree.path`, `model` vs `initial_prompt`, and
/// `name` vs `session_id`. A record where any of these pairs coincide cannot catch a
/// transposition between them — see `restore_and_state_snapshot_do_not_transpose_a_same_typed_field_pair`.
fn distinct_record() -> WindowRecord {
    WindowRecord {
        id: 5,
        name: "record-name".into(),
        runtime: Runtime::Claude,
        cwd: PathBuf::from("/tmp/distinct/cwd-value"),
        project: Some(PathBuf::from("/tmp/distinct/project-value")),
        worktree: Some(state::WorktreeRecord {
            repo_root: PathBuf::from("/tmp/distinct/repo-root-value"),
            path: PathBuf::from("/tmp/distinct/worktree-path-value"),
            branch: "feature/distinct".into(),
        }),
        model: Some("model-value".into()),
        initial_prompt: Some("initial-prompt-value".into()),
        session_id: Some("session-id-value".into()),
        created_at: 1_650_000_000,
        status: Status::Idle,
        run: None,
    }
}

/// Guards the exact blind spot the M6.5 review flagged: before this task, `state.rs` was
/// pure serde derive on both sides of the (de)serialization, so there was nowhere for two
/// same-typed fields to be silently swapped. `restore` and `state_snapshot`'s hand-written
/// `Entry ↔ WindowRecord` mappings are the first real conflation sites in this module.
///
/// Checked through two different surfaces so a bug in one direction cannot cancel out a
/// matching bug in the other and pass anyway: `WindowInfo` (via `list()`) verifies most of
/// `restore`'s mapping independently of `state_snapshot`; `initial_prompt` has no
/// `WindowInfo` field to read back through, so `state_snapshot`'s own round trip is the
/// only way to check it — and it also re-checks every other field on the way back out.
#[tokio::test]
async fn restore_and_state_snapshot_do_not_transpose_a_same_typed_field_pair() {
    let m = manager();
    let record = distinct_record();
    m.restore(StateFile {
        version: state::STATE_VERSION,
        next_id: record.id + 1,
        windows: vec![record.clone()],
        runs: Vec::new(),
    });

    let info = find(&m, record.id);
    assert_eq!(info.name, record.name);
    assert_eq!(info.cwd, record.cwd);
    assert_eq!(info.project, record.project.clone().unwrap());
    let worktree = record.worktree.clone().unwrap();
    assert_eq!(info.worktree, Some(worktree.path.clone()));
    assert_eq!(info.branch, Some(worktree.branch.clone()));
    assert_eq!(info.session_id, record.session_id);
    assert_eq!(info.model, record.model);

    let snapshot = m.state_snapshot();
    let out = snapshot
        .windows
        .iter()
        .find(|w| w.id == record.id)
        .expect("restored record is in the snapshot");
    assert_eq!(out.name, record.name);
    assert_eq!(out.cwd, record.cwd);
    assert_eq!(out.project, record.project);
    assert_eq!(out.worktree, record.worktree);
    assert_eq!(out.model, record.model);
    assert_eq!(out.initial_prompt, record.initial_prompt);
    assert_eq!(out.session_id, record.session_id);
}

/// Sparse restored ids (1 and 9, not 1 and 2) are the case a naive "count of restored
/// windows" or "max saved next_id, trusted verbatim" implementation gets wrong: `restore`
/// must derive `next_id` from the *largest loaded id*, not from how many records there
/// are or from a caller-supplied `next_id` alone, or a `create` a few windows later would
/// silently reuse id 9.
#[tokio::test]
async fn restore_next_id_skips_a_sparse_gap() {
    let m = manager();
    let state = StateFile {
        version: state::STATE_VERSION,
        // Deliberately wrong/stale: a correct `next_id` would already be 10. `restore`
        // must not trust this value blindly.
        next_id: 2,
        windows: vec![
            window_record(1, "low", std::env::temp_dir(), None),
            window_record(9, "high", std::env::temp_dir(), None),
        ],
        runs: Vec::new(),
    };
    m.restore(state);

    for n in 0..9 {
        let info = create(
            &m,
            spec(&format!("filler-{n}")),
            std::env::temp_dir(),
            80,
            24,
        )
        .await
        .unwrap();
        assert_ne!(
            info.id, 9,
            "a created window must never reuse the restored id 9"
        );
    }
}

/// The other half of the saturation risk: a restored record already holding `u32::MAX`
/// leaves no larger id to hand out. `create` must refuse loudly rather than let
/// `next_id`'s `+= 1` wrap back to 0 and silently reuse an id already on the table.
#[tokio::test]
async fn create_refuses_when_the_restored_id_space_is_exhausted() {
    let m = manager();
    let state = StateFile {
        version: state::STATE_VERSION,
        next_id: 1,
        windows: vec![window_record(u32::MAX, "maxed", std::env::temp_dir(), None)],
        runs: Vec::new(),
    };
    m.restore(state);

    let err = create(&m, spec("one-too-many"), std::env::temp_dir(), 80, 24)
        .await
        .unwrap_err();
    assert!(err.to_string().contains("no window ids remain"), "{err}");
    // The refusal must not have created (and then abandoned) a window either.
    assert_eq!(m.list().len(), 1, "{:?}", m.list());
}

/// Decision 9's debounce: a burst of changes (twenty renames, no sleeps between them)
/// reaches disk as one write of the *final* state, and it does so within a second even
/// though the persister only fires 100 ms after the burst goes quiet.
#[tokio::test]
async fn persister_writes_changes_within_a_second() {
    let m = manager();
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("state.json");
    let shutdown = tokio_util::sync::CancellationToken::new();
    let handle = state::spawn_persister(m.clone(), path.clone(), shutdown.clone());

    let id = create_id(&m, spec("original"), std::env::temp_dir(), 80, 24).await;
    let deadline = Instant::now() + Duration::from_secs(1);
    loop {
        let (loaded, _problems) = state::load(&path);
        if loaded.windows.iter().any(|w| w.id == id) {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "state.json never listed the new window within 1s"
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }

    for n in 0..20 {
        m.rename(id, format!("name-{n}")).unwrap();
    }
    let deadline = Instant::now() + Duration::from_secs(1);
    loop {
        let (loaded, _problems) = state::load(&path);
        if loaded
            .windows
            .iter()
            .any(|w| w.id == id && w.name == "name-19")
        {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "state.json never settled on the final rename within 1s"
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }

    shutdown.cancel();
    handle.await.unwrap();
}
