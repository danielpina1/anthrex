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

/// Reads the pid recorded in `<data_dir>/daemon.pid`, or `"unknown"` if it cannot be read.
fn holder_pid(data_dir: &Path) -> String {
    std::fs::read_to_string(data_dir.join("daemon.pid"))
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "unknown".to_string())
}

impl DaemonLock {
    /// Acquires the lock in `data_dir`, retrying every 100 ms until `wait` elapses.
    ///
    /// `data_dir` must already exist. On success the lock is held until the returned
    /// `DaemonLock` is dropped. On failure the error names the data directory and the pid
    /// of the daemon that appears to hold it.
    pub fn acquire(data_dir: &Path, wait: Duration) -> anyhow::Result<DaemonLock> {
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
                return Ok(DaemonLock { _file: file });
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
