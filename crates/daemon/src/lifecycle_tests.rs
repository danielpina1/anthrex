//! `lifecycle`'s own unit tests, split out under the repo's `#[path = "..._tests.rs"]`
//! convention (as `server.rs`/`server_tests.rs` already do) to keep `lifecycle.rs`
//! inside AGENTS.md rule 8's ~600-line guideline. A pure move: these are the same
//! tests, still a child module of `lifecycle`, so they still reach its private
//! `bind_socket_locked` and `UMASK_LOCK`.

use super::*;

#[test]
fn stale_socket_file_is_removed() {
    let dir = tempfile::tempdir().unwrap();
    let sock = dir.path().join("sub").join("d.sock");
    std::fs::create_dir_all(sock.parent().unwrap()).unwrap();
    std::fs::write(&sock, b"not a socket").unwrap();
    prepare_socket(&sock).unwrap();
    assert!(!sock.exists());
    let mode = std::fs::metadata(sock.parent().unwrap())
        .unwrap()
        .permissions()
        .mode()
        & 0o777;
    assert_eq!(mode, 0o700);
}

#[test]
fn live_socket_is_an_error() {
    let dir = tempfile::tempdir().unwrap();
    let sock = dir.path().join("d.sock");
    let _listener = std::os::unix::net::UnixListener::bind(&sock).unwrap();
    let err = prepare_socket(&sock).unwrap_err();
    assert!(err.to_string().contains("already"));
    assert!(sock.exists());
}

/// I5: a root-owned sticky directory (/tmp) is the deliberate override and stays
/// allowed - nobody but us can replace what we create inside it.
#[test]
fn root_owned_sticky_parent_is_tolerated() {
    let tmp = PathBuf::from("/tmp");
    // SAFETY: getuid has no preconditions and cannot fail.
    let current_uid = unsafe { libc::getuid() };
    let meta = std::fs::metadata(&tmp).unwrap();
    if meta.uid() == current_uid {
        // Running as root (or otherwise owns /tmp): the scenario this test exercises
        // (a parent directory we don't own) doesn't apply here.
        return;
    }
    assert_eq!(meta.uid(), 0, "/tmp is expected to be root-owned");
    assert_ne!(meta.mode() & 0o1000, 0, "/tmp is expected to be sticky");
    let sock = tmp.join(format!("anthrex-prep-{}.sock", std::process::id()));
    assert!(!sock.exists());
    let result = prepare_socket(&sock);
    assert!(result.is_ok(), "{result:?}");
}

/// I5: any other directory we do not own is refused by name, because an attacker who
/// pre-created it would otherwise see every keystroke going through the socket.
#[test]
fn foreign_owned_parent_without_the_sticky_bit_is_fatal() {
    // SAFETY: getuid has no preconditions and cannot fail.
    let current_uid = unsafe { libc::getuid() };
    if current_uid == 0 {
        return; // root can chmod anything, so there is no failure to observe.
    }
    // /usr is root-owned and NOT sticky: exactly the shape of a directory another
    // user pre-created for us.
    let dir = PathBuf::from("/usr");
    let meta = std::fs::metadata(&dir).unwrap();
    assert_ne!(meta.uid(), current_uid);
    assert_eq!(meta.mode() & 0o1000, 0);
    let err = prepare_socket(&dir.join("anthrex-should-never-bind.sock"))
        .unwrap_err()
        .to_string();
    assert!(err.contains("/usr"), "{err}");
    assert!(err.contains(&format!("uid {}", meta.uid())), "{err}");
}

/// I5: the socket file itself must end up 0600, since the directory may be shared.
#[tokio::test]
async fn the_bound_socket_is_chmod_0600() {
    let dir = tempfile::tempdir().unwrap();
    let sock = dir.path().join("d.sock");
    prepare_socket(&sock).unwrap();
    let _listener = bind_socket(&sock).unwrap();
    assert_eq!(
        std::fs::metadata(&sock).unwrap().permissions().mode() & 0o777,
        0o600
    );
}

/// Decision 25: `bind_socket` must leave the process's umask exactly as it found it,
/// whatever that was — a mutation that forgot to restore it, or restored a hardcoded
/// value instead of the one it read, would leak a tightened (or loosened) mask into
/// every file this process creates afterwards.
///
/// The starting mask is deliberately `0o037`, not `0o022`: `0o022` is both the value
/// this test used to set *and* the value it asserted was restored, so a mutation that
/// hard-coded the restore to `unsafe { libc::umask(0o022) }` instead of round-tripping
/// `previous_umask` passed unchanged (finding 2, M6 fix wave 1). `0o037` is also not a
/// common real-world default, unlike `0o022`, so a mutation that hard-codes some other
/// plausible-looking default is caught too.
///
/// Holds `UMASK_LOCK` for the whole sequence and calls the lock-free
/// `bind_socket_locked` directly (not the public `bind_socket`, which would try to
/// take the same lock and deadlock): umask is process-wide, and other tests in this
/// binary call `bind_socket` concurrently, so without holding the lock across its own
/// two raw `umask` calls too this test is racy against them. Whether `bind_socket`
/// itself takes `UMASK_LOCK` is a separate property, covered by
/// `bind_socket_serializes_concurrent_umask_use` below, which goes through the public
/// entry point precisely because this test cannot (see that test's doc comment).
#[tokio::test]
async fn bind_restores_the_umask() {
    let dir = tempfile::tempdir().unwrap();
    let sock = dir.path().join("d.sock");
    let _guard = crate::lock(&UMASK_LOCK);
    // SAFETY: umask has no preconditions and cannot fail; both calls here bracket the
    // temporary value this test sets so the process's real mask is restored after.
    // `UMASK_LOCK` is held for the whole bracket, so no concurrently running test can
    // observe or clobber the value in between.
    let real_mask = unsafe { libc::umask(0o037) };
    let result = bind_socket_locked(&sock);
    // SAFETY: see above.
    let mask_after_bind = unsafe { libc::umask(real_mask) };
    let _listener = result.unwrap();
    assert_eq!(
        mask_after_bind, 0o037,
        "bind_socket did not restore the umask it found"
    );
    assert_eq!(
        std::fs::metadata(&sock).unwrap().permissions().mode() & 0o777,
        0o600
    );
}

/// Finding 1, M6 fix wave 1 (task-2 review): `bind_restores_the_umask` above never
/// calls the *public* `bind_socket` — by construction it can't, since it holds
/// `UMASK_LOCK` itself and `bind_socket` would deadlock trying to take the same lock.
/// So deleting `bind_socket`'s `let _guard = crate::lock(&UMASK_LOCK);` line left
/// every test in this module green.
///
/// `libc::umask` is process-global, so without that lock, concurrent `bind_socket`
/// calls' read-tighten-restore sequences can interleave: thread A reads the ambient
/// mask and tightens to `0o077`; thread B, running concurrently, reads *A's* `0o077`
/// as if it were ambient and later restores to that instead of to what was truly
/// ambient before either started. The mask observed once every call has returned is
/// then wrong, even though neither call did anything incorrect in isolation.
///
/// This drives many concurrent calls through the public `bind_socket` and checks the
/// ambient mask survives the burst unchanged. Reading the mask itself, before and
/// after, is bracketed by `UMASK_LOCK` too — not part of what's under test, since
/// `bind_socket` takes the very same lock as long as its guard line still exists; this
/// only keeps those two reads race-free against the burst and against
/// `bind_restores_the_umask`'s own direct umask manipulation.
#[test]
fn bind_socket_serializes_concurrent_umask_use() {
    let dir = tempfile::tempdir().unwrap();

    let starting = {
        let _guard = crate::lock(&UMASK_LOCK);
        // SAFETY: umask has no preconditions and cannot fail. `UMASK_LOCK` is held
        // for both calls, so this round trip is race-free.
        unsafe {
            let m = libc::umask(0o000);
            libc::umask(m);
            m
        }
    };

    // `bind_socket` calls `tokio::net::UnixListener::bind`, which needs an active
    // Tokio reactor context even though `bind_socket` itself is synchronous; each
    // plain `std::thread` below enters this runtime's context explicitly rather than
    // being spawned as a `#[tokio::test]` task, so the burst is genuine OS-thread
    // concurrency, not tasks cooperatively yielding on one or a few worker threads.
    let rt = tokio::runtime::Runtime::new().unwrap();
    let handle = rt.handle().clone();
    let threads: Vec<_> = (0..16)
        .map(|i| {
            let sock = dir.path().join(format!("umask-race-{i}.sock"));
            let handle = handle.clone();
            std::thread::spawn(move || {
                let _guard = handle.enter();
                let _listener = bind_socket(&sock).unwrap();
            })
        })
        .collect();
    for t in threads {
        t.join().unwrap();
    }

    let ending = {
        let _guard = crate::lock(&UMASK_LOCK);
        // SAFETY: see above.
        unsafe {
            let m = libc::umask(starting);
            libc::umask(m);
            m
        }
    };
    assert_eq!(
        ending, starting,
        "16 concurrent bind_socket calls left the ambient umask changed: {ending:o} != {starting:o}"
    );
}
