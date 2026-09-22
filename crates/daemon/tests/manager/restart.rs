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
/// with `restart`'s own `restart_wait_deadline` (production default `KILL_GRACE + 2s`),
/// the arithmetic-equality shape `docs/timing-budgets.md`'s standing rule 1 flags as a
/// defect regardless of how the number was arrived at. In the ordinary case here the
/// shell dies on SIGHUP at once (AGENTS.md), so the real cost is nowhere near either
/// bound.
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

#[path = "restart/stale_cleanup.rs"]
mod stale_cleanup;

#[path = "restart/shutdown_race.rs"]
mod shutdown_race;
