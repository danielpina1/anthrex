//! State survival across a real daemon restart, as opposed to the parent module's own
//! startup/lock/socket lifecycle. A window created and renamed in one daemon lifetime
//! must still be there, exited, after a real stop and start (M6.5's headline test), and a
//! restored window's `Runtime::Claude`/`Runtime::Codex` sessions must resume rather than
//! relaunch fresh. A submodule of `lifecycle.rs` (AGENTS.md rule 8), sharing its `opts`,
//! `shell_spec`, `wait_for_path` and `Client` fixtures via `use super::*`.

use super::*;

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
        managed: None,
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
                    //
                    // The comment above had vt100's two readers exactly backwards, and CI
                    // run 35707817474 collected on it. `Screen::contents()` rejoins a soft
                    // wrap *seamlessly*; it is `Screen::rows()` that splits there. So a
                    // marker straddling a column boundary is present in the `contents()`
                    // string this loop gates on and in no single row — the gate passed and
                    // searching row by row panicked. Whether it straddles is a function of
                    // the argv's length modulo 80, and the argv embeds absolute temp paths,
                    // so it fired on macOS CI and never here. Rejoin the wrapped rows into
                    // logical lines *first* and search those: now a marker is missing only
                    // when it is genuinely absent, at any width and any argv length.
                    let vt = mirror.screen();
                    let (rows, cols) = vt.size();
                    let row_texts: Vec<String> = vt.rows(0, cols).collect();
                    let mut logical: Vec<String> = Vec::new();
                    let mut current = String::new();
                    for row in 0..rows {
                        current.push_str(&row_texts[usize::from(row)]);
                        if !vt.row_wrapped(row) {
                            logical.push(std::mem::take(&mut current));
                        }
                    }
                    if !current.is_empty() {
                        logical.push(current);
                    }
                    let argv_line = logical
                        .iter()
                        .find(|line| line.contains("[resume] [thr-1]"))
                        .unwrap_or_else(|| panic!("[resume] [thr-1] not found: {logical:?}"));
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
