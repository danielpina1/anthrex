//! `WindowManager::restart`: task M6.7's headline tests, and fix wave 5's regression
//! tests for the review's Critical 1 (a stale cleanup record breaking `kill`, a second
//! restart, the `child_alive` stale-event race and `shutdown`, all after one restart),
//! Major 2 (decision 18's kill-wait timeout must refuse, not restart anyway) and Major 3
//! (a restart must be refused after `shutdown`, mirroring `create`'s own check).
//!
//! A submodule of `tests/manager.rs` (AGENTS.md rule 8; the review itself named "the
//! restart block" as the seam to split on). Shares that file's fixtures via `use
//! super::*`, including `window_record`, which several tests here use to build a dormant
//! window through `restore` before restarting it.
//!
//! `restart` vs `remove_with_worktree` mutual exclusion (Major 4) is tested in
//! `manager_worktree/removal/ordering.rs` instead, alongside the removal-ordering tests it
//! is the symmetric half of — it needs a real worktree window, which this binary's
//! `manager()` fixture deliberately never makes (see `manager_worktree.rs`'s own doc
//! comment on why worktree coverage lives in its own test binary).

use super::*;

/// Whether any process in `pid`'s group is still there. Signal 0 performs only the
/// existence/permission check, so this is a pure question, never an action — the same
/// check `manager_worktree`'s own `group_alive` uses, for the same reason: `ESRCH` is the
/// only answer that means the whole group is gone, and a reaped child leaves no zombie
/// behind because the window's own waiter (`window.rs`'s `pty-wait-{id}` thread) collects
/// it before the `Exited` event is ever sent.
fn group_alive(pid: u32) -> bool {
    // SAFETY: `pid` came from `child_pid`, and signal 0 delivers nothing.
    unsafe { libc::killpg(pid as libc::pid_t, 0) == 0 }
}

/// The last `pid-<digits>` occurrence in `text`, ignoring an unexpanded `pid-$$` (the
/// literal echo of the typed command, which appears before the shell's own output and
/// whose digit scan comes back empty).
fn last_pid(text: &str) -> Option<String> {
    let mut result = None;
    let mut idx = 0;
    while let Some(pos) = text[idx..].find("pid-") {
        let start = idx + pos + "pid-".len();
        let digits: String = text[start..]
            .chars()
            .take_while(char::is_ascii_digit)
            .collect();
        if !digits.is_empty() {
            result = Some(digits);
        }
        idx = start;
    }
    result
}

/// M6.7's headline test for decision 17: a restored (dormant) window has no process to
/// kill, so `restart` relaunches at once. Time bound: `wait_until`'s 8 s deadline is a
/// failsafe far past this path's real cost (a PTY spawn and a shell prompt, tens of
/// milliseconds per `docs/timing-budgets.md`'s idle measurements) — not the expected
/// cost, which this test does not assert on at all.
#[tokio::test]
async fn restart_of_a_restored_shell_runs_it_again() {
    let m = manager();
    let cwd = std::env::temp_dir();
    m.restore(StateFile {
        version: state::STATE_VERSION,
        next_id: 2,
        windows: vec![window_record(1, "restored-shell", cwd.clone(), None)],
        runs: Vec::new(),
    });
    assert_eq!(find(&m, 1).status, Status::Exited, "sanity: starts dormant");

    m.restart(1).await.unwrap();

    wait_until("restarted window leaves Exited", || {
        find(&m, 1).status != Status::Exited
    })
    .await;
    m.write_input(1, b"echo re-$((2+3))\n").unwrap();
    wait_until("restarted shell echoes its command", || {
        let (snap, _, _) = m.snapshot(1).unwrap();
        String::from_utf8_lossy(&snap).contains("re-5")
    })
    .await;
}

/// M6.7's other headline test, decision 18: restarting a *live* window kills the running
/// shell first, then relaunches a new one — proven by a different pid than the first
/// shell reported for the identical command.
///
/// Time bound: `wait_until`'s 8 s deadline exceeds this path's own worst case with real
/// margin — production's kill grace is `KILL_GRACE` (3 s) and `restart`'s own wait adds 2
/// more, for a 5 s internal deadline, on top of which a PTY spawn and shell round trip
/// cost tens of milliseconds (`docs/timing-budgets.md`). It is deliberately *not* written
/// as `assert!(elapsed < Duration::from_secs(5))`: that literal would coincide exactly
/// with `restart`'s own `kill_grace + 2s` deadline, the arithmetic-equality shape
/// `docs/timing-budgets.md`'s standing rule 1 flags as a defect regardless of how the
/// number was arrived at. In the ordinary case here the shell dies on SIGHUP at once
/// (AGENTS.md), so the real cost is nowhere near either bound.
#[tokio::test]
async fn restart_of_a_live_window_replaces_the_process() {
    let m = manager();
    let id = create_id(&m, spec("live-restart"), std::env::temp_dir(), 80, 24).await;
    wait_until("shell started", || find(&m, id).status != Status::Starting).await;

    m.write_input(id, b"echo pid-$$\n").unwrap();
    wait_until("first pid printed", || {
        let (snap, _, _) = m.snapshot(id).unwrap();
        last_pid(&String::from_utf8_lossy(&snap)).is_some()
    })
    .await;
    let (first_snap, _, _) = m.snapshot(id).unwrap();
    let first_pid =
        last_pid(&String::from_utf8_lossy(&first_snap)).expect("first pid captured above");

    m.restart(id).await.unwrap();

    wait_until("restarted window leaves Exited", || {
        find(&m, id).status != Status::Exited
    })
    .await;
    m.write_input(id, b"echo pid-$$\n").unwrap();
    wait_until("second pid differs from the first", || {
        let (snap, _, _) = m.snapshot(id).unwrap();
        last_pid(&String::from_utf8_lossy(&snap)).is_some_and(|pid| pid != first_pid)
    })
    .await;
}

/// Design decision 18: a second `Restart` for a window already being restarted is
/// refused, not queued. `tokio::join!` polls both futures on this test's single-threaded
/// runtime; `WindowManager::restart`'s phase A runs synchronously up to its first await
/// point (see `manager/restart.rs`), so whichever future is polled first sets
/// `restarting` and the other observes it already set before either does anything that
/// could block — deterministic, not a race this test happens to win.
#[tokio::test]
async fn concurrent_restart_is_refused() {
    let m = manager();
    let id = create_id(&m, spec("concurrent-restart"), std::env::temp_dir(), 80, 24).await;
    wait_until("shell started", || find(&m, id).status != Status::Starting).await;

    let (first, second) = tokio::join!(m.restart(id), m.restart(id));
    let results = [first, second];
    let ok_count = results.iter().filter(|r| r.is_ok()).count();
    let refused_count = results
        .iter()
        .filter(|r| {
            r.as_ref()
                .err()
                .is_some_and(|e| e.to_string().contains("already restarting"))
        })
        .count();
    assert_eq!(ok_count, 1, "{results:?}");
    assert_eq!(refused_count, 1, "{results:?}");
}

/// Design decision 19: a restored window whose saved `cwd` no longer exists cannot be
/// relaunched into it.
#[tokio::test]
async fn restart_needs_the_directory() {
    let m = manager();
    let missing = std::env::temp_dir().join("anthrex-test-restart-missing-dir");
    m.restore(StateFile {
        version: state::STATE_VERSION,
        next_id: 2,
        windows: vec![window_record(1, "missing-dir", missing, None)],
        runs: Vec::new(),
    });

    let err = m.restart(1).await.unwrap_err();
    assert!(
        err.to_string().contains("directory does not exist"),
        "{err}"
    );
}

/// The test task M6.7's brief leaves out: `concurrent_restart_is_refused` above only
/// proves the `restarting` flag *blocks* a second restart while one is in flight — it
/// says nothing about whether the flag is ever released after a *failed* restart, and
/// would pass identically whether or not that release existed. A leaked flag would brick
/// this window forever: every later restart refused with "already restarting", with
/// nothing short of a daemon restart able to clear it (see `manager/restart.rs`'s
/// `Restarting` guard). Reusing `restart_needs_the_directory`'s scenario — a restore
/// whose cwd is missing is a ready-made, deterministic way to make `restart` fail — and
/// restarting the same window a second time proves the guard's `Drop` actually ran: a
/// leaked flag would change the second error from "directory does not exist" to "already
/// restarting".
#[tokio::test]
async fn a_failed_restart_leaves_the_window_restartable() {
    let m = manager();
    let missing = std::env::temp_dir().join("anthrex-test-restart-missing-dir-2");
    m.restore(StateFile {
        version: state::STATE_VERSION,
        next_id: 2,
        windows: vec![window_record(1, "missing-dir-2", missing, None)],
        runs: Vec::new(),
    });

    let first = m.restart(1).await.unwrap_err();
    assert!(
        first.to_string().contains("directory does not exist"),
        "{first}"
    );

    let second = m.restart(1).await.unwrap_err();
    assert!(
        second.to_string().contains("directory does not exist"),
        "the `restarting` flag leaked after a failed restart, refusing the next one \
         instead of repeating the same cwd error: {second}"
    );
    assert!(
        !second.to_string().contains("already restarting"),
        "{second}"
    );
}

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

/// Major 2 (fix wave 5 review): decision 18's timeout behaviour. `wait_for_exit` used to
/// warn and fall through to phase C regardless, which is exactly the one path the
/// `child_alive` deviation's own safety argument does not cover — see `restart.rs`'s
/// module doc and `wait_for_exit`'s doc comment. On a timeout the restart must be refused
/// with decision 18's exact message, not merely logged and retried.
///
/// Forcing a *real* process to survive `kill`'s HUP/TERM/KILL escalation deterministically
/// is not practical (AGENTS.md: "an interactive shell exits at once" on HUP, and nothing
/// can ignore `SIGKILL`). So this drives the manager-visible side of the race directly: a
/// manager built with `WindowManager::new` whose event receiver is never pumped can never
/// learn that `WindowEvent::Exited` arrived, so `Entry.child_alive` can never go false —
/// deterministically reproducing "the wait ran out" without needing a process that
/// actually refuses to die. `config.kill_grace` is shortened only so the test does not
/// have to sit through the real, unrelated `KILL_GRACE` this window's `kill()` also starts
/// escalating on (`crate::process::escalate` reads that constant directly, not
/// `ManagerConfig.kill_grace`, so shortening this field changes nothing about what
/// actually happens to the real child — only how long `wait_for_exit`'s own deadline is).
#[tokio::test]
async fn restart_refuses_when_the_kill_wait_times_out() {
    let mut config = ManagerConfig::new("/tmp/unused.sock".into(), "/bin/sh".into());
    config.kill_grace = Duration::from_millis(1);
    let (m, _events) = WindowManager::new(config);
    let id = create_id(
        &m,
        spec("never-confirmed-dead"),
        std::env::temp_dir(),
        80,
        24,
    )
    .await;
    assert!(
        find(&m, id).status != Status::Exited,
        "sanity: the window starts out live"
    );

    let err = m.restart(id).await.unwrap_err();
    assert_eq!(
        err.to_string(),
        format!("window {id} did not exit; not restarted")
    );
}

/// Major 3 (fix wave 5 review): `begin_restart` was missing the same `shutting_down`
/// admission check `create`'s `admit` makes. Reachable in production because
/// `requests::restart` detaches (`server/requests.rs`'s own doc comment says it is
/// deliberately never aborted) while `lifecycle::run` calls `manager.shutdown().await`
/// only after `serve` returns — a restart admitted just before shutdown began could still
/// complete afterward and spawn a child nothing would ever kill.
#[tokio::test]
async fn restart_is_refused_after_shutdown() {
    let m = manager();
    let id = create_id(
        &m,
        spec("restart-after-shutdown"),
        std::env::temp_dir(),
        80,
        24,
    )
    .await;
    wait_until("shell started", || find(&m, id).status != Status::Starting).await;

    m.shutdown().await;

    let err = m.restart(id).await.unwrap_err();
    assert!(err.to_string().contains("shutting down"), "{err}");
}

/// Major 1 (fix wave 5 **re-review**): `begin_restart`'s `shutting_down` check above only
/// guards *admission*. It cannot constrain a restart that was already admitted before
/// `shutdown` began — none of `restart`'s exit paths re-reads the flag after phase A, so an
/// in-flight restart can still swap a live process in *after* `shutdown()` has returned,
/// using a flag it read seconds earlier. The fix re-checks `shutting_down` in
/// `finish_restart`, under the very lock the swap itself takes — the one place a race
/// between "admitted" and "shutting down" cannot land ambiguously, because whichever side
/// acquires that lock first is the one the other observes.
///
/// Variant A here forces the race deterministically with a real, signal-ignoring shell
/// (`trap '' HUP TERM`), which only ever dies to the unblockable `SIGKILL`
/// `crate::process::escalate` sends at its hardcoded `KILL_GRACE` (3s, `process.rs`) —
/// giving a wide, real window in which `shutdown` can be called while phase B's kill wait
/// is still outstanding. The same script is `ManagerConfig.shell`, so phase C's fresh spawn
/// for the restart is *also* this stubborn shell, which is what lets the test tell the two
/// processes apart and confirm the second one is truly dead, not merely "swapped out",
/// after everything settles.
#[tokio::test]
async fn restart_admitted_before_shutdown_is_refused_and_leaves_no_process_behind() {
    use std::os::unix::fs::PermissionsExt;

    let dir = tempfile::tempdir().unwrap();
    let pid_log = dir.path().join("pids.log");
    let shell = dir.path().join("stubborn-shell.sh");
    std::fs::write(
        &shell,
        format!(
            "#!/bin/sh\ntrap '' HUP TERM\nprintf '%s\\n' \"$$\" >> '{}'\nwhile :; do :; done\n",
            pid_log.display()
        ),
    )
    .unwrap();
    std::fs::set_permissions(&shell, std::fs::Permissions::from_mode(0o755)).unwrap();

    // `ManagerConfig::new`'s default `kill_grace` is the real `KILL_GRACE` (process.rs),
    // giving `wait_for_exit` its production deadline of `kill_grace + 2s` — comfortably
    // past the ~3s the stubborn shell actually takes to die to `SIGKILL`, so phase B's own
    // wait succeeds for the right reason rather than timing out (that is Minor 2's
    // scenario, not this one).
    let config = ManagerConfig::new("/tmp/unused.sock".into(), shell.display().to_string());
    let (m, mut events) = WindowManager::new(config);
    let pump = m.clone();
    tokio::spawn(async move {
        while let Some((id, ev)) = events.recv().await {
            pump.handle_event(id, ev);
        }
    });

    let id = create_id(
        &m,
        spec("restart-across-shutdown"),
        std::env::temp_dir(),
        80,
        24,
    )
    .await;
    wait_until("stubborn shell logged its pid", || {
        std::fs::read_to_string(&pid_log)
            .map(|s| !s.trim().is_empty())
            .unwrap_or(false)
    })
    .await;

    let restart_task = {
        let m = m.clone();
        tokio::spawn(async move { m.restart(id).await })
    };
    // Give phase A/B a moment to run (set `restarting`, send the first HUP) before
    // `shutdown` sets its own flag — not a bound on anything this test asserts, only
    // ordering: `shutting_down` must land while the kill wait is genuinely outstanding,
    // which is true for a wide, multi-second window here (the shell ignores both HUP and
    // TERM), not a knife's edge this sleep has to hit precisely.
    tokio::time::sleep(Duration::from_millis(400)).await;

    m.shutdown().await;
    let result = restart_task.await.unwrap();

    // Whether phase C's fresh spawn survives is checked by process listing, not by
    // waiting for it to log its own pid: `SIGKILL` can (and, observed while building this
    // test, sometimes does) reach a just-forked child before it has run any of its own
    // script at all, racing ahead of the very `printf` that would have logged it. Matching
    // on this test's own unique script path is unambiguous either way.
    let needle = shell.display().to_string();
    let survivors = || {
        std::process::Command::new("pgrep")
            .arg("-f")
            .arg(&needle)
            .output()
            .map(|out| String::from_utf8_lossy(&out.stdout).trim().to_string())
            .unwrap_or_default()
    };

    if result.is_ok() {
        // Pre-fix behaviour: `finish_restart` swapped a live process in after `shutdown`
        // had already returned, and nothing ever signals it. Confirm and kill whatever is
        // left before asserting RED, rather than leaving a leaked shell behind.
        for pid in survivors().lines().filter_map(|l| l.trim().parse().ok()) {
            // SAFETY: matched by this test's own unique, just-created script path.
            unsafe {
                libc::kill(pid, libc::SIGKILL);
            }
        }
    }

    let err = result.expect_err(
        "a restart admitted before shutdown() must be refused once shutdown() has set its \
         flag, not allowed to swap a live process in afterward",
    );
    assert!(err.to_string().contains("shutting down"), "{err}");

    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let left = survivors();
        if left.is_empty() {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "a process spawned after shutdown() returned is still running: {left}"
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

/// Variant B of the same finding, with no injected timing at all: an ordinary shell that
/// dies on `HUP` at once (AGENTS.md). The re-review reproduced this "first try" because the
/// race is not actually close — once phase B's kill wait has confirmed the old child dead,
/// `shutdown`'s own wait for that same, already-finished escalation resolves close to
/// instantly (a `watch` channel notification), while phase C still has a real PTY spawn
/// ahead of it (tens of ms, `docs/timing-budgets.md`'s idle table). Calling `shutdown` the
/// moment the old child is confirmed gone at the OS level reliably lands inside that gap.
#[tokio::test]
async fn restart_admitted_before_shutdown_ordinary_shell_variant() {
    let m = manager();
    let id = create_id(
        &m,
        spec("post-shutdown-swap-ordinary"),
        std::env::temp_dir(),
        80,
        24,
    )
    .await;
    wait_until("shell started", || find(&m, id).status != Status::Starting).await;
    let original_pid = m.child_pid(id).unwrap().expect("live window has a pid");

    let restart_task = {
        let m = m.clone();
        tokio::spawn(async move { m.restart(id).await })
    };

    // "Wait for the old child to be reaped" — the re-review's own synchronization point
    // for this variant, entirely from outside the manager: once the process group is gone
    // at the OS level, phase B's kill has done its job and `restart` is on its way into
    // phase C.
    let deadline = Instant::now() + Duration::from_secs(5);
    while group_alive(original_pid) {
        assert!(Instant::now() < deadline, "old child was never reaped");
        tokio::time::sleep(Duration::from_millis(1)).await;
    }

    m.shutdown().await;
    let result = restart_task.await.unwrap();

    if result.is_ok() {
        // Pre-fix behaviour: confirm and clean up rather than leaving a leaked shell.
        if let Ok(Some(pid)) = m.child_pid(id) {
            // SAFETY: this pid is this window's own current process, just reported by the
            // manager itself.
            unsafe {
                libc::killpg(pid as libc::pid_t, libc::SIGKILL);
            }
        }
    }

    let err = result.expect_err(
        "a restart admitted before shutdown() must be refused once shutdown() has set its \
         flag, not allowed to swap a live process in afterward",
    );
    assert!(err.to_string().contains("shutting down"), "{err}");
}
