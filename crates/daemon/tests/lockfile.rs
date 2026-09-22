//! Isolates one assertion out of `daemon::lockfile`'s own `--lib` unit tests: that a
//! `DaemonLock`, once released, is reacquirable with `Duration::ZERO` — no wait at all.
//!
//! That assertion used to live at the end of `lockfile::tests::second_acquire_fails_while_held`
//! (see that test's comment) as a `Duration::from_secs(2)` wait instead, because the
//! zero-wait version was intermittently failing there. M6 fix wave 1 (task-2 review
//! finding 3) tracked the real cause down experimentally rather than just widening the
//! margin further:
//!
//! - A standalone C reproduction of the exact acquire/conflict/close/zero-wait-reacquire
//!   sequence survived 100,000 iterations under 28-way CPU saturation with zero failures —
//!   ruling out plain OS scheduling jitter or the kernel being slow to make a just-closed
//!   `flock` visible.
//! - The same sequence, run only against `lockfile::` (`cargo test -p anthrex-daemon --lib
//!   lockfile:: -- --test-threads=16`) under the same CPU load, also had zero failures in
//!   20,000 iterations.
//! - Run unfiltered inside the full `--lib` binary — alongside this crate's real,
//!   `portable-pty`-spawning window/process tests running on other threads of the *same*
//!   process — it failed on iteration 0.
//!
//! The mechanism: `fork()` duplicates a process's *entire* file descriptor table,
//! regardless of `O_CLOEXEC` — that flag only takes effect at the child's own later
//! `exec()`. So for the window between an unrelated sibling test's `fork()` (setting up a
//! PTY child's session before its `exec()`) and that child's `exec()`, the child can hold
//! a live duplicate of whatever fd was open in the parent at that instant, including a
//! lock file fd some *other*, concurrently-running instance of this same test had just
//! `close()`d. That extends the flock's effective lifetime past the original holder's own
//! `drop()`, and a strictly `Duration::ZERO` reacquire can lose that race — not because the
//! kernel is slow, but because a second process is transiently, legitimately still holding
//! the same lock.
//!
//! This is its own test binary (Rust's `tests/*.rs` convention: each file compiles to a
//! separate process), and nothing here spawns a PTY or forks a child process, so no
//! sibling test's `fork()` can ever run in this process — the assertion is deterministic
//! here in a way it structurally cannot be inside `--lib`.

use daemon::lockfile::DaemonLock;
use std::time::Duration;

/// A released lock is available again with `Duration::ZERO` — no retry-loop wait needed.
/// `acquire_waits_for_release` (in `lockfile.rs`'s own unit tests) proves a *background*
/// release unblocks a waiting `acquire` within its wait; it cannot stand in for this,
/// because it never checks that the wait can be zero.
#[test]
fn instant_reacquire_after_release() {
    let dir = tempfile::tempdir().unwrap();
    let first = DaemonLock::acquire(dir.path(), Duration::ZERO).unwrap();
    drop(first);
    let _second = DaemonLock::acquire(dir.path(), Duration::ZERO)
        .expect("a released lock must be reacquirable with no wait at all");
}
