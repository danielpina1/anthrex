//! The window lifecycle outside create and restart: rename, `kill`, `shutdown` (including
//! the group-leader/descendant cleanup edge case), the parser-panic path, and window name
//! validation.
//!
//! A submodule of `tests/manager.rs`, split out for AGENTS.md rule 8 (see that file's own
//! doc comment); keeps its `manager`, `spec`, `wait_until`, `find`, `create` and
//! `create_id` fixtures via `use super::*`, the same way `manager_worktree.rs`'s own
//! submodules share its fixtures.

use super::*;
use unicode_segmentation::UnicodeSegmentation;

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

/// Decision 22: a window name is trimmed and must be 1 to 64 characters with no control
/// characters, enforced identically on `create` and `rename` so a name refused at
/// creation can never be reached through a rename either. The 65-character, `\x1b` and
/// `"  "` (blank-after-trim) cases each carry decision 22's exact message; a 64-character
/// name is accepted; and a rename to the window's own current name succeeds, because a
/// window is not a duplicate of itself.
///
/// The length limit is counted in *grapheme clusters* (`unicode_segmentation`), the same
/// unit `crates/tui` already uses for user-facing text — not bytes and not `char`s. A
/// combining-character sequence (`"e\u{0301}"`, two `char`s that render and are perceived
/// as one glyph) makes that distinction observable: 64 copies is 64 `char`-pairs (128
/// `char`s) but exactly 64 grapheme clusters, and must be accepted; a `char`-counting
/// validator would wrongly refuse it.
#[tokio::test]
async fn names_are_validated_on_create_and_rename() {
    let m = manager();
    let base = create_id(&m, spec("base"), std::env::temp_dir(), 80, 24).await;

    let too_long = "a".repeat(65);
    // Two distinct 64-character names, so accepting one via `create` and the other via
    // `rename` cannot collide with each other under the existing duplicate-name check.
    let sixty_four_create = "a".repeat(64);
    let sixty_four_rename = "b".repeat(64);
    let control = "bad\x1bname";
    let blank = "  ";
    let cases: [(&str, &str); 3] = [
        (too_long.as_str(), "name must be at most 64 characters"),
        (control, "name must not contain control characters"),
        (blank, "name must not be empty"),
    ];

    for (name, expected) in cases {
        let err = create(&m, spec(name), std::env::temp_dir(), 80, 24)
            .await
            .unwrap_err();
        assert_eq!(err.to_string(), expected, "create({name:?})");

        let err = m.rename(base, name.to_string()).unwrap_err();
        assert_eq!(err.to_string(), expected, "rename({name:?})");
    }

    // A 64-character name is accepted by both create and rename.
    let created = create(&m, spec(&sixty_four_create), std::env::temp_dir(), 80, 24)
        .await
        .unwrap();
    assert_eq!(created.name, sixty_four_create);
    let sixty_four = sixty_four_rename;
    m.rename(base, sixty_four.clone()).unwrap();
    assert_eq!(find(&m, base).name, sixty_four);

    // Rename to the window's own current name succeeds: it is not a duplicate of itself.
    m.rename(base, sixty_four.clone()).unwrap();
    assert_eq!(find(&m, base).name, sixty_four);

    // Grapheme clusters, not `char`s: 64 combining-character sequences is 64 grapheme
    // clusters (accepted) but 128 `char`s (which a naive `.chars().count()` validator
    // would wrongly refuse).
    let combining_64 = "e\u{0301}".repeat(64);
    assert_eq!(combining_64.chars().count(), 128);
    m.rename(base, combining_64.clone()).unwrap();
    assert_eq!(find(&m, base).name, combining_64);

    let combining_65 = "e\u{0301}".repeat(65);
    let err = m.rename(base, combining_65).unwrap_err();
    assert_eq!(err.to_string(), "name must be at most 64 characters");
}

/// Fix wave 6, Minor: decision 22's "no control characters" was implemented with
/// `char::is_control()`, which correctly covers the whole Unicode `Cc` category (`\x1b`,
/// `\n`, `\r`, `\t`, `\x7f`) but not U+202E RIGHT-TO-LEFT OVERRIDE, which is category `Cf`
/// ("format"), not `Cc`. A name carrying it renders with its glyphs reordered without its
/// bytes changing — the "Trojan Source" spoofing class — which the live-daemon review
/// reproduced: `anthrex rename 1 "bad\u{202e}name"` succeeded and `anthrex ls` rendered
/// the reordered glyphs. The fix rejects the specific bidi formatting characters
/// (U+202A-U+202E, U+2066-U+2069), not the whole `Cf` category, because `Cf` also contains
/// U+200D ZERO WIDTH JOINER, required to fuse a legitimate multi-codepoint emoji sequence
/// into the single grapheme cluster it is rendered and counted as (decision 22's own
/// 64-*grapheme* limit, not 64-`char`, exists for exactly this reason) — so this test
/// checks both directions: the bidi override is refused, and a name built entirely from
/// family-emoji ZWJ sequences at exactly the 64-grapheme limit is still accepted, on both
/// `create` and `rename`.
#[tokio::test]
async fn rename_rejects_bidi_override_but_accepts_family_emoji() {
    let m = manager();
    let base = create_id(&m, spec("base2"), std::env::temp_dir(), 80, 24).await;

    let bidi = "bad\u{202E}name";
    let err = create(&m, spec(bidi), std::env::temp_dir(), 80, 24)
        .await
        .unwrap_err();
    assert_eq!(err.to_string(), "name must not contain control characters");
    let err = m.rename(base, bidi.to_string()).unwrap_err();
    assert_eq!(err.to_string(), "name must not contain control characters");

    // man + ZWJ + woman + ZWJ + girl + ZWJ + boy: one grapheme cluster built from three
    // ZWJs (Cf), repeated 64 times so the length check and the character check are both
    // exercised at once, exactly as the review reproduced it live against the daemon.
    // A second, distinct ZWJ sequence (man + ZWJ + woman, a couple emoji) covers `rename`,
    // so accepting one via `create` and the other via `rename` cannot collide with each
    // other under the existing duplicate-name check — the same reason
    // `names_are_validated_on_create_and_rename` above uses two distinct 64-character
    // fixtures rather than one shared between `create` and `rename`.
    let family = "\u{1F468}\u{200D}\u{1F469}\u{200D}\u{1F467}\u{200D}\u{1F466}";
    let couple = "\u{1F468}\u{200D}\u{1F469}";
    let family_64 = family.repeat(64);
    let couple_64 = couple.repeat(64);
    assert_eq!(
        family_64.graphemes(true).count(),
        64,
        "the fixture itself must be exactly 64 grapheme clusters"
    );
    assert_eq!(couple_64.graphemes(true).count(), 64);
    let created = create(&m, spec(&family_64), std::env::temp_dir(), 80, 24)
        .await
        .unwrap();
    assert_eq!(created.name, family_64);
    m.rename(base, couple_64.clone()).unwrap();
    assert_eq!(find(&m, base).name, couple_64);
}
