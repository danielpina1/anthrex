//! The refusal and shutdown-race tests: Major 2 and 3 (fix wave 5 review) — the
//! kill-wait timeout must refuse a restart rather than proceed anyway, and a restart must
//! be refused once `shutdown` has run — Major 1 (the re-review: an already-admitted
//! restart must still be refused if `shutdown` lands mid-flight, in two variants), Minor 2
//! (a kill-wait timeout must not leave `cleanups[id]` stale for a later `kill`), and item 4
//! (`shutdown` must wait for a restarted leader's orphaned descendant). A submodule of
//! `restart.rs` (AGENTS.md rule 8) sharing its `group_alive` helper via `use super::*`.

use super::*;

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
/// actually refuses to die. `config.restart_wait_deadline` is shortened only so the test
/// does not have to sit through its real, production-sized default; `kill_grace` is left
/// alone, since the real, unrelated `KILL_GRACE` this window's `kill()` also starts
/// escalating on (`crate::process::escalate` reads that constant directly, not
/// `ManagerConfig.kill_grace`) is unaffected either way.
#[tokio::test]
async fn restart_refuses_when_the_kill_wait_times_out() {
    let mut config = ManagerConfig::new("/tmp/unused.sock".into(), "/bin/sh".into());
    config.restart_wait_deadline = Duration::from_millis(1);
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

    // `ManagerConfig::new`'s default `restart_wait_deadline` is `KILL_GRACE + 2s` —
    // comfortably past the ~3s the stubborn shell actually takes to die to `SIGKILL`, so
    // phase B's own wait succeeds for the right reason rather than timing out (that is
    // Minor 2's scenario, not this one).
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

/// Minor 2 (fix wave 5 re-review): the kill-wait timeout refusal (Major 2, the previous
/// wave) leaves `cleanups[id]` populated. `tick`'s retain keeps that record for as long as
/// the entry exists (`entries.contains_key(id) || !*done.borrow()`, `mod.rs`), which is
/// forever for a window that is merely refused, not removed — so a later `kill(id)` for the
/// same window becomes the exact silent no-op Critical 1 fixed, on the one path this wave's
/// own Major 2 fix created.
///
/// Driven with a real, *non*-ignoring script that logs each `HUP`/`TERM` it receives rather
/// than exiting on them, so a *second*, independent signal delivery is directly observable
/// (a repeat of the same eventual death is not: nothing distinguishes "a fresh escalation
/// re-signalled" from "the original one was always going to get there").
///
/// `config.restart_wait_deadline` is shortened so `wait_for_exit` gives up long before the
/// real, hardcoded `KILL_GRACE` (`process.rs`, unaffected by this) the original escalation
/// needs to finally force the issue with `SIGKILL` — guaranteeing a genuine timeout, not a
/// race. Fix wave 8, Minor: this used to shorten `config.kill_grace` instead, relying on
/// `wait_for_exit`'s deadline being computed as `kill_grace + 2s` — an injected value
/// racing a hardcoded one with a margin of 900ms (`3s - (100ms + 2s)`), the exact
/// injected-vs-hardcoded near-equality shape `docs/timing-budgets.md`'s standing rule 1
/// warns about, even though the margin was provably positive by construction. Now that
/// `wait_for_exit` reads its own field, this sets it directly to a small value with no
/// relationship to `kill_grace` at all (left at its default, `KILL_GRACE`, since nothing
/// here uses it) — a ~2.9s margin against the real `KILL_GRACE`, about 3x wider than
/// before, and no longer built from two constants that merely happened not to coincide.
#[tokio::test]
async fn kill_after_a_timed_out_restart_still_signals_the_window() {
    use std::os::unix::fs::PermissionsExt;

    let dir = tempfile::tempdir().unwrap();
    let log = dir.path().join("signals.log");
    let ready = dir.path().join("ready");
    let shell = dir.path().join("logging-shell.sh");
    std::fs::write(
        &shell,
        format!(
            "#!/bin/sh\ntrap 'echo hup >> \"{}\"' HUP\ntrap 'echo term >> \"{}\"' TERM\n\
             : > \"{}\"\nwhile :; do :; done\n",
            log.display(),
            log.display(),
            ready.display()
        ),
    )
    .unwrap();
    std::fs::set_permissions(&shell, std::fs::Permissions::from_mode(0o755)).unwrap();

    let mut config = ManagerConfig::new("/tmp/unused.sock".into(), shell.display().to_string());
    // See this test's own doc comment: `restart_wait_deadline` is independent of
    // `kill_grace` (left at its default), so this sets `wait_for_exit`'s own deadline
    // directly rather than deriving it from a value the real escalation also depends on.
    config.restart_wait_deadline = Duration::from_millis(100);
    let (m, mut events) = WindowManager::new(config);
    let pump = m.clone();
    tokio::spawn(async move {
        while let Some((id, ev)) = events.recv().await {
            pump.handle_event(id, ev);
        }
    });

    let id = create_id(&m, spec("timeout-then-kill"), std::env::temp_dir(), 80, 24).await;
    wait_until("logging shell's traps are registered", || ready.exists()).await;

    // Guarantees the process is dead before this test ends, on every exit path including
    // a panicking assertion below — this script only ever dies to `SIGKILL`, so nothing
    // but an explicit signal from this guard, or the original escalation's own hardcoded
    // one, will ever end it.
    struct KillOnDrop(Arc<WindowManager>, u32);
    impl Drop for KillOnDrop {
        fn drop(&mut self) {
            if let Ok(Some(pid)) = self.0.child_pid(self.1) {
                // SAFETY: this is this test's own window's own process.
                unsafe {
                    libc::killpg(pid as libc::pid_t, libc::SIGKILL);
                }
            }
        }
    }
    let _cleanup = KillOnDrop(m.clone(), id);

    let err = m.restart(id).await.unwrap_err();
    assert_eq!(
        err.to_string(),
        format!("window {id} did not exit; not restarted")
    );

    // By now the original escalation has sent HUP (t=0) and TERM (t=1s per the real,
    // hardcoded `HUP_GRACE`) — both logged, neither fatal to this script.
    let after_timeout = std::fs::read_to_string(&log).unwrap_or_default();
    assert_eq!(
        after_timeout.matches("hup").count(),
        1,
        "sanity: exactly one HUP before the retry: {after_timeout:?}"
    );

    // The regression: a fresh, explicit `kill` for this same window must actually signal
    // it again, not silently believe cleanup is already in hand.
    m.kill(id).unwrap();
    let deadline = Instant::now() + Duration::from_secs(1);
    loop {
        let hups = std::fs::read_to_string(&log)
            .unwrap_or_default()
            .matches("hup")
            .count();
        if hups >= 2 {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "a second kill() after a timed-out restart sent no new signal — \
             cleanups[id] was left stale, the exact no-op Critical 1 fixed"
        );
        tokio::time::sleep(Duration::from_millis(10)).await;
    }

    // The original escalation's own hardcoded `SIGKILL` lands on its own schedule
    // regardless of any of the above; wait for the real death rather than declaring
    // victory the moment a second signal was merely sent.
    let pid = m.child_pid(id).unwrap();
    if let Some(pid) = pid {
        wait_until("the window's process to actually die", || !group_alive(pid)).await;
    }
}

/// Item 4 (fix wave 5 re-review): the restart analogue of
/// `lifecycle::shutdown_waits_for_cleanup_after_the_group_leader_exits`, and the test
/// `orphaned_cleanups` had none of before this — a grep for `orphaned_cleanups` across
/// every test crate in the workspace found nothing, so a future simplification of
/// `Inner::orphan_cleanup` back to a bare `self.cleanups.remove(&id)` would have been green
/// on all 785 other tests.
///
/// The mechanism this pins: `finish_restart` only ever runs once the old group leader is
/// confirmed gone (`child_alive == false`), which makes it tempting to conclude the
/// escalation behind it has therefore finished. It has not — `crate::process::escalate`
/// polls the whole process *group*, not the leader alone, so a leader that exits on `HUP`
/// while leaving a descendant behind keeps its own escalation thread running, through
/// `HUP_GRACE` to `SIGTERM`, well past the moment `finish_restart` evicts its record from
/// `cleanups` to make room for this id's new, live process. Moving that record to
/// `orphaned_cleanups` instead of dropping it is what keeps `shutdown` able to wait for it;
/// a bare `remove` would leave nothing to wait for and the descendant would never be
/// signalled.
///
/// Deliberately simpler than the `shutdown`-after-`kill` sibling this mirrors: that test
/// isolates a Linux child-subreaper setting so it can positively confirm the descendant was
/// *reaped*. This test only needs to confirm `TERM` was *delivered* before `shutdown`
/// returns (`term_file`'s existence, written synchronously by the descendant's own trap
/// before it exits), which needs no subreaper — the descendant is signalled by process
/// *group*, not by parent/child relationship, and whoever ends up reaping it once it exits
/// (`init` on Linux once reparented, `launchd` on macOS) is not this test's concern.
#[tokio::test]
async fn shutdown_after_a_restart_waits_for_the_old_leaders_orphaned_descendant() {
    let dir = tempfile::tempdir().unwrap();
    let pid_file = dir.path().join("descendant.pid");
    let term_file = dir.path().join("term");
    let descendant = dir.path().join("descendant.sh");
    let leader = dir.path().join("leader.sh");
    std::fs::write(
        &descendant,
        format!(
            "trap '' HUP\ntrap 'printf term > \"{}\"; exit 0' TERM\nprintf '%s' \"$$\" > '{}'\nwhile :; do :; done\n",
            term_file.display(),
            pid_file.display()
        ),
    )
    .unwrap();
    std::fs::write(
        &leader,
        format!(
            "trap 'exit 0' HUP\n/bin/sh '{}' &\nwait\n",
            descendant.display()
        ),
    )
    .unwrap();

    let m = manager();
    let id = create_id(
        &m,
        spec("restart-cleanup-descendant"),
        std::env::temp_dir(),
        80,
        24,
    )
    .await;
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

    // The restart's own phase B kills the leader (`HUP`, trapped to `exit 0` at once,
    // leaving the descendant behind in the group); phase C then spawns a fresh, ordinary
    // shell to replace it.
    m.restart(id).await.unwrap();
    wait_until("restart leaves Exited", || {
        find(&m, id).status != Status::Exited
    })
    .await;

    let started = Instant::now();
    m.shutdown().await;
    let term_delivered_before_return = term_file.exists();
    let elapsed = started.elapsed();

    if !term_delivered_before_return {
        // SAFETY: this pid is the descendant this test's own fixture spawned; kill what
        // this probe would otherwise prove leaked, before asserting on it.
        unsafe {
            libc::kill(descendant_pid, libc::SIGKILL);
        }
    }
    assert!(
        term_delivered_before_return,
        "shutdown returned after {elapsed:?} without the old leader's orphaned descendant \
         ever being signalled — orphaned_cleanups must keep shutdown waiting for an \
         escalation finish_restart evicted from cleanups, or a live descendant a restart \
         leaves behind survives the daemon's own shutdown"
    );
}
