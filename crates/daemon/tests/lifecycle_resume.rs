//! M6.7's end-to-end resume test over a real `daemon::run`, alone in its own test binary
//! because it sets `ANTHREX_CLAUDE_BIN` and `ANTHREX_CODEX_BIN` in this process's
//! environment (M8b.17 review): in edition 2024 that races any `environ` reader or process
//! spawn on another libtest thread (`worktree_env.rs`'s module doc is the precedent). Keep
//! this file to this one test. It shares `lifecycle.rs`'s fixtures through
//! `lifecycle/common.rs`.

use daemon::run;
use daemon::state::{StateFile, WindowRecord};
use proto::{ClientMsg, DaemonMsg, Runtime, Status};
use std::path::PathBuf;
use std::time::Duration;

#[path = "lifecycle/common.rs"]
mod common;
use common::*;

/// The runtime command overrides this test sets to its own fake.
const BIN_VARS: [&str; 2] = ["ANTHREX_CLAUDE_BIN", "ANTHREX_CODEX_BIN"];

/// A minimal, valid `WindowRecord` for this file's test — the fields the test
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
        kind: Default::default(),
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
/// `argv.sh` also answers `--version` and exits, the way `scripts/pty-smoke.py`'s own
/// resume fixture already does. `lifecycle::run` probes `runtimes.codex.command
/// --version` at startup and holds every launch, this test's restarts included, until
/// that probe finishes. Without the answer, the probe ran `exec sleep 60` and spent its
/// whole `CODEX_PROBE_TIMEOUT`, which raced `recv()`'s own 5 s bound.
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
        "#!/bin/sh\nif [ \"$1\" = --version ]; then printf 'codex-cli 0.155.0\\n'; exit 0; fi\nprintf 'ARGV:'\nfor a in \"$@\"; do printf ' [%s]' \"$a\"; done\nprintf '\\n'\nprintf 'READY\\n'\nexec sleep 60\n",
    )
    .unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    // Run the fixture once, through its side-effect-free `--version` branch, before the
    // daemon starts. On macOS the first exec of a freshly written executable waits for a
    // security assessment (`docs/timing-budgets.md`, "First exec of a freshly written
    // executable"); paid by the startup probe instead, it would sit inside the probe's
    // budget that every restart below waits on, against a bound of half that budget.
    // Before the environment changes below, and waited for, so no spawn overlaps them.
    {
        let mut warm = std::process::Command::new(&script)
            .arg("--version")
            .stdout(std::process::Stdio::null())
            .spawn()
            .unwrap();
        // A harness deadline, not a budget: it only turns a wedged exec into a failure.
        let deadline = std::time::Instant::now() + Duration::from_secs(60);
        let status = loop {
            if let Some(status) = warm.try_wait().unwrap() {
                break status;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "warming the fixture did not finish"
            );
            std::thread::sleep(Duration::from_millis(10));
        };
        assert!(status.success(), "warming the fixture failed: {status}");
    }

    // M8b.17 review: `ANTHREX_CLAUDE_BIN`/`ANTHREX_CODEX_BIN` win over `config.toml`
    // (config decision 6), so an inherited value (the safety policy points both at a
    // path that does not exist) would launch nothing. Both are set to this test's own
    // `argv.sh`, which `config.toml` below names as well: whichever wins, the program is
    // this fake, never a real agent. The daemon reads them when it starts, below.
    let restore = [BIN_VARS[0], BIN_VARS[1]].map(|key| (key, std::env::var_os(key)));
    // SAFETY: this is the only test in this binary, and no other thread of it exists
    // yet (the daemon starts below), so nothing reads `environ` while it changes.
    unsafe {
        for key in BIN_VARS {
            std::env::set_var(key, &script);
        }
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
        let restart_sent = std::time::Instant::now();

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
                DaemonMsg::Ack { request } if request == "restart" => {
                    // `restart` waits on the launch gate, and the gate stays closed until
                    // the startup `--version` probe of `runtimes.codex.command` finishes
                    // or spends its whole `CODEX_PROBE_TIMEOUT` budget. A fixture that
                    // does not answer `--version` holds this restart for that whole
                    // budget, and every `recv()` above is bounded by the same five
                    // seconds: the two raced, and macOS CI run 35822907546 lost. Half
                    // the budget is far above a restart's real cost (a spawn of tens of
                    // milliseconds, `docs/timing-budgets.md`) and far below a probe
                    // that ran out its clock.
                    let waited = restart_sent.elapsed();
                    assert!(
                        waited < daemon::lifecycle::CODEX_PROBE_TIMEOUT / 2,
                        "window {window_id}'s restart took {waited:?}: it waited behind \
                         the startup Codex probe, so the fixture did not answer --version"
                    );
                    acked = true;
                }
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

    // SAFETY: as above; the daemon has returned, and this binary has no other test.
    unsafe {
        for (key, value) in restore {
            match value {
                Some(value) => std::env::set_var(key, value),
                None => std::env::remove_var(key),
            }
        }
    }
}
