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
    /// the file to a detached child as its raw stderr fd (`crates/tui/src/spawn.rs`),
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
    /// to `.1` before failing to recreate a fresh current file (`ENOSPC`). Reopening
    /// `self.path()` recovers *that* case, because `create` there only needs to make a
    /// fresh directory entry once the disk has space again. It does not recover the far
    /// more common failure, an unwritable directory: the very first rename in `rotate()`
    /// is what fails then, before anything is renamed away, so `self.path()` still names
    /// the *original* file — reopening it for append succeeds (no permission is needed to
    /// append to a file that already exists, only to create or rename one), and
    /// subsequent writes simply keep landing in the same file they always did, growing
    /// past the cap. In the rarer ENOSPC-after-rename case, that reopen of `self.path()`
    /// fails too (the directory entry `rotate()` renamed away is gone, and `create` needs
    /// permission this directory does not have to make a new one), so `self.file` is left
    /// exactly as it was: still pointing at the file now reachable only as `.1` on disk,
    /// which is where subsequent writes land until a later successful rotation reassigns
    /// `self.file` — not lost, just filed under the wrong name for a while. Either way,
    /// `self.len` is re-derived from real on-disk length rather than trusted, since it may
    /// now be describing a file that moved out from under it.
    ///
    /// The diagnostic below is latched (`rotation_failure_reported`), not printed on every
    /// call: `tracing_appender`'s non-blocking worker thread silently drops `Write` errors
    /// (its own source has only a `// TODO: print to stderr`), so this is the only place
    /// on the path from a daemon log call to disk that can ever surface a rotation failure
    /// at all — but `self.len` stays above the cap for as long as the directory does, so
    /// every write in that span re-enters here. Reporting once per failure *episode*
    /// (latched here, cleared in `rotate_if_needed` the moment a rotation next succeeds)
    /// is what keeps the diagnostic's own destination, `daemon.stderr.log`
    /// (`crates/tui/src/spawn.rs`), bounded on its own; the cap
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
#[path = "logfile_tests.rs"]
mod tests;
