//! End-to-end tests for `daemon::run` itself — not `server::serve` in isolation, the way
//! `tests/server.rs`'s harness starts things, but the whole startup and shutdown sequence:
//! the lifetime lock, the socket, the pid file. Each test drives a real `daemon::run` on
//! its own temporary socket and data directory, and talks to it with raw frames, exactly
//! like `tests/server.rs` does — this file just cannot reuse `tests/support`, because that
//! harness starts `server::serve` directly and never exercises `run`'s own setup and
//! teardown, which is what these tests are about.

use daemon::lockfile::DaemonLock;
use daemon::state::{StateFile, WindowRecord};
use daemon::{DaemonOptions, run};
use proto::{ClientKind, ClientMsg, DaemonMsg, PROTO_VERSION, Runtime, Status, WindowSpec};
use proto::{read_frame, write_frame};
use std::path::{Path, PathBuf};
use std::time::Duration;
use tokio::net::UnixStream;
use tokio::net::unix::{OwnedReadHalf, OwnedWriteHalf};

/// A `DaemonOptions` for a fresh, isolated socket and data directory. `lock_wait` is
/// `Duration::ZERO` throughout this file: these tests want a locked-out daemon to fail
/// fast, not to sit in `acquire`'s retry loop.
fn opts(socket: PathBuf, data_dir: PathBuf) -> DaemonOptions {
    DaemonOptions {
        socket_path: socket,
        // A config path that does not exist: `run` does not read it in this milestone's
        // task, but the field must be filled in regardless.
        config_path: data_dir.join("config.toml"),
        data_dir,
        lock_wait: Duration::ZERO,
    }
}

fn shell_spec(name: &str) -> WindowSpec {
    WindowSpec {
        name: Some(name.into()),
        runtime: Runtime::Shell,
        cwd: std::env::temp_dir(),
        worktree_branch: None,
        model: None,
        initial_prompt: None,
    }
}

async fn wait_for_path(path: &Path, timeout: Duration) {
    tokio::time::timeout(timeout, async {
        loop {
            if path.exists() {
                return;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .unwrap_or_else(|_| panic!("timed out waiting for {} to appear", path.display()));
}

struct Client {
    rd: OwnedReadHalf,
    wr: OwnedWriteHalf,
}

impl Client {
    async fn connect(socket: &Path) -> (Self, DaemonMsg) {
        let stream = UnixStream::connect(socket)
            .await
            .unwrap_or_else(|e| panic!("connecting to {}: {e}", socket.display()));
        let (rd, wr) = stream.into_split();
        let mut c = Client { rd, wr };
        c.send(ClientMsg::Hello {
            proto_version: PROTO_VERSION,
            client: ClientKind::Cli,
        })
        .await;
        let first = c.recv().await;
        (c, first)
    }

    async fn send(&mut self, m: ClientMsg) {
        write_frame(&mut self.wr, &m).await.unwrap();
    }

    async fn recv(&mut self) -> DaemonMsg {
        tokio::time::timeout(Duration::from_secs(5), read_frame(&mut self.rd))
            .await
            .expect("timed out waiting for a frame")
            .unwrap()
            .expect("daemon closed the connection")
    }

    async fn recv_until(&mut self, mut pred: impl FnMut(&DaemonMsg) -> bool) -> DaemonMsg {
        tokio::time::timeout(Duration::from_secs(8), async {
            loop {
                let m = self.recv().await;
                if pred(&m) {
                    return m;
                }
            }
        })
        .await
        .expect("timed out waiting for the expected message")
    }
}

/// The one-line-mutation check this test aims at: an implementation that skipped the
/// lock, or that unlinked the socket unconditionally instead of by inode, would still
/// pass a version of this test that only checked `Ok(())`. So it also checks every piece
/// of cleanup decision 24 and decision 26 promise: the socket and pid file are gone, and
/// the lock is free enough for a fresh `DaemonLock::acquire` to succeed at once.
#[tokio::test]
async fn run_starts_serves_and_stops_cleanly() {
    let dir = tempfile::tempdir().unwrap();
    let socket = dir.path().join("d.sock");
    let data_dir = dir.path().join("data");
    let handle = tokio::spawn(run(opts(socket.clone(), data_dir.clone())));
    wait_for_path(&socket, Duration::from_secs(5)).await;

    let (mut c, welcome) = Client::connect(&socket).await;
    match welcome {
        DaemonMsg::Welcome { windows, .. } => assert!(windows.is_empty(), "{windows:?}"),
        other => panic!("expected Welcome, got {other:?}"),
    }

    c.send(ClientMsg::CreateWindow {
        spec: shell_spec("w"),
        cols: 80,
        rows: 24,
    })
    .await;
    c.recv_until(|m| matches!(m, DaemonMsg::Created { .. }))
        .await;

    c.send(ClientMsg::Shutdown).await;
    c.recv_until(|m| matches!(m, DaemonMsg::Bye { .. })).await;

    let result = tokio::time::timeout(Duration::from_secs(10), handle)
        .await
        .expect("daemon::run did not return within 10s")
        .expect("the daemon task panicked");
    assert!(result.is_ok(), "run returned an error: {result:?}");

    assert!(!socket.exists(), "the socket file was not removed");
    assert!(
        !data_dir.join("daemon.pid").exists(),
        "daemon.pid was not removed"
    );
    let _lock = DaemonLock::acquire(&data_dir, Duration::ZERO)
        .expect("the lock must be free once run has returned");
}

/// The adversarial case decision 24 exists for: a second daemon must never be able to
/// share the first's data directory, must never touch a socket of its own before the
/// lock check fails, and must never disturb the daemon that is still holding it.
#[tokio::test]
async fn second_daemon_with_the_same_data_dir_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    let socket1 = dir.path().join("d1.sock");
    let socket2 = dir.path().join("d2.sock");
    let data_dir = dir.path().join("data");
    let handle1 = tokio::spawn(run(opts(socket1.clone(), data_dir.clone())));
    wait_for_path(&socket1, Duration::from_secs(5)).await;

    let err = run(opts(socket2.clone(), data_dir.clone()))
        .await
        .expect_err("a second daemon on the same data dir must be refused");
    assert!(err.to_string().contains("another anthrex daemon"), "{err}");
    assert!(
        !socket2.exists(),
        "the refused daemon must never create its own socket"
    );

    // The first daemon must be completely unaffected by the second's failed attempt.
    let (mut c, welcome) = Client::connect(&socket1).await;
    assert!(matches!(welcome, DaemonMsg::Welcome { .. }), "{welcome:?}");
    c.send(ClientMsg::Shutdown).await;
    c.recv_until(|m| matches!(m, DaemonMsg::Bye { .. })).await;
    tokio::time::timeout(Duration::from_secs(10), handle1)
        .await
        .expect("daemon::run did not return within 10s")
        .expect("the daemon task panicked")
        .expect("run returned an error");
}

/// Fix wave 10, item 1 — the review's reproduction, run directly against `daemon::run`
/// (the same entry point two racing `anthrex new` calls' detached
/// `daemon start --foreground` children actually run), not a throwaway script:
///
/// Before fix wave 10, a daemon that lost the race for `DaemonLock` just sat in its
/// retry loop for up to `lock_wait`, and would win the lock — and go on to become a
/// live, unrequested second daemon — the moment the winner released it, including by
/// the winner simply being told to stop. So `B` here is given a `lock_wait` many times
/// longer than this test could plausibly take: with that bug, `B` would sit blocked for
/// that whole duration (or until `A` released, whichever came first) before this
/// assertion could even run. `B` must instead notice `A`'s socket answering on its very
/// first contended lock attempt and return well before any of that — so this is
/// deterministic, not a race won by timing luck: `A` is not stopped until well after
/// `B` has already returned.
///
/// Whole-branch-review Major 4: fix wave 10 made `B` return `Ok(())` here, which fixed
/// the phantom-daemon hazard above but silently ate decision 24's own refusal at the
/// same time — `anthrex daemon start --foreground` against a live daemon exited 0
/// instead of failing with `another anthrex daemon is running`, failing the brief's own
/// manual check 10. `B` not becoming a second daemon (no socket of its own, no bound
/// listener, A completely unaffected) and `B` *reporting* that refusal to its caller
/// are two different guarantees — fix wave 10 only needed the first. This test now pins
/// both: `B` must still return promptly (the phantom-avoidance mechanism, unchanged),
/// but it must do so with the exact `Err` decision 24 specifies, not `Ok(())`.
#[tokio::test]
async fn a_racing_second_start_is_refused_and_does_not_become_a_phantom_daemon() {
    let dir = tempfile::tempdir().unwrap();
    let socket = dir.path().join("d.sock");
    let data_dir = dir.path().join("data");

    let handle_a = tokio::spawn(run(opts(socket.clone(), data_dir.clone())));
    wait_for_path(&socket, Duration::from_secs(5)).await;

    // B races in against the same socket and data dir while A is fully up and serving
    // — exactly the reported scenario, and exactly decision 24's manual check 10. Its
    // own `lock_wait` (2s) is deliberately much larger than the near-instant return the
    // fix should produce.
    let mut b_opts = opts(socket.clone(), data_dir.clone());
    b_opts.lock_wait = Duration::from_secs(2);
    let started = std::time::Instant::now();
    let b_result = run(b_opts).await;
    let elapsed = started.elapsed();

    let err = b_result.expect_err(
        "B raced against a live A on the same socket and data dir; decision 24 says \
         this must be refused, not silently succeed",
    );
    assert!(
        err.to_string()
            .contains("another anthrex daemon is running"),
        "{err}"
    );
    assert!(
        err.to_string().contains(&data_dir.display().to_string()),
        "the error must name the data directory (decision 24): {err}"
    );
    assert!(
        elapsed < Duration::from_millis(500),
        "B took {elapsed:?} to return — the old bug blocked for the full lock_wait \
         (2s) before either winning the lock or bailing; the fix must notice A on the \
         very first contended attempt instead"
    );

    // A must be completely unaffected: still the only daemon, still answering, right up
    // until this test stops it itself.
    let (mut c, welcome) = Client::connect(&socket).await;
    assert!(matches!(welcome, DaemonMsg::Welcome { .. }), "{welcome:?}");
    c.send(ClientMsg::Shutdown).await;
    c.recv_until(|m| matches!(m, DaemonMsg::Bye { .. })).await;
    tokio::time::timeout(Duration::from_secs(10), handle_a)
        .await
        .expect("daemon::run did not return within 10s")
        .expect("the daemon task panicked")
        .expect("run returned an error");

    // Now that A is fully gone, nothing must be listening — not A (it stopped on
    // request), and not some lingering B that had actually bound a second socket
    // before this function ever got control back.
    assert!(
        UnixStream::connect(&socket).await.is_err(),
        "no daemon should be reachable once A stopped and B had already yielded"
    );
}

/// Decision 26's other adversarial case: a daemon mid-shutdown must unlink its socket
/// path only if the file there is still the one it bound — never a replacement that
/// another daemon (here, a plain listener standing in for one) bound at the same path in
/// the meantime.
#[tokio::test]
async fn a_stopping_daemon_leaves_a_replacement_socket_alone() {
    let dir = tempfile::tempdir().unwrap();
    let socket = dir.path().join("d.sock");
    let data_dir = dir.path().join("data");
    let handle = tokio::spawn(run(opts(socket.clone(), data_dir.clone())));
    wait_for_path(&socket, Duration::from_secs(5)).await;

    // Open the connection that will carry `Shutdown` *before* the swap below, exactly as
    // decision 26 describes: an already-open connection outlives the path it was opened
    // through, so the daemon can still be asked to stop after its socket file has been
    // replaced out from under it.
    let (mut c, _welcome) = Client::connect(&socket).await;

    std::fs::remove_file(&socket).unwrap();
    let replacement = std::os::unix::net::UnixListener::bind(&socket).unwrap();
    replacement.set_nonblocking(true).unwrap();

    c.send(ClientMsg::Shutdown).await;
    c.recv_until(|m| matches!(m, DaemonMsg::Bye { .. })).await;

    let result = tokio::time::timeout(Duration::from_secs(10), handle)
        .await
        .expect("daemon::run did not return within 10s")
        .expect("the daemon task panicked");
    assert!(result.is_ok(), "run returned an error: {result:?}");

    assert!(
        socket.exists(),
        "the stopping daemon must leave the replacement socket file alone"
    );
    let probe = std::os::unix::net::UnixStream::connect(&socket)
        .expect("the replacement listener's path must still be connectable");
    let deadline = std::time::Instant::now() + Duration::from_secs(2);
    let accepted = loop {
        match replacement.accept() {
            Ok(_) => break true,
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                if std::time::Instant::now() >= deadline {
                    break false;
                }
                std::thread::sleep(Duration::from_millis(10));
            }
            Err(_) => break false,
        }
    };
    assert!(
        accepted,
        "the replacement listener must still be accepting connections"
    );
    drop(probe);
}

/// Decision 11's advertised guarantee: "a socket file that has disappeared therefore
/// means the state is on disk." Fix wave 4, item 5 (M6.5 review, Minor 4): the shutdown
/// path used to unlink the socket unconditionally even when the final flush failed, which
/// falsifies that guarantee at exactly the moment it would matter.
///
/// `data_dir/state.json` is pre-created as a directory, so `state::save`'s
/// rename-`.tmp`-over-`path` step fails with `EISDIR` on every attempt, including the
/// final one — the socket must still be on disk once `run` returns.
#[tokio::test]
async fn a_failed_final_flush_leaves_the_socket_in_place() {
    let dir = tempfile::tempdir().unwrap();
    let socket = dir.path().join("d.sock");
    let data_dir = dir.path().join("data");
    std::fs::create_dir_all(&data_dir).unwrap();
    std::fs::create_dir(data_dir.join("state.json")).unwrap();

    let handle = tokio::spawn(run(opts(socket.clone(), data_dir.clone())));
    wait_for_path(&socket, Duration::from_secs(5)).await;

    let (mut c, _welcome) = Client::connect(&socket).await;
    c.send(ClientMsg::Shutdown).await;
    c.recv_until(|m| matches!(m, DaemonMsg::Bye { .. })).await;

    let result = tokio::time::timeout(Duration::from_secs(10), handle)
        .await
        .expect("daemon::run did not return within 10s")
        .expect("the daemon task panicked");
    assert!(result.is_ok(), "run returned an error: {result:?}");

    assert!(
        socket.exists(),
        "a failed final flush must leave the socket in place, or decision 11's guarantee \
         (\"a socket file that has disappeared means the state is on disk\") is false \
         exactly when it matters"
    );
}

/// M6.5's headline lifecycle test (decisions 9, 11, 12 and 14 together): a window created
/// and renamed in one daemon lifetime is listed, exited, in the next one, over a *real*
/// stop and start — not a direct call to `restore` — so decision 11's shutdown order (the
/// persister stopped and awaited, then one final flush, then the socket unlinked) is
/// exactly what is under test, not assumed.
#[tokio::test]
async fn windows_survive_a_daemon_restart_as_exited() {
    let dir = tempfile::tempdir().unwrap();
    let socket = dir.path().join("d.sock");
    let data_dir = dir.path().join("data");

    let handle = tokio::spawn(run(opts(socket.clone(), data_dir.clone())));
    wait_for_path(&socket, Duration::from_secs(5)).await;

    let (mut c, _welcome) = Client::connect(&socket).await;
    c.send(ClientMsg::CreateWindow {
        spec: shell_spec("keep"),
        cols: 80,
        rows: 24,
    })
    .await;
    let window_id = match c
        .recv_until(|m| matches!(m, DaemonMsg::Created { .. }))
        .await
    {
        DaemonMsg::Created { window_id } => window_id,
        other => panic!("expected Created, got {other:?}"),
    };

    c.send(ClientMsg::Rename {
        window_id,
        name: "kept".into(),
    })
    .await;
    c.recv_until(|m| matches!(m, DaemonMsg::Ack { request } if request == "rename"))
        .await;

    c.send(ClientMsg::Shutdown).await;
    c.recv_until(|m| matches!(m, DaemonMsg::Bye { .. })).await;
    tokio::time::timeout(Duration::from_secs(10), handle)
        .await
        .expect("first daemon::run did not return within 10s")
        .expect("the daemon task panicked")
        .expect("first run returned an error");

    // The socket is gone only after decision 11's whole sequence, so by this point the
    // rename must already be on disk — checked directly, ahead of the second `run`, so a
    // failure here points at the shutdown flush rather than at restore.
    let state_path = data_dir.join("state.json");
    assert!(
        state_path.exists(),
        "state.json must exist after a clean shutdown"
    );
    let (loaded, problems) = daemon::state::load(&state_path);
    assert!(problems.is_empty(), "{problems:?}");
    let record = loaded
        .windows
        .iter()
        .find(|w| w.id == window_id)
        .expect("state.json holds the created window");
    assert_eq!(record.name, "kept");

    // Second daemon, same data dir: the window comes back exited, with decision 14's
    // reason and placeholder screen.
    let handle2 = tokio::spawn(run(opts(socket.clone(), data_dir.clone())));
    wait_for_path(&socket, Duration::from_secs(5)).await;

    let (mut c2, welcome) = Client::connect(&socket).await;
    let windows = match welcome {
        DaemonMsg::Welcome { windows, .. } => windows,
        other => panic!("expected Welcome, got {other:?}"),
    };
    let restored = windows
        .iter()
        .find(|w| w.id == window_id)
        .expect("restored window is listed in Welcome");
    assert_eq!(restored.name, "kept");
    assert_eq!(restored.status, Status::Exited);
    assert_eq!(
        restored.exit.as_ref().map(|e| e.reason.as_str()),
        Some("daemon restarted")
    );

    c2.send(ClientMsg::Subscribe {
        window_id,
        cols: 80,
        rows: 24,
    })
    .await;
    let bytes = match c2
        .recv_until(|m| matches!(m, DaemonMsg::Snapshot { .. }))
        .await
    {
        DaemonMsg::Snapshot { bytes, .. } => bytes,
        other => panic!("expected Snapshot, got {other:?}"),
    };
    let mut parser = vt100::Parser::new(24, 80, 0);
    parser.process(&bytes);
    let screen = parser.screen().contents();
    assert!(
        screen.contains("this window stopped when the daemon restarted"),
        "{screen}"
    );

    c2.send(ClientMsg::Shutdown).await;
    c2.recv_until(|m| matches!(m, DaemonMsg::Bye { .. })).await;
    tokio::time::timeout(Duration::from_secs(10), handle2)
        .await
        .expect("second daemon::run did not return within 10s")
        .expect("the daemon task panicked")
        .expect("second run returned an error");
}

/// A minimal, valid `WindowRecord` for this file's restart test — the fields the test
/// actually cares about set, everything else at a harmless default.
fn restart_test_record(
    id: u32,
    name: &str,
    runtime: Runtime,
    cwd: PathBuf,
    model: &str,
    initial_prompt: &str,
    session_id: &str,
) -> WindowRecord {
    WindowRecord {
        id,
        name: name.to_string(),
        runtime,
        cwd,
        project: None,
        worktree: None,
        model: Some(model.to_string()),
        initial_prompt: Some(initial_prompt.to_string()),
        session_id: Some(session_id.to_string()),
        created_at: 1_700_000_000,
        status: Status::Exited,
        run: None,
    }
}

/// M6.7's end-to-end acceptance test for design decisions 15-17: a restart resumes a
/// restored window's saved session, for both runtimes, over a real `daemon::run` (not a
/// direct `WindowManager::restart` call the way `tests/manager.rs` drives it) — so
/// `runtimes.claude.command`/`runtimes.codex.command` from `config.toml`, milestone 6's
/// own resolution path, is what actually launches the resume, not a bin path handed in
/// directly by the test.
///
/// `argv.sh` stands in for both `claude` and `codex`: it prints every argument it was
/// given, one per line entry, then `exec sleep 60` so the window stays alive to be
/// subscribed to. That is enough to check the resume argv (design decisions 15-16)
/// without needing either real agent installed.
///
/// The `READY` line printed right after the argv is deliberate (fix wave 5 re-review,
/// Minor 3): it pins that the argv assertion below checks the argv *line itself*, not
/// "nothing else is on screen after it" — a fixture that never printed anything past the
/// argv would pass either way and hide the distinction.
#[tokio::test]
async fn restart_resumes_claude_and_codex_sessions() {
    let dir = tempfile::tempdir().unwrap();
    let socket = dir.path().join("d.sock");
    let data_dir = dir.path().join("data");
    std::fs::create_dir_all(&data_dir).unwrap();

    let script = dir.path().join("argv.sh");
    std::fs::write(
        &script,
        "#!/bin/sh\nprintf 'ARGV:'\nfor a in \"$@\"; do printf ' [%s]' \"$a\"; done\nprintf '\\n'\nprintf 'READY\\n'\nexec sleep 60\n",
    )
    .unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
    }

    let opts = opts(socket.clone(), data_dir.clone());
    std::fs::write(
        &opts.config_path,
        format!(
            "[runtimes.claude]\ncommand = {:?}\n\n[runtimes.codex]\ncommand = {:?}\n",
            script.display().to_string(),
            script.display().to_string(),
        ),
    )
    .unwrap();

    let cwd = std::env::temp_dir();
    const CLAUDE_ID: u32 = 1;
    const CODEX_ID: u32 = 2;
    let state = StateFile {
        version: daemon::state::STATE_VERSION,
        next_id: 3,
        windows: vec![
            restart_test_record(
                CLAUDE_ID,
                "claude-w",
                Runtime::Claude,
                cwd.clone(),
                "opus",
                "do it",
                "sess-abc",
            ),
            restart_test_record(
                CODEX_ID,
                "codex-w",
                Runtime::Codex,
                cwd.clone(),
                "m1",
                "go",
                "thr-1",
            ),
        ],
        runs: Vec::new(),
    };
    daemon::state::save(&data_dir.join("state.json"), &state).unwrap();

    let handle = tokio::spawn(run(opts));
    wait_for_path(&socket, Duration::from_secs(5)).await;

    let (mut c, welcome) = Client::connect(&socket).await;
    match welcome {
        DaemonMsg::Welcome { windows, .. } => assert_eq!(windows.len(), 2, "{windows:?}"),
        other => panic!("expected Welcome, got {other:?}"),
    }

    for (window_id, must_contain, must_not_contain) in [
        (CLAUDE_ID, "[--resume] [sess-abc]", "[do it]"),
        (CODEX_ID, "[resume] [thr-1]", "[go]"),
    ] {
        c.send(ClientMsg::Subscribe {
            window_id,
            cols: 80,
            rows: 24,
        })
        .await;
        // The placeholder screen decision 14 gives a dormant window; its content is not
        // what this test is about.
        c.recv_until(|m| matches!(m, DaemonMsg::Snapshot { window_id: w, .. } if *w == window_id))
            .await;

        c.send(ClientMsg::Restart { window_id }).await;

        // Collect every Snapshot/Output for this window into a screen mirror until the
        // restart's own Ack arrives *and* the argv line has actually shown up on
        // screen — the two can interleave in either order (the Ack comes from the
        // detached restart task, the Snapshot/Output from the forwarder's own
        // reattach), so both conditions are tracked independently rather than assumed
        // to arrive in a fixed sequence.
        let mut mirror = vt100::Parser::new(24, 80, 0);
        let mut acked = false;
        let deadline = std::time::Instant::now() + Duration::from_secs(8);
        loop {
            let msg = c.recv().await;
            match msg {
                DaemonMsg::Snapshot {
                    window_id: w,
                    cols,
                    rows,
                    bytes,
                } if w == window_id => {
                    mirror = vt100::Parser::new(rows, cols, 0);
                    mirror.process(&bytes);
                }
                DaemonMsg::Output {
                    window_id: w,
                    bytes,
                } if w == window_id => {
                    mirror.process(&bytes);
                }
                DaemonMsg::Ack { request } if request == "restart" => acked = true,
                DaemonMsg::Error { request, message } if request == "restart" => {
                    panic!("restart of window {window_id} failed: {message}")
                }
                _ => {}
            }
            let screen = mirror.screen().contents();
            if acked && screen.contains(must_contain) {
                assert!(
                    !screen.contains(must_not_contain),
                    "window {window_id}: {screen:?}"
                );
                if window_id == CODEX_ID {
                    // Decision 16, restored per Minor 5 (fix wave 5 review): the brief
                    // requires the argv line to *end* with `[resume] [thr-1]` and to show
                    // `[-m] [m1]` *before* it — both dropped from the original assertion,
                    // which only checked substring containment anywhere on screen. The
                    // property is separately pinned at the unit level
                    // (`launch/mod.rs`'s `codex_resume_puts_resume_last_and_drops_the_prompt`),
                    // but the brief asked for it here too, against a real spawned process.
                    //
                    // Minor 3 (fix wave 5 re-review): the fixture's whole argv is one
                    // `printf`-built logical line, but at 80 columns it soft-wraps across
                    // several screen rows, and vt100's own `contents()` joins rows with
                    // `\n` regardless of whether the break was a real newline or a wrap —
                    // flattening the *whole screen* into one line and asserting against its
                    // end asserted more than the property means: it required the argv line
                    // to be the last thing on the 24-row screen, not merely that the argv
                    // line itself ends with resume last. `Screen::row_wrapped` tells the two
                    // kinds of row boundary apart, so the argv line's own rows can be
                    // reassembled and checked on their own, whatever the shell prints after
                    // it.
                    //
                    // Anchored on `must_contain` (`[resume] [thr-1]`, already known present
                    // — that is what let this loop iteration reach here), not on `ARGV:`:
                    // the argv text is long enough to scroll `ARGV:` itself off the top of
                    // the 24-row screen well before the whole line has arrived, while the
                    // tail end — what this assertion actually cares about — is still on
                    // screen by construction.
                    let vt = mirror.screen();
                    let (rows, cols) = vt.size();
                    let row_texts: Vec<String> = vt.rows(0, cols).collect();
                    let hit_row = row_texts
                        .iter()
                        .position(|r| r.contains("[resume] [thr-1]"))
                        .unwrap_or_else(|| panic!("[resume] [thr-1] not found: {row_texts:?}"));
                    let mut start_row = hit_row as u16;
                    while start_row > 0 && vt.row_wrapped(start_row - 1) {
                        start_row -= 1;
                    }
                    let mut end_row = hit_row as u16;
                    while end_row + 1 < rows && vt.row_wrapped(end_row) {
                        end_row += 1;
                    }
                    let argv_line: String =
                        row_texts[usize::from(start_row)..=usize::from(end_row)].concat();
                    assert!(
                        argv_line.trim_end().ends_with("[resume] [thr-1]"),
                        "the argv line must end with resume last: {argv_line:?}"
                    );
                    let m_pos = argv_line
                        .find("[-m] [m1]")
                        .unwrap_or_else(|| panic!("-m m1 not on the argv line: {argv_line:?}"));
                    let resume_pos = argv_line.find("[resume] [thr-1]").expect("checked above");
                    assert!(
                        m_pos < resume_pos,
                        "-m must appear before resume: {argv_line:?}"
                    );
                }
                break;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "window {window_id} did not resume within 8s: acked={acked}, screen={:?}",
                mirror.screen().contents()
            );
        }
    }

    c.send(ClientMsg::Shutdown).await;
    c.recv_until(|m| matches!(m, DaemonMsg::Bye { .. })).await;
    tokio::time::timeout(Duration::from_secs(10), handle)
        .await
        .expect("daemon::run did not return within 10s")
        .expect("the daemon task panicked")
        .expect("run returned an error");
}
