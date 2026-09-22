//! The daemon's lifetime lock: `<data_dir>/daemon.lock`, held with `flock` for as long as
//! the daemon owns its data directory. See design decision 24. Two daemons can never share
//! a data directory while this lock works: the second `acquire` blocks (or fails, once its
//! wait is exhausted) until the first releases it, which happens only when its `DaemonLock`
//! value is dropped at the end of `lifecycle::run`.

use std::fs::OpenOptions;
use std::io;
use std::os::fd::AsRawFd;
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

/// How often a blocked `acquire` or `wait_released` retries the lock.
const RETRY_INTERVAL: Duration = Duration::from_millis(100);

/// Holds the daemon's lifetime lock file open. Dropping it closes the file descriptor,
/// which releases the `flock` — `flock` locks belong to the open file description, so
/// nothing else needs to run on drop.
#[derive(Debug)]
pub struct DaemonLock {
    // Never read, but must stay alive: dropping it closes the fd and releases the flock.
    _file: std::fs::File,
}

/// Tries to take an exclusive, non-blocking `flock` on `file`. `Ok(true)` means the lock
/// was taken; `Ok(false)` means somebody else holds it right now; `Err` is any other
/// failure opening or locking the file.
fn try_lock(file: &std::fs::File) -> io::Result<bool> {
    // SAFETY: `file` owns a live, open file descriptor for the duration of this call, and
    // `flock` has no other preconditions.
    let rc = unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) };
    if rc == 0 {
        return Ok(true);
    }
    let err = io::Error::last_os_error();
    if err.raw_os_error() == Some(libc::EWOULDBLOCK) {
        Ok(false)
    } else {
        Err(err)
    }
}

/// Tries, once, without blocking, to claim `data_dir`'s lock — and, unlike
/// [`DaemonLock::acquire`], never waits for it. Used by `ensure_daemon`
/// (`crates/tui/src/spawn.rs`) to decide, among several callers racing to start a
/// daemon, which one is responsible for actually spawning it.
///
/// `Ok(Some(lock))`: nobody held it, so *this* caller is the one that should spawn —
/// and must hold `lock` for as long as that decision needs to stay exclusive (through
/// the actual `spawn_detached` call), or a sibling caller checking a moment later would
/// see the lock free again and make the same "nobody's got this" decision itself. Drop
/// it once the detached child has been launched; the child re-acquires the same lock
/// for real, for its own whole lifetime, via [`DaemonLock::acquire`], and will simply
/// wait the short distance until this transient claim is dropped.
///
/// `Ok(None)`: somebody else holds it right now — a caller seeing that must not spawn
/// a redundant competitor of its own, only wait for whoever does.
///
/// `Err`: something other than contention went wrong (most likely: the directory does
/// not exist yet). This tells the caller nothing about contention, so it must not be
/// read as "someone else has this covered."
pub fn try_claim(data_dir: &Path) -> io::Result<Option<DaemonLock>> {
    let path = lock_path(data_dir);
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .mode(0o600)
        .open(&path)?;
    if try_lock(&file)? {
        Ok(Some(DaemonLock { _file: file }))
    } else {
        Ok(None)
    }
}

/// Reads the pid recorded in `<data_dir>/daemon.pid`, or `"unknown"` if it cannot be read.
///
/// `pub(crate)`, not private: `lifecycle::run`'s `Acquired::AlreadyRunning` arm needs
/// this same pid to build decision 24's exact refusal message itself (whole-branch-
/// review Major 4) — `acquire_or_yield` only builds it internally for its own deadline
/// bail below, not for a caller that already knows the goal is met some other way.
pub(crate) fn holder_pid(data_dir: &Path) -> String {
    std::fs::read_to_string(data_dir.join("daemon.pid"))
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "unknown".to_string())
}

/// What [`DaemonLock::acquire_or_yield`] found while waiting.
///
/// Fix wave 10, item 1: a caller blocked in the retry loop is not actually waiting to
/// *hold the lock* — it is waiting for its real goal, "a daemon exists", to become
/// true. Those two usually coincide (nobody else is running, so acquiring the lock is
/// how you become the daemon), but they can come apart: if somebody else's daemon is
/// already up while this caller is still contending for the lock, the goal is already
/// met and this caller must not go on to bind a second, unrequested daemon just
/// because the lock later happens to free up (e.g. because that other daemon stopped).
#[derive(Debug)]
pub enum Acquired {
    /// Nobody else was running; the lock is now held.
    Locked(DaemonLock),
    /// Gave up waiting for the lock because `already_running` reported that the goal
    /// this wait exists for is already satisfied. No lock is held.
    AlreadyRunning,
}

impl DaemonLock {
    /// Acquires the lock in `data_dir`, retrying every 100 ms until `wait` elapses.
    ///
    /// `data_dir` must already exist. On success the lock is held until the returned
    /// `DaemonLock` is dropped. On failure the error names the data directory and the pid
    /// of the daemon that appears to hold it.
    pub fn acquire(data_dir: &Path, wait: Duration) -> anyhow::Result<DaemonLock> {
        match Self::acquire_or_yield(data_dir, wait, || false)? {
            Acquired::Locked(lock) => Ok(lock),
            // The probe above never returns true, so this arm is unreachable: kept as
            // a match rather than an `if let` so a future change to `acquire_or_yield`
            // that could actually produce it here fails to compile silently ignoring
            // it, instead of just being missed.
            Acquired::AlreadyRunning => unreachable!(
                "acquire's own probe always returns false; AlreadyRunning cannot happen"
            ),
        }
    }

    /// Injectable twin of [`acquire`] (following `project::detect_roots_with`'s
    /// convention): on every failed lock attempt, also asks `already_running` whether
    /// the thing this wait exists to produce has already shown up some other way.
    ///
    /// This is where the layering choice for fix wave 10, item 1 lives. `already_running`
    /// is an opaque predicate — this module never learns what it checks (a socket, a
    /// pid file, anything else) — so `lockfile.rs` stays a plain lock primitive with no
    /// knowledge of daemons or sockets; the one caller that actually knows what "a
    /// daemon exists" means (`lifecycle::run`, which already owns the socket path) is
    /// the one that supplies the check. The alternative — reimplementing this same
    /// retry-and-deadline loop in the caller so it can poll the socket itself between
    /// lock attempts — would duplicate `RETRY_INTERVAL`/deadline bookkeeping that
    /// already lives here for exactly this purpose. Threading a closure through keeps
    /// the loop in one place without teaching it what it is polling for.
    ///
    /// `already_running` is checked immediately after every *failed* `try_lock` —
    /// before sleeping, before checking the deadline — so a caller contending against
    /// an already-live daemon notices on its very first attempt, long before any real
    /// timing race against that daemon later stopping could even begin.
    pub fn acquire_or_yield(
        data_dir: &Path,
        wait: Duration,
        mut already_running: impl FnMut() -> bool,
    ) -> anyhow::Result<Acquired> {
        let path = lock_path(data_dir);
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .mode(0o600)
            .open(&path)?;
        let deadline = Instant::now() + wait;
        loop {
            if try_lock(&file)? {
                return Ok(Acquired::Locked(DaemonLock { _file: file }));
            }
            if already_running() {
                return Ok(Acquired::AlreadyRunning);
            }
            if Instant::now() >= deadline {
                anyhow::bail!(
                    "another anthrex daemon is running with data directory {} (pid {})",
                    data_dir.display(),
                    holder_pid(data_dir)
                );
            }
            std::thread::sleep(RETRY_INTERVAL);
        }
    }
}

fn lock_path(data_dir: &Path) -> PathBuf {
    data_dir.join("daemon.lock")
}

/// Blocks until `lock_path` is unlocked, or `timeout` elapses. Returns whether it was
/// released. Tries `LOCK_EX | LOCK_NB` on a fresh descriptor every 100 ms, releasing at
/// once on success so it never itself holds the lock the caller is waiting to see freed.
///
/// A missing lock file counts as released: nothing is holding it.
pub fn wait_released(lock_path: &Path, timeout: Duration) -> bool {
    let deadline = Instant::now() + timeout;
    loop {
        if is_released(lock_path) {
            return true;
        }
        if Instant::now() >= deadline {
            return false;
        }
        std::thread::sleep(RETRY_INTERVAL);
    }
}

fn is_released(lock_path: &Path) -> bool {
    let file = match OpenOptions::new().read(true).write(true).open(lock_path) {
        Ok(f) => f,
        Err(_) => return true,
    };
    match try_lock(&file) {
        Ok(true) => {
            // SAFETY: `file` is a live, open file descriptor for the duration of this call.
            unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_UN) };
            true
        }
        Ok(false) | Err(_) => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn second_acquire_fails_while_held() {
        let dir = tempfile::tempdir().unwrap();
        let first = DaemonLock::acquire(dir.path(), Duration::ZERO).unwrap();
        let err = DaemonLock::acquire(dir.path(), Duration::ZERO)
            .unwrap_err()
            .to_string();
        assert!(err.contains("another anthrex daemon"), "{err}");
        drop(first);
        // This used to end with a third `acquire(dir.path(), Duration::from_secs(2))` to
        // prove a release unblocks a following acquire "at once" (a `Duration::ZERO`
        // attempt was intermittently losing that race). The real mechanism, established
        // experimentally (M6 fix wave 1, task-2 review finding 3): `fork()` duplicates a
        // process's *entire* fd table regardless of `O_CLOEXEC`, which only takes effect
        // at the child's own later `exec()`. This crate's `--lib` binary runs this test
        // alongside `portable-pty`-spawning tests on other threads of the same process, so
        // a sibling test's `fork()` can transiently duplicate this test's own just-closed
        // lock fd into a child that hasn't `exec()`'d yet, keeping the flock held past
        // `first`'s `drop()` above — not scheduling jitter, and not something widening the
        // wait here actually tests, since it made this assertion redundant with
        // `acquire_waits_for_release` below (same property, looser bound). The zero-wait,
        // instant-reacquire assertion now lives in `crates/daemon/tests/lockfile.rs`,
        // which is its own test binary with no forking siblings in its process.
    }

    #[test]
    fn try_claim_holds_the_lock_until_dropped() {
        let dir = tempfile::tempdir().unwrap();
        let claimed = try_claim(dir.path())
            .unwrap()
            .expect("nobody holds it yet, so this must claim it");
        // Unlike a claim-then-release probe, this must still be held: a second claim
        // attempt while the first is alive must see contention, not "free."
        assert!(
            try_claim(dir.path()).unwrap().is_none(),
            "the first claim must still be holding the lock"
        );
        drop(claimed);
        // Once actually dropped, the lock is free again with no wait at all.
        let _lock = DaemonLock::acquire(dir.path(), Duration::ZERO)
            .expect("dropping the claimed lock must release it");
    }

    #[test]
    fn try_claim_reports_contention_without_erroring() {
        let dir = tempfile::tempdir().unwrap();
        let _held = DaemonLock::acquire(dir.path(), Duration::ZERO).unwrap();
        assert!(
            try_claim(dir.path()).unwrap().is_none(),
            "somebody else holds it right now"
        );
    }

    #[test]
    fn try_claim_errors_on_a_missing_directory_rather_than_claiming() {
        let dir = tempfile::tempdir().unwrap();
        let missing = dir.path().join("does-not-exist");
        assert!(
            try_claim(&missing).is_err(),
            "a missing directory is a real I/O problem, not \"uncontended\""
        );
    }

    #[test]
    fn acquire_waits_for_release() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().to_path_buf();
        let held = DaemonLock::acquire(&path, Duration::ZERO).unwrap();
        let releaser = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(300));
            drop(held);
        });
        let started = Instant::now();
        let _second = DaemonLock::acquire(&path, Duration::from_secs(2)).unwrap();
        assert!(
            started.elapsed() < Duration::from_millis(1500),
            "{:?}",
            started.elapsed()
        );
        releaser.join().unwrap();
    }

    /// Fix wave 10, item 1's core contract, pinned deterministically (no sleeps, no
    /// threads, no timing luck): with the lock already held by someone else, a probe
    /// that reports "already running" must make `acquire_or_yield` yield instead of
    /// waiting for the lock — even when `wait` is enormous, and even though the lock
    /// would in fact still be free for the taking the moment the holder drops it.
    /// `acquire_or_yield` runs entirely on this thread with no `.await`, so there is no
    /// concurrency to arrange: the held lock alone is enough to force the first
    /// `try_lock` to fail, and the probe result decides everything from there.
    #[test]
    fn acquire_or_yield_yields_on_the_first_contended_attempt_when_already_running() {
        let dir = tempfile::tempdir().unwrap();
        let _held = DaemonLock::acquire(dir.path(), Duration::ZERO).unwrap();

        let calls = std::cell::Cell::new(0u32);
        let result = DaemonLock::acquire_or_yield(dir.path(), Duration::from_secs(3600), || {
            calls.set(calls.get() + 1);
            true
        });

        match result {
            Ok(Acquired::AlreadyRunning) => {}
            other => panic!("expected AlreadyRunning, got {other:?}"),
        }
        assert_eq!(
            calls.get(),
            1,
            "must consult the probe on the very first contended attempt, not spin \
             first and risk racing the holder's own release"
        );
    }

    /// The probe is re-consulted on every contended attempt, not just the first — a
    /// caller that only becomes reachable partway through the wait must still be
    /// noticed before the deadline, not just at the start.
    #[test]
    fn acquire_or_yield_notices_already_running_appearing_mid_wait() {
        let dir = tempfile::tempdir().unwrap();
        let _held = DaemonLock::acquire(dir.path(), Duration::ZERO).unwrap();

        let calls = std::cell::Cell::new(0u32);
        let result = DaemonLock::acquire_or_yield(dir.path(), Duration::from_secs(3600), || {
            let n = calls.get() + 1;
            calls.set(n);
            n >= 3
        });

        assert!(matches!(result, Ok(Acquired::AlreadyRunning)), "{result:?}");
        assert_eq!(calls.get(), 3);
    }

    /// When nothing is ever "already running", a released lock is still acquired
    /// normally — the new probe must not change the ordinary, uncontended-eventually
    /// outcome, only add an early exit for the case it exists for.
    #[test]
    fn acquire_or_yield_still_acquires_the_lock_when_never_already_running() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().to_path_buf();
        let held = DaemonLock::acquire(&path, Duration::ZERO).unwrap();
        let releaser = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(300));
            drop(held);
        });

        let started = Instant::now();
        let result = DaemonLock::acquire_or_yield(&path, Duration::from_secs(2), || false).unwrap();
        assert!(matches!(result, Acquired::Locked(_)), "{result:?}");
        assert!(
            started.elapsed() < Duration::from_millis(1500),
            "{:?}",
            started.elapsed()
        );
        releaser.join().unwrap();
    }

    #[test]
    fn wait_released_reports_both_outcomes() {
        let dir = tempfile::tempdir().unwrap();
        let held = DaemonLock::acquire(dir.path(), Duration::ZERO).unwrap();
        let lock_file = lock_path(dir.path());
        assert!(!wait_released(&lock_file, Duration::from_millis(300)));
        drop(held);
        assert!(wait_released(&lock_file, Duration::from_secs(2)));
    }
}
