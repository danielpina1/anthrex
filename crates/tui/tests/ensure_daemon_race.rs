//! Fix wave 10, item 1's other half: `tui::spawn::ensure_daemon` must not let two
//! callers racing against an empty data directory each spawn their own competing
//! detached daemon-start child. `daemon::lockfile`'s own `acquire_or_yield` (see
//! `crates/daemon/src/lockfile.rs`'s tests and `crates/daemon/tests/it/lifecycle.rs`)
//! covers what happens if a redundant child *is* spawned anyway; this file proves the
//! redundant spawn is prevented at the source in the first place, which is what
//! actually closes the race in practice for the reported two-caller case (see
//! `.superpowers/sdd/M6-persistence/fix-wave-10-report.md`).

use std::time::{Duration, Instant};

/// `ANTHREX_DATA_DIR`/`ANTHREX_SOCKET` are process-global; this file has exactly one
/// test that touches them, matching `crates/tui/src/spawn.rs`'s own `ENV_LOCK`
/// convention for whoever adds a second. `tokio::sync::Mutex`, not `std::sync::Mutex`:
/// the guard is held across `.await` points below.
static ENV_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

#[tokio::test]
async fn only_one_racing_ensure_daemon_call_spawns_a_child() {
    let _guard = ENV_LOCK.lock().await;
    let dir = tempfile::tempdir().unwrap();
    // Stream S re-review N1: `HOME` points at an empty directory of this test's for the
    // whole test, so a call that ever resolved the default data directory (the isolation
    // variables gone) would create it here, where the end of the test looks, never in
    // the user's real one.
    let home = tempfile::tempdir().unwrap();
    let real_home = std::env::var_os("HOME");
    // SAFETY: serialized by ENV_LOCK; this binary has this one test, and nothing else in
    // it reads the environment concurrently.
    unsafe { std::env::set_var("HOME", home.path()) };
    let data_dir = dir.path().join("data");
    let socket = dir.path().join("d.sock");
    let marker = dir.path().join("invocations.log");

    // Stands in for the real `anthrex` binary: it never binds the socket (so both
    // `ensure_daemon` calls run their full poll, `ENSURE_DAEMON_SOCKET_WAIT` - 10s since
    // milestone 9.11's decision 31, 3s before - and give up), it just records that
    // it was launched as `daemon start --foreground` — which is all this test needs to
    // count how many competing children actually got spawned.
    let script = dir.path().join("counting_daemon.sh");
    testexec::write_executable(
        &script,
        format!("#!/bin/sh\necho invoked >> {:?}\nexit 0\n", marker),
    );

    // SAFETY: serialized by ENV_LOCK; nothing else in this process reads these
    // variables concurrently.
    unsafe {
        std::env::set_var("ANTHREX_DATA_DIR", &data_dir);
        std::env::set_var("ANTHREX_SOCKET", &socket);
    }

    let a = tokio::spawn({
        let script = script.clone();
        let socket = socket.clone();
        async move { tui::spawn::ensure_daemon(&script, &socket).await }
    });
    let b = tokio::spawn({
        let script = script.clone();
        let socket = socket.clone();
        async move { tui::spawn::ensure_daemon(&script, &socket).await }
    });

    // The spawn decision happens right at the top of `ensure_daemon`, long before its
    // own socket poll ends — so the invocation count settles almost immediately. Wait
    // with a deadline loop for the first line to land (AGENTS.md hard rule 6), then
    // hold for a further bounded window to give a second, buggy spawn a real chance to
    // show up before declaring success.
    let deadline = Instant::now() + Duration::from_secs(2);
    loop {
        if !std::fs::read_to_string(&marker)
            .unwrap_or_default()
            .is_empty()
            || Instant::now() >= deadline
        {
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    tokio::time::sleep(Duration::from_millis(300)).await;
    let contents = std::fs::read_to_string(&marker).unwrap_or_default();
    let count = contents.lines().filter(|l| *l == "invoked").count();

    assert_eq!(
        count, 1,
        "exactly one of the two racing ensure_daemon calls must spawn a daemon-start \
         child, not both (and not zero): {contents:?}"
    );

    // Both calls are waited out before the isolation variables go (stream S re-review
    // N1): the loser keeps claiming while it polls and, once the winner's spawn marker is
    // `SPAWN_MARKER_FRESH` (5 s) old with no daemon behind it, spawns a stand-in of its own,
    // which resolves the data directory from the environment at that moment. The stub
    // never binds, so each call ends at its own deadline: `ENSURE_DAEMON_SOCKET_WAIT`
    // (10 s, imported) plus one `SPAWN_HANDOFF_GRACE` (0.25 s); 5 s more for a loaded
    // machine. A call that outlived that fails the test with the variables still set.
    let limit = tui::spawn::ENSURE_DAEMON_SOCKET_WAIT + Duration::from_secs(5);
    for call in [a, b] {
        let _ = tokio::time::timeout(limit, call)
            .await
            .expect("an ensure_daemon call outlived its own deadline");
    }

    // SAFETY: see above.
    unsafe {
        std::env::remove_var("ANTHREX_DATA_DIR");
        std::env::remove_var("ANTHREX_SOCKET");
    }

    let touched: Vec<_> = std::fs::read_dir(home.path())
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .collect();
    // SAFETY: see above.
    unsafe {
        match real_home {
            Some(value) => std::env::set_var("HOME", value),
            None => std::env::remove_var("HOME"),
        }
    }
    assert!(
        touched.is_empty(),
        "an ensure_daemon call of this test ran without its isolation variables and \
         reached the default data directory: {touched:?}"
    );
}
