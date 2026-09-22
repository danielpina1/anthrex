//! Fix wave 10, item 1's other half: `tui::spawn::ensure_daemon` must not let two
//! callers racing against an empty data directory each spawn their own competing
//! detached daemon-start child. `daemon::lockfile`'s own `acquire_or_yield` (see
//! `crates/daemon/src/lockfile.rs`'s tests and `crates/daemon/tests/lifecycle.rs`)
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
    let data_dir = dir.path().join("data");
    let socket = dir.path().join("d.sock");
    let marker = dir.path().join("invocations.log");

    // Stands in for the real `anthrex` binary: it never binds the socket (so both
    // `ensure_daemon` calls run their full 3s poll and give up), it just records that
    // it was launched as `daemon start --foreground` — which is all this test needs to
    // count how many competing children actually got spawned.
    let script = dir.path().join("counting_daemon.sh");
    std::fs::write(
        &script,
        format!("#!/bin/sh\necho invoked >> {:?}\nexit 0\n", marker),
    )
    .unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
    }

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
    // own 3s socket-poll — so the invocation count settles almost immediately. Wait
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

    // SAFETY: see above.
    unsafe {
        std::env::remove_var("ANTHREX_DATA_DIR");
        std::env::remove_var("ANTHREX_SOCKET");
    }

    assert_eq!(
        count, 1,
        "exactly one of the two racing ensure_daemon calls must spawn a daemon-start \
         child, not both (and not zero): {contents:?}"
    );

    // Both calls eventually give up on their own (the stub never binds the socket);
    // let them finish so nothing of this test's outlives it.
    let _ = tokio::time::timeout(Duration::from_secs(5), a).await;
    let _ = tokio::time::timeout(Duration::from_secs(5), b).await;
}
