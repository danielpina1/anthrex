//! Size-based rotation for `<data_dir>/daemon.log`. Design decision 28: rotation runs
//! synchronously inside [`std::io::Write::write`], on whatever thread calls it —
//! `tracing_appender::non_blocking` wraps this type and funnels every write through its
//! own single dedicated thread, so in production there is exactly one writer and rotation
//! never races a concurrent write to the *same* `RotatingFile`. The type still has to be
//! safe if that assumption is ever wrong (a `Mutex<RotatingFile>` shared by more than one
//! caller, say), which is what `rotation_is_size_safe_under_concurrent_writers_behind_a_mutex`
//! below pins down: every write is atomic under the caller's own lock, so nothing here can
//! tear a line or let a file grow past its bound no matter how the caller serializes calls.
//!
//! A `rotate()` failure (a read-only directory, a full disk, ...) never fails the write
//! that triggered it and never wedges logging shut for good: see
//! `RotatingFile::recover_from_failed_rotation` and `rotation_failure_does_not_wedge_logging_shut`
//! below. Exceeding the size cap once is the recoverable direction; losing every write from
//! then on, silently, is not.

use std::fs::{File, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};

/// The production size limit: rotate before a write would take `daemon.log` past this.
pub const LOG_MAX_BYTES: u64 = 10 * 1024 * 1024;

/// The production file count, current file included: `daemon.log`, `.1`, `.2`, `.3`, `.4`.
pub const LOG_KEEP: usize = 5;

/// A `daemon.log` that rotates itself by size. See the module doc for the concurrency
/// contract.
pub struct RotatingFile {
    dir: PathBuf,
    name: String,
    max_bytes: u64,
    keep: usize,
    file: File,
    len: u64,
    /// Latches once [`Self::recover_from_failed_rotation`] has reported the current
    /// failure episode, and clears the moment a `rotate()` call succeeds. Without this,
    /// every write past the cap re-attempts rotation, fails identically, and re-emits the
    /// diagnostic — one line per write for as long as the directory stays broken, not one
    /// per failure episode. See `rotation_failure_diagnostic_is_reported_once_per_episode`.
    rotation_failure_reported: bool,
}

impl RotatingFile {
    /// Opens (creating if needed) `<dir>/<name>` for appending, tracking its current
    /// length so the very next write already knows whether it needs to rotate first.
    ///
    /// `keep` counts the current file itself, so `keep - 1` rotated files (`.1` through
    /// `.{keep-1}`) can exist alongside it — `keep = 5` is decision 28's "at most `LOG_KEEP`
    /// = 5 files".
    pub fn open(dir: &Path, name: &str, max_bytes: u64, keep: usize) -> io::Result<Self> {
        std::fs::create_dir_all(dir)?;
        let path = dir.join(name);
        let file = OpenOptions::new().create(true).append(true).open(&path)?;
        let len = file.metadata()?.len();
        Ok(Self {
            dir: dir.to_path_buf(),
            name: name.to_string(),
            max_bytes,
            keep,
            file,
            len,
            rotation_failure_reported: false,
        })
    }

    /// Rotates now if the file is already over the cap, with nothing new to write.
    ///
    /// [`Write::write`] below only ever checks the cap when it has bytes of its own to
    /// add; a caller that opens a `RotatingFile` purely to enforce decision 28's bound at
    /// *open* time — `anthrex`'s CLI parent, capping `daemon.stderr.log` before handing
    /// the file to a detached child as its raw stderr fd (`crates/cli/src/spawn.rs`),
    /// where nothing afterwards funnels through this `Write` impl at all — needs a way to
    /// ask "is this already too big?" without writing anything. This is that: the same
    /// rotate-or-recover logic `write` uses, with zero incoming bytes.
    pub fn rotate_if_over_cap(&mut self) {
        self.rotate_if_needed(0);
    }

    /// Rotates if `len` bytes already on top of `incoming` new ones would exceed the cap.
    /// Shared by [`Write::write`] (`incoming` = the buffer it is about to write) and
    /// [`Self::rotate_if_over_cap`] (`incoming` = 0, so this only fires when the file was
    /// already over the cap before this call).
    fn rotate_if_needed(&mut self, incoming: u64) {
        // Decision 28: rotate *before* a write that would take the file past the limit,
        // never after. A file already at or under the limit that receives a write which
        // lands it exactly on the limit does not rotate — only a write that would go
        // strictly past does.
        if self.len > 0 && self.len.saturating_add(incoming) > self.max_bytes {
            // A rotation failure must not turn into losing every write for the rest of
            // the process's life (task-3 review, Major finding): `self.len` used to be
            // reset only on a *successful* rotation, so a failure here left it stuck
            // above `max_bytes` forever, and every later write re-tried rotation, failed
            // the same way, and was lost too — even long after whatever broke the
            // directory was fixed, since nothing ever cleared the stuck `len`. Losing all
            // future logging is worse than exceeding the cap once, so a failure here does
            // not propagate: it recovers the writer (below) and falls through to the
            // write itself.
            match self.rotate() {
                Ok(()) => self.rotation_failure_reported = false,
                Err(e) => self.recover_from_failed_rotation(&e),
            }
        }
    }

    fn path(&self) -> PathBuf {
        self.dir.join(&self.name)
    }

    fn rotated_path(&self, n: usize) -> PathBuf {
        self.dir.join(format!("{}.{n}", self.name))
    }

    /// Deletes the oldest rotated file, shifts every other rotated file up by one suffix,
    /// moves the current file to `.1`, and opens a fresh, empty current file.
    ///
    /// Decision 28, spelled out for `keep = 5`: delete `.4`, rename `.3` to `.4`, `.2` to
    /// `.3`, `.1` to `.2`, then the current file to `.1`. A rename source that does not
    /// exist yet (the log has not rotated `keep` times yet) is skipped rather than treated
    /// as an error.
    ///
    /// The two branches handle a removal failure differently on purpose, not by oversight:
    /// deleting `oldest` (the `keep >= 2` branch) is cleanup of a file that may simply not
    /// exist yet, if the log hasn't rotated `keep` times before — an error there is
    /// expected and ignored. Deleting `self.path()` in the `keep < 2` branch instead *is*
    /// the rotation (there is nowhere else for the content to go), so its error is the
    /// caller's business and propagated with `?`, same as the renames above it.
    fn rotate(&mut self) -> io::Result<()> {
        if self.keep >= 2 {
            let oldest = self.rotated_path(self.keep - 1);
            let _ = std::fs::remove_file(&oldest);
            for i in (1..=self.keep.saturating_sub(2)).rev() {
                let from = self.rotated_path(i);
                if from.exists() {
                    std::fs::rename(&from, self.rotated_path(i + 1))?;
                }
            }
            std::fs::rename(self.path(), self.rotated_path(1))?;
        } else {
            // keep <= 1: there is nowhere to rotate to, so the only option that keeps the
            // "at most `keep` files" property is to drop the old content.
            std::fs::remove_file(self.path())?;
        }
        self.file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(self.path())?;
        self.len = 0;
        Ok(())
    }

    /// Recovers `self.file`/`self.len` after `rotate()` fails (a read-only directory, a
    /// full disk, ...) so the caller can still write the bytes that triggered the failed
    /// rotation instead of losing them — see [`Write::write`] below for why that matters.
    ///
    /// `rotate()` can fail partway: e.g. it may have successfully renamed the current file
    /// to `.1` before failing to recreate a fresh current file (`ENOSPC`, or the same
    /// permission problem). Reopening `self.path()` recovers that case too, and does not
    /// need write permission on the directory when the file already exists — only
    /// *creating* a new directory entry does — so it succeeds even in the common failure
    /// (an unwritable directory) where `rotate()` itself could not rename or remove
    /// anything. If that reopen also fails, `self.file` is left as whatever it was, which
    /// in the ordinary case (nothing renamed it away) is still the original, perfectly
    /// writable file. Either way, `self.len` is re-derived from real on-disk length rather
    /// than trusted, since it may now be describing a file that moved out from under it.
    ///
    /// The diagnostic below is latched (`rotation_failure_reported`), not printed on every
    /// call: `tracing_appender`'s non-blocking worker thread silently drops `Write` errors
    /// (its own source has only a `// TODO: print to stderr`), so this is the only place
    /// on the path from a daemon log call to disk that can ever surface a rotation failure
    /// at all — but `self.len` stays above the cap for as long as the directory does, so
    /// every write in that span re-enters here. Reporting once per failure *episode*
    /// (latched here, cleared in `rotate_if_needed` the moment a rotation next succeeds)
    /// is what keeps the diagnostic's own destination, `daemon.stderr.log`
    /// (`crates/cli/src/spawn.rs`), bounded on its own; the cap
    /// `RotatingFile::rotate_if_over_cap` gives it at open time is only a backstop for
    /// growth this latch cannot see (many separate failure/recovery episodes over a long
    /// run, or an unrelated panic).
    fn recover_from_failed_rotation(&mut self, err: &io::Error) {
        if !self.rotation_failure_reported {
            // A direct write to the real `io::stderr()` handle, not `eprintln!`: the
            // macro checks a thread-local capture override before it ever reaches the
            // real stream (that override is exactly what the default `cargo test`
            // harness installs per-test, and production has nothing like it, so the two
            // behave identically outside of tests), and this message must land on the
            // one thing production actually redirects — the real fd — for the detached-
            // daemon path this exists for to work at all.
            let message = format!(
                "anthrex daemon: failed to rotate {}: {err}; continuing to log to the existing file past its size limit\n",
                self.path().display()
            );
            let _ = io::stderr().write_all(message.as_bytes());
            self.rotation_failure_reported = true;
        }
        if let Ok(reopened) = OpenOptions::new()
            .create(true)
            .append(true)
            .open(self.path())
        {
            self.file = reopened;
        }
        self.len = self.file.metadata().map(|m| m.len()).unwrap_or(self.len);
    }
}

impl Write for RotatingFile {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let incoming = buf.len() as u64;
        // A buffer larger than `max_bytes` on its own is not split: it still rotates the
        // (nonempty) file it would overflow, then goes in whole.
        self.rotate_if_needed(incoming);
        self.file.write_all(buf)?;
        self.len += incoming;
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        self.file.flush()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `recover_from_failed_rotation`'s diagnostic writes directly to the real
    /// process-wide stderr fd (2), deliberately not through `eprintln!` — the macro
    /// checks `cargo test`'s per-test output-capture override before it ever reaches the
    /// real stream, which would make `capture_stderr` below observe nothing no matter how
    /// many times the diagnostic fires (confirmed by experiment: an `eprintln!` in this
    /// position never appears in the fd this redirects, only a direct
    /// `io::stderr().write_all()` does). `capture_stderr` redirects that real fd for the
    /// duration of a closure so a test can assert on what was actually written, not just
    /// on internal state. Because fd 2 is process-global and `cargo test` runs this
    /// binary's unit tests on multiple threads, every test that triggers this diagnostic
    /// must hold this lock for as long as it might write, or two such tests running
    /// concurrently could interleave into each other's capture.
    static STDERR_CAPTURE_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    /// Redirects the real stderr fd to a temp file for the duration of `f`, then restores
    /// it and returns what was written, alongside `f`'s own result. Caller must hold
    /// [`STDERR_CAPTURE_LOCK`].
    fn capture_stderr<T>(f: impl FnOnce() -> T) -> (T, String) {
        use std::io::{Read, Seek, SeekFrom};
        use std::os::unix::io::AsRawFd;

        let mut tmp = tempfile::tempfile().unwrap();
        // SAFETY: `dup`/`dup2` on valid, open fds (2, and `tmp`'s own) cannot fail short
        // of running out of file descriptors; the saved fd is closed again below before
        // returning, so nothing leaks past this function.
        let saved_stderr = unsafe { libc::dup(2) };
        assert!(saved_stderr >= 0, "failed to save stderr fd");
        let rc = unsafe { libc::dup2(tmp.as_raw_fd(), 2) };
        assert!(rc >= 0, "failed to redirect stderr to the capture file");

        let result = f();

        // SAFETY: restores fd 2 to what it was before, then releases the saved copy.
        unsafe {
            libc::dup2(saved_stderr, 2);
            libc::close(saved_stderr);
        }

        tmp.seek(SeekFrom::Start(0)).unwrap();
        let mut captured = String::new();
        tmp.read_to_string(&mut captured).unwrap();
        (result, captured)
    }

    /// Writes 40 lines of exactly 100 bytes each with a 1024-byte limit and a 3-file
    /// keep: each rotation completes exactly 10 lines (1000 bytes) before the 11th would
    /// push it to 1100, so the last two rotations' worth of history survives (`.1`, `.2`)
    /// and the oldest is gone (`.3` absent) — `keep = 3` means the current file plus two
    /// rotated ones.
    #[test]
    fn rotates_by_size_and_keeps_n_files() {
        let dir = tempfile::tempdir().unwrap();
        let mut file = RotatingFile::open(dir.path(), "daemon.log", 1024, 3).unwrap();
        let mut last_line = String::new();
        for n in 0..40 {
            let head = format!("line-{n:02}");
            let line = format!("{head:<99}\n");
            assert_eq!(line.len(), 100);
            file.write_all(line.as_bytes()).unwrap();
            last_line = line;
        }

        let log = dir.path().join("daemon.log");
        let l1 = dir.path().join("daemon.log.1");
        let l2 = dir.path().join("daemon.log.2");
        let l3 = dir.path().join("daemon.log.3");
        assert!(log.exists());
        assert!(l1.exists());
        assert!(l2.exists());
        assert!(!l3.exists(), "keep = 3 must not retain a fourth file");

        for path in [&log, &l1, &l2] {
            let len = std::fs::metadata(path).unwrap().len();
            assert!(len <= 1024, "{path:?} is {len} bytes, over the limit");
            assert_eq!(
                len % 100,
                0,
                "{path:?} is {len} bytes: a 100-byte line was split across files"
            );
        }
        let contents = std::fs::read_to_string(&log).unwrap();
        assert!(
            contents.ends_with(&last_line),
            "the last line written must be the last line in the live file: {contents:?}"
        );
    }

    /// A pre-existing `daemon.log` a previous daemon left behind must be appended to, not
    /// clobbered, and its length must be tracked from the start so the very first write on
    /// top of it can already trigger rotation correctly.
    #[test]
    fn appends_to_an_existing_log() {
        let dir = tempfile::tempdir().unwrap();
        let existing = vec![b'x'; 1000];
        std::fs::write(dir.path().join("daemon.log"), &existing).unwrap();

        let mut file = RotatingFile::open(dir.path(), "daemon.log", 1024, 3).unwrap();
        file.write_all(&[b'y'; 100]).unwrap();

        let rotated = std::fs::read(dir.path().join("daemon.log.1")).unwrap();
        assert_eq!(
            rotated, existing,
            "the rotated file must hold the original bytes"
        );
        let current = std::fs::read(dir.path().join("daemon.log")).unwrap();
        assert_eq!(current, vec![b'y'; 100]);
    }

    /// A single write bigger than the whole limit is not split or refused: it lands whole,
    /// in a fresh file, because there is nothing else a `Write` impl can correctly do with
    /// a caller-supplied buffer it does not own the framing of.
    #[test]
    fn an_oversized_write_goes_whole_into_a_fresh_file() {
        let dir = tempfile::tempdir().unwrap();
        let mut file = RotatingFile::open(dir.path(), "daemon.log", 1024, 3).unwrap();
        let big = vec![b'z'; 5000];
        file.write_all(&big).unwrap();

        assert!(!dir.path().join("daemon.log.1").exists());
        let contents = std::fs::read(dir.path().join("daemon.log")).unwrap();
        assert_eq!(contents, big);
    }

    /// The same oversized-write rule applies when the current file is not already empty:
    /// it rotates the existing content out first (because leaving it would silently exceed
    /// the limit), then the oversized write still goes in whole, unsplit, on top of the
    /// fresh file.
    #[test]
    fn an_oversized_write_after_existing_content_rotates_then_lands_whole() {
        let dir = tempfile::tempdir().unwrap();
        let mut file = RotatingFile::open(dir.path(), "daemon.log", 1024, 3).unwrap();
        file.write_all(&[b'a'; 200]).unwrap();
        let big = vec![b'z'; 5000];
        file.write_all(&big).unwrap();

        let rotated = std::fs::read(dir.path().join("daemon.log.1")).unwrap();
        assert_eq!(rotated, vec![b'a'; 200]);
        let current = std::fs::read(dir.path().join("daemon.log")).unwrap();
        assert_eq!(current, big);
    }

    /// Pins the three boundary values decision 28 turns on: a write landing one byte under
    /// the limit must not rotate, one landing exactly on the limit must not rotate either
    /// (only a write that would go *past* it rotates), and one landing one byte over must
    /// rotate first. Checked with exact byte content, not just file existence, so a
    /// mutation that rotates one write too early or late is caught even though it would
    /// still leave every file present.
    #[test]
    fn boundary_writes_rotate_only_strictly_past_the_limit() {
        let dir = tempfile::tempdir().unwrap();
        let mut file = RotatingFile::open(dir.path(), "daemon.log", 100, 3).unwrap();
        let log = dir.path().join("daemon.log");
        let rotated = dir.path().join("daemon.log.1");

        file.write_all(&[b'a'; 60]).unwrap();
        file.write_all(&[b'b'; 39]).unwrap(); // len = 99: one byte under the limit.
        assert!(!rotated.exists());
        assert_eq!(std::fs::metadata(&log).unwrap().len(), 99);

        file.write_all(b"c").unwrap(); // len = 100: exactly on the limit.
        assert!(
            !rotated.exists(),
            "a write landing exactly on the limit must not rotate"
        );
        assert_eq!(std::fs::metadata(&log).unwrap().len(), 100);

        file.write_all(b"d").unwrap(); // len would be 101: one byte over.
        assert!(
            rotated.exists(),
            "a write one byte past the limit must rotate"
        );
        let rotated_bytes = std::fs::read(&rotated).unwrap();
        assert_eq!(rotated_bytes.len(), 100);
        assert_eq!(*rotated_bytes.last().unwrap(), b'c');
        let current = std::fs::read(&log).unwrap();
        assert_eq!(current, vec![b'd']);
    }

    /// `RotatingFile` is only ever driven by `tracing_appender`'s single non-blocking
    /// writer thread in production (see the module doc), but the type must not corrupt
    /// data if it is ever shared behind a lock: every file must stay within its bound and
    /// no line may end up torn across two files, no matter how many threads' writes
    /// interleave through the mutex.
    #[test]
    fn rotation_is_size_safe_under_concurrent_writers_behind_a_mutex() {
        use std::sync::{Arc, Mutex};

        let dir = tempfile::tempdir().unwrap();
        let file = RotatingFile::open(dir.path(), "daemon.log", 500, 4).unwrap();
        let shared = Arc::new(Mutex::new(file));
        const LINE_LEN: usize = 20;

        let threads: Vec<_> = (0..8)
            .map(|t| {
                let shared = Arc::clone(&shared);
                std::thread::spawn(move || {
                    for n in 0..50u32 {
                        let mut content = format!("t{t}-l{n:04}");
                        content.truncate(LINE_LEN - 1);
                        while content.len() < LINE_LEN - 1 {
                            content.push('.');
                        }
                        let mut line = content;
                        line.push('\n');
                        assert_eq!(line.len(), LINE_LEN);
                        shared.lock().unwrap().write_all(line.as_bytes()).unwrap();
                    }
                })
            })
            .collect();
        for t in threads {
            t.join().unwrap();
        }

        for name in ["daemon.log", "daemon.log.1", "daemon.log.2", "daemon.log.3"] {
            let path = dir.path().join(name);
            if !path.exists() {
                continue;
            }
            let len = std::fs::metadata(&path).unwrap().len();
            assert!(len <= 500, "{name} is {len} bytes, over the limit");
            assert_eq!(
                len % LINE_LEN as u64,
                0,
                "{name} is {len} bytes: not a whole number of lines, so one was torn"
            );
        }
        assert!(
            !dir.path().join("daemon.log.4").exists(),
            "keep = 4 must not retain a fifth file"
        );
    }

    /// Task-3 review, Major finding: a rotation failure used to leave `self.len` stuck
    /// above the cap forever, because it was only ever reset *after* a successful
    /// rotation. So the write that triggered the failed rotation was lost, and — because
    /// `len` was never reset — every write after it re-evaluated the same "would exceed
    /// the cap" condition, retried rotation, failed the same way, and was lost too. Not
    /// just for as long as the directory stayed unwritable: forever, since restoring
    /// permissions did nothing to un-stick `len`.
    ///
    /// The ruling: losing all future logging is worse than exceeding the size cap once.
    /// A failed rotation must leave the writer usable — the triggering write (and every
    /// write after it, until the directory is writable again) must still land, growing
    /// past the cap if it has to — and rotation must resume on its own once the directory
    /// is writable again, with no restart needed.
    #[test]
    fn rotation_failure_does_not_wedge_logging_shut() {
        use std::os::unix::fs::PermissionsExt;

        // This test's writes go through `recover_from_failed_rotation`, which prints to
        // the real stderr fd — see `STDERR_CAPTURE_LOCK`'s doc comment for why every such
        // test must hold this lock for the duration.
        let _stderr_guard = STDERR_CAPTURE_LOCK.lock().unwrap();

        let dir = tempfile::tempdir().unwrap();
        let mut file = RotatingFile::open(dir.path(), "daemon.log", 100, 3).unwrap();
        file.write_all(&[b'a'; 99]).unwrap();

        let mode_before = std::fs::metadata(dir.path()).unwrap().permissions().mode();
        std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o500)).unwrap();

        // Restore the directory's permissions even if an assertion below panics, so this
        // test can never leave a read-only temp directory behind to wedge the rest of the
        // suite (tempfile's own cleanup needs to remove it).
        struct RestorePerms(PathBuf, u32);
        impl Drop for RestorePerms {
            fn drop(&mut self) {
                let _ = std::fs::set_permissions(&self.0, std::fs::Permissions::from_mode(self.1));
            }
        }
        let _restore = RestorePerms(dir.path().to_path_buf(), mode_before);

        // 99 + 5 > 100: this write must try to rotate. The directory denies the
        // rename/remove rotation needs, so rotation itself fails — but the write must
        // still land instead of being lost.
        file.write_all(&[b'b'; 5])
            .expect("a rotation failure must not fail the triggering write");
        let current = std::fs::read(dir.path().join("daemon.log")).unwrap();
        assert!(
            current.ends_with(&[b'b'; 5]),
            "the triggering write must still land in the file"
        );
        assert!(
            current.len() > 100,
            "failing to rotate must grow past the cap rather than lose data; got {} bytes",
            current.len()
        );

        // A second write, still under the read-only directory, must also still land —
        // proving this isn't a one-shot recovery that then wedges shut again.
        file.write_all(&[b'c'; 5]).unwrap();
        let current = std::fs::read(dir.path().join("daemon.log")).unwrap();
        assert!(
            current.ends_with(&[b'c'; 5]),
            "logging must keep working on every write while the directory stays unwritable"
        );

        // Once the directory is writable again, the very next threshold crossing must
        // rotate successfully with no restart needed.
        std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(mode_before)).unwrap();
        file.write_all(&[b'd'; 200]).unwrap();
        assert!(
            dir.path().join("daemon.log.1").exists(),
            "rotation must resume on its own once the directory is writable again"
        );
    }

    /// Re-review Major #1: `recover_from_failed_rotation`'s diagnostic used to print on
    /// every write past the cap, not once per failure episode — `self.len` stays above
    /// `max_bytes` for as long as the directory does, so every write re-entered the same
    /// "rotation failed" branch and re-printed. Measured directly here, not inferred: N
    /// writes under a persistently unwritable directory must produce exactly one line on
    /// stderr, and a fresh failure *after* a rotation has since succeeded must be reported
    /// again — the latch tracks episodes, not "has this ever failed".
    #[test]
    fn rotation_failure_diagnostic_is_reported_once_per_episode() {
        use std::os::unix::fs::PermissionsExt;

        let _stderr_guard = STDERR_CAPTURE_LOCK.lock().unwrap();

        let dir = tempfile::tempdir().unwrap();
        let mut file = RotatingFile::open(dir.path(), "daemon.log", 100, 3).unwrap();
        file.write_all(&[b'a'; 99]).unwrap();

        let mode_before = std::fs::metadata(dir.path()).unwrap().permissions().mode();
        std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o500)).unwrap();
        struct RestorePerms(PathBuf, u32);
        impl Drop for RestorePerms {
            fn drop(&mut self) {
                let _ = std::fs::set_permissions(&self.0, std::fs::Permissions::from_mode(self.1));
            }
        }
        let _restore = RestorePerms(dir.path().to_path_buf(), mode_before);

        // Five writes, every one of them past the cap, under a directory that stays
        // unwritable the whole time: one failure episode, so exactly one line.
        let (_, captured) = capture_stderr(|| {
            for _ in 0..5 {
                file.write_all(&[b'b'; 5]).unwrap();
            }
        });
        let lines = captured
            .lines()
            .filter(|l| l.contains("failed to rotate"))
            .count();
        assert_eq!(
            lines, 1,
            "5 writes under a persistently failing rotation must produce exactly one \
             diagnostic line, not one per write: {captured:?}"
        );

        // Let a rotation succeed: the episode ends.
        std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(mode_before)).unwrap();
        file.write_all(&[b'c'; 200]).unwrap();
        assert!(
            dir.path().join("daemon.log.1").exists(),
            "the successful rotation that ends the episode must actually happen"
        );

        // Break the directory again: this is a *new* episode and must be reported again.
        std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o500)).unwrap();
        let (_, captured2) = capture_stderr(|| {
            file.write_all(&[b'd'; 5]).unwrap();
        });
        let lines2 = captured2
            .lines()
            .filter(|l| l.contains("failed to rotate"))
            .count();
        assert_eq!(
            lines2, 1,
            "a new failure episode after a successful rotation must be reported again: {captured2:?}"
        );
    }

    /// `rotate_if_over_cap` is `open_stderr_sink`'s hook (`crates/cli/src/spawn.rs`) for
    /// bounding `daemon.stderr.log` at open time, since nothing after that point writes
    /// to it through a `Write` call this crate controls. Pinned here at the `RotatingFile`
    /// level: a file already over the cap, with nothing new to write, must still rotate.
    #[test]
    fn rotate_if_over_cap_rotates_an_already_oversized_file_with_nothing_to_write() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("daemon.log"), vec![b'x'; 500]).unwrap();

        let mut file = RotatingFile::open(dir.path(), "daemon.log", 100, 3).unwrap();
        file.rotate_if_over_cap();

        assert!(
            dir.path().join("daemon.log.1").exists(),
            "an already-oversized file must be rotated away, not left in place"
        );
        let current_len = std::fs::metadata(dir.path().join("daemon.log"))
            .unwrap()
            .len();
        assert_eq!(
            current_len, 0,
            "the fresh current file after an over-the-cap rotation must start empty"
        );

        // A file at or under the cap must not be touched — this is a cap check, not an
        // unconditional rotation.
        let dir2 = tempfile::tempdir().unwrap();
        std::fs::write(dir2.path().join("daemon.log"), vec![b'x'; 50]).unwrap();
        let mut file2 = RotatingFile::open(dir2.path(), "daemon.log", 100, 3).unwrap();
        file2.rotate_if_over_cap();
        assert!(
            !dir2.path().join("daemon.log.1").exists(),
            "a file under the cap must not be rotated"
        );
    }
}
