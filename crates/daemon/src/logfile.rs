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
        })
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
    fn recover_from_failed_rotation(&mut self, err: &io::Error) {
        // `tracing_appender`'s non-blocking worker thread silently drops `Write` errors
        // (its own source has only a `// TODO: print to stderr`), so this is the only
        // place on the path from a daemon log call to disk that can ever surface a
        // rotation failure at all.
        eprintln!(
            "anthrex daemon: failed to rotate {}: {err}; continuing to log to the existing file past its size limit",
            self.path().display()
        );
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
        // Decision 28: rotate *before* a write that would take the file past the limit,
        // never after. A file already at or under the limit that receives a write which
        // lands it exactly on the limit does not rotate — only a write that would push it
        // strictly past does. A buffer larger than `max_bytes` on its own is not split: it
        // still rotates the (nonempty) file it would overflow, then goes in whole.
        if self.len > 0 && self.len.saturating_add(incoming) > self.max_bytes {
            // A rotation failure must not turn into losing every write for the rest of the
            // process's life (task-3 review, Major finding): `self.len` used to be reset
            // only on a *successful* rotation, so a failure here left it stuck above
            // `max_bytes` forever, and every later write re-tried rotation, failed the same
            // way, and was lost too — even long after whatever broke the directory was
            // fixed, since nothing ever cleared the stuck `len`. Losing all future logging
            // is worse than exceeding the cap once, so a failure here does not propagate:
            // it recovers the writer (below) and falls through to the write itself.
            if let Err(e) = self.rotate() {
                self.recover_from_failed_rotation(&e);
            }
        }
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
}
