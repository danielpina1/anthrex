use super::*;
use proto::Runtime;

fn spec(name: &str, cwd: std::path::PathBuf) -> WindowSpec {
    WindowSpec {
        name: Some(name.to_string()),
        runtime: Runtime::Shell,
        cwd,
        worktree_branch: None,
        model: None,
        initial_prompt: None,
    }
}

/// Minor 1 (fix wave 8 re-review): the third instance of the same defect shape —
/// phase C's own failure path (the window's `cwd` no longer exists by the time
/// `spawn_for_restart` checks it, `spawn_for_restart`'s own `directory does not
/// exist` bail above) left `cleanups[id]` behind. Phase B's `self.kill(id)?` inserts
/// the record and confirms the child dead; `finish_restart`, the only site that used
/// to evict it, is never reached because phase C bails first. `tick`'s retain
/// (`entries.contains_key(id) || !*done.borrow()`) then keeps the stale record for as
/// long as the window itself exists — forever, for a window merely refused, not
/// removed.
///
/// Reaches `Inner.cleanups` directly (`pub(super)`, visible from this descendant of
/// `manager`) rather than trying to observe the leak from outside the crate: every
/// black-box consequence this record could have (masking a fresh `kill`, blocking a
/// later `start_cleanup`) is gated on `entry.child_alive`, which is already `false`
/// by the time phase C runs, on every path that reaches it — exactly why the
/// re-review calls this Minor, not Major. That does not make the record itself
/// harmless to leave behind, only harmless *today*; this test pins the record's
/// absence directly rather than waiting for a future path to make it observable from
/// outside the crate.
#[tokio::test]
async fn a_directory_gone_before_phase_c_does_not_leave_a_stale_cleanups_record() {
    let (m, mut events) = WindowManager::new(ManagerConfig::new(
        "/tmp/unused-restart-cleanup-test.sock".into(),
        "/bin/sh".into(),
    ));
    let pump = m.clone();
    tokio::spawn(async move {
        while let Some((id, ev)) = events.recv().await {
            pump.handle_event(id, ev);
        }
    });

    let dir = tempfile::tempdir().unwrap();
    let cwd = dir.path().to_path_buf();
    let info = m
        .create(
            spec("vanishing-cwd", cwd.clone()),
            cwd.clone(),
            None,
            80,
            24,
        )
        .await
        .unwrap();
    let id = info.id;
    assert!(
        crate::lock(&m.inner)
            .entries
            .get(&id)
            .is_some_and(|e| e.child_alive),
        "sanity: the window starts out live"
    );

    // Deleting the directory out from under the still-live shell: the shell keeps
    // running (its cwd is only a name, not a hold on the directory's existence), so
    // phase B's kill-and-wait still confirms a clean exit; only phase C's fresh
    // `cwd.is_dir()` check, which the restarted process would need to actually start
    // in, sees it gone. `TempDir`'s own `Drop` is skipped since this removes the
    // directory itself.
    std::fs::remove_dir_all(&dir).unwrap();
    std::mem::forget(dir);

    let err = m.restart(id).await.unwrap_err();
    assert!(
        err.to_string().contains("directory does not exist"),
        "{err}"
    );

    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    loop {
        let stale = crate::lock(&m.inner).cleanups.contains_key(&id);
        if !stale {
            break;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "cleanups[{id}] was still present after phase C's own failure path — \
             the third instance of the stale-record defect, now on the \
             directory-gone path"
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

/// fix-wave-12-re-review Minor 2: `Restarting::drop`'s doc comment claimed
/// `orphan_cleanup` was a no-op "when phase B was never reached (nothing was ever
/// inserted under this id)". True before whole-branch-review Major 1 moved the cwd
/// check ahead of phase B; false after — the new check can now bail *before* this
/// restart attempt's own phase B ever calls `self.kill`, while `cleanups[id]` already
/// holds a live record from a *prior*, unrelated `kill()`. The old, unconditional
/// `orphan_cleanup` call evicted that record anyway, stranding it in
/// `orphaned_cleanups` where nothing keyed on `id` will find it again: a later
/// `start_cleanup(id)` no longer short-circuits on `cleanups.contains_key`, and spawns
/// a second `escalate` on the same still-alive pid.
///
/// Events are deliberately never pumped (same technique as
/// `restart_refuses_when_the_kill_wait_times_out` in
/// `daemon/tests/manager/restart/shutdown_race.rs`), so `Entry.child_alive` can never
/// observe the real process's actual fate — this pins the manager-visible race
/// deterministically rather than depending on how fast a real shell dies to `SIGHUP`.
#[tokio::test]
async fn a_cwd_bail_before_phase_b_does_not_orphan_an_unrelated_kills_record() {
    let (m, _events) = WindowManager::new(ManagerConfig::new(
        "/tmp/unused-restart-cwd-bail-test.sock".into(),
        "/bin/sh".into(),
    ));

    let dir = tempfile::tempdir().unwrap();
    let cwd = dir.path().to_path_buf();
    let info = m
        .create(spec("cwd-bail", cwd.clone()), cwd.clone(), None, 80, 24)
        .await
        .unwrap();
    let id = info.id;
    assert!(
        crate::lock(&m.inner)
            .entries
            .get(&id)
            .is_some_and(|e| e.child_alive),
        "sanity: the window starts out live"
    );

    // A separate, unrelated `kill()` — not through `restart` — inserts the cleanup
    // record this test protects.
    m.kill(id).unwrap();
    assert!(
        crate::lock(&m.inner).cleanups.contains_key(&id),
        "sanity: kill() inserted a cleanup record"
    );

    // Remove the cwd out from under the still-"live" (per the manager's own
    // bookkeeping — events are never pumped) window, then restart it: the new
    // between-phase-A-and-B check must bail before this attempt ever touches phase B.
    std::fs::remove_dir_all(&dir).unwrap();
    std::mem::forget(dir);
    let err = m.restart(id).await.unwrap_err();
    assert!(
        err.to_string().contains("directory does not exist"),
        "{err}"
    );

    // The record belongs to the earlier, unrelated kill() — this restart attempt never
    // reached phase B, so it must be left exactly where it was, not moved into
    // `orphaned_cleanups`.
    let inner = crate::lock(&m.inner);
    assert!(
        inner.cleanups.contains_key(&id),
        "cleanups[{id}] was moved to orphaned_cleanups by a restart attempt that never \
         reached phase B — a later start_cleanup(id) would now spawn a second escalate \
         on the same still-alive pid"
    );
    assert!(
        inner.orphaned_cleanups.is_empty(),
        "the unrelated kill()'s record should not have been orphaned"
    );
}

/// Final-gate finding F1: `phase_b_entered` used to be set unconditionally right
/// before phase B's `self.kill(id)`, not derived from whether that kill actually
/// inserted a `cleanups[id]` record. `start_cleanup` (`entry.rs`) returns `Ok(())`
/// early — inserting nothing — when `cleanups[id]` is already occupied, which happens
/// whenever an *unrelated*, earlier `kill()` on the same id is still escalating. This
/// attempt then owns no record at all, yet used to mark itself as having entered phase
/// B regardless, and `Restarting::drop` evicted the *other* operation's live record on
/// this attempt's own timeout — exactly the harm the eviction was built to prevent,
/// just reached by a fifth path instead of the four already named in `restart.rs`'s
/// module doc comment.
///
/// `restart_wait_deadline` is set to 1ms so phase B's wait times out virtually
/// immediately. Events are deliberately never pumped (same technique as
/// `a_cwd_bail_before_phase_b_does_not_orphan_an_unrelated_kills_record`), so
/// `Entry.child_alive` never observes the real process's actual death — without that,
/// a real shell can die to `SIGHUP` inside the 1ms deadline and `wait_for_exit` would
/// see `child_alive == false` and return `true` before the deadline is ever checked,
/// making the restart succeed instead of timing out and turning this into a flaky
/// test of the wrong path.
#[tokio::test]
async fn a_kill_wait_timeout_does_not_orphan_an_unrelated_kills_record() {
    let mut config = ManagerConfig::new(
        "/tmp/unused-restart-foreign-record-test.sock".into(),
        "/bin/sh".into(),
    );
    config.restart_wait_deadline = Duration::from_millis(1);
    let (m, _events) = WindowManager::new(config);

    let dir = tempfile::tempdir().unwrap();
    let cwd = dir.path().to_path_buf();
    let info = m
        .create(
            spec("foreign-cleanup-record", cwd.clone()),
            cwd.clone(),
            None,
            80,
            24,
        )
        .await
        .unwrap();
    let id = info.id;
    assert!(
        crate::lock(&m.inner)
            .entries
            .get(&id)
            .is_some_and(|e| e.child_alive),
        "sanity: the window starts out live"
    );

    // A separate, unrelated `kill()` — not through `restart` — inserts the cleanup
    // record this test protects. Its escalation thread is real and will eventually
    // reap the child; the point is that phase B's own `self.kill(id)` call below finds
    // `cleanups[id]` already occupied and inserts nothing of its own.
    m.kill(id).unwrap();
    assert!(
        crate::lock(&m.inner).cleanups.contains_key(&id),
        "sanity: kill() inserted a cleanup record"
    );

    // `restart_wait_deadline` is 1ms, so phase B's wait times out almost immediately —
    // this attempt calls `self.kill(id)` (short-circuiting on the unrelated record
    // above, inserting nothing), then bails with "did not exit" before the child is
    // actually confirmed gone.
    let err = m.restart(id).await.unwrap_err();
    assert!(err.to_string().contains("did not exit"), "{err}");

    // The record belongs to the earlier, unrelated kill() — this restart attempt never
    // inserted a record of its own, so it must be left exactly where it was, not moved
    // into `orphaned_cleanups` out from under the operation that actually owns it.
    let inner = crate::lock(&m.inner);
    assert!(
        inner.cleanups.contains_key(&id),
        "LEAK: cleanups[{id}] (from the unrelated kill) was moved into \
         orphaned_cleanups by a restart attempt whose own kill inserted nothing"
    );
    assert!(
        inner.orphaned_cleanups.is_empty(),
        "the unrelated kill()'s record should not have been orphaned"
    );
}

/// Whole-branch-review Major 1: decision 19's cwd precondition used to be checked
/// only in phase C, *after* phase B's kill had already ended the live process — a
/// restart the user was told was refused had, in fact, destroyed their running
/// agent. Live reproduction from the review: `rm -rf` a live shell's cwd, run
/// `restart` → refused with `directory does not exist`, and the window is left
/// `exited` anyway. This pins the fix directly: the refusal must land *before* the
/// live process is touched at all, not merely before the caller sees the error.
#[tokio::test]
async fn a_refused_restart_does_not_kill_the_live_process() {
    let (m, mut events) = WindowManager::new(ManagerConfig::new(
        "/tmp/unused-restart-refusal-test.sock".into(),
        "/bin/sh".into(),
    ));
    let pump = m.clone();
    tokio::spawn(async move {
        while let Some((id, ev)) = events.recv().await {
            pump.handle_event(id, ev);
        }
    });

    let dir = tempfile::tempdir().unwrap();
    let cwd = dir.path().to_path_buf();
    let info = m
        .create(
            spec("live-cwd-removed", cwd.clone()),
            cwd.clone(),
            None,
            80,
            24,
        )
        .await
        .unwrap();
    let id = info.id;

    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    let pid = loop {
        if let Some(pid) = crate::lock(&m.inner)
            .entries
            .get(&id)
            .filter(|e| e.child_alive)
            .and_then(|e| e.pid())
        {
            break pid;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "window never came alive with a pid"
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    };

    // The shell keeps running once its cwd is gone (its cwd is only a name); only
    // the restart's own precondition check sees it missing.
    std::fs::remove_dir_all(&dir).unwrap();
    std::mem::forget(dir);

    let err = m.restart(id).await.unwrap_err();
    assert!(
        err.to_string().contains("directory does not exist"),
        "{err}"
    );

    // The refusal must not have touched the live process at all: still alive at
    // the OS level, and the manager must still consider it so — not the
    // pre-fix behaviour, where the window came back listed as `exited`.
    assert!(
        // SAFETY: `pid` is this window's own live process, just reported by the
        // manager itself; signal 0 performs only the existence check.
        unsafe { libc::killpg(pid as libc::pid_t, 0) == 0 },
        "a refused restart killed the live process anyway"
    );
    assert!(
        crate::lock(&m.inner).entries.get(&id).unwrap().child_alive,
        "a refused restart cleared child_alive on the live entry"
    );
    assert_ne!(
        crate::lock(&m.inner).entries.get(&id).unwrap().status,
        Status::Exited,
        "a refused restart left the still-live window listed as exited"
    );

    // Clean up: this window still owns a real running shell.
    let _ = m.kill(id);
}
