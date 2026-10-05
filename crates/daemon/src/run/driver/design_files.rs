//! Milestone 9.6 decision 12: how a design file reaches the disk (blocking; called on
//! `spawn_blocking` by `driver/design_io.rs`). A version is written once and never
//! replaced (DF §5.3): a temp file, fsynced, then a hard link into place, which fails
//! where a rename would replace. The index, `versions.json`, is replaced by a rename,
//! its writes ordered so the newest wins.

use std::collections::{HashMap, HashSet};
use std::fs::{File, OpenOptions};
use std::io::{self, ErrorKind, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

/// Numbers every temp file this process makes, so no two writes share one (m2).
static TEMP_SEQ: AtomicU64 = AtomicU64::new(0);

/// How a temp file is put in place; `std::fs::hard_link` but in a test.
pub type Link<'a> = &'a dyn Fn(&Path, &Path) -> io::Result<()>;

/// Orders the index's writes per file (fix round 1, m2), as `effects::RunWrites` orders
/// `run.json`'s: each is numbered when its effect is applied, and one numbered before
/// the last written is dropped, so the newest index wins.
#[derive(Default)]
pub struct DocWrites {
    next: AtomicU64,
    last: Mutex<HashMap<PathBuf, Arc<Mutex<u64>>>>,
    /// The version files whose write is in flight (ruling WB-B-I1).
    writing: Mutex<HashSet<PathBuf>>,
    /// Each round's pack file: what its first start wrote, once written (FW-33).
    packs: Mutex<HashMap<PathBuf, PackSlot>>,
}

/// One round's pack file: held by the start writing or reading it, and, once its first
/// start read it back, that start's length and SHA-256 (the final fix wave's FW-33).
pub type PackSlot = Arc<tokio::sync::Mutex<Option<crate::run::design::pack::PackFile>>>;

/// A version file's write in flight, marked until it is dropped.
pub struct Writing<'a> {
    writes: &'a DocWrites,
    path: PathBuf,
}

impl Drop for Writing<'_> {
    fn drop(&mut self) {
        crate::lock(&self.writes.writing).remove(&self.path);
    }
}

impl DocWrites {
    /// Marks `path`'s write in flight until the mark is dropped.
    pub fn begin(&self, path: &Path) -> Writing<'_> {
        crate::lock(&self.writing).insert(path.to_path_buf());
        Writing {
            writes: self,
            path: path.to_path_buf(),
        }
    }

    /// The pack file `path`'s slot ([`PackSlot`]).
    pub fn pack_slot(&self, path: &Path) -> PackSlot {
        crate::lock(&self.packs)
            .entry(path.to_path_buf())
            .or_default()
            .clone()
    }

    /// A test's removal of a pack file, as if no start had written it: its slot goes.
    #[cfg(test)]
    pub fn forget_pack(&self, path: &Path) {
        let _ = std::fs::remove_file(path);
        crate::lock(&self.packs).remove(path);
    }

    /// Whether `path`'s write is in flight.
    pub fn in_flight(&self, path: &Path) -> bool {
        crate::lock(&self.writing).contains(path)
    }

    /// The write of `text` to the index `path`, numbered now, to run on a blocking thread.
    pub fn index_writer(
        &self,
        path: PathBuf,
        text: String,
    ) -> impl FnOnce() -> Result<(), String> + Send + 'static {
        let seq = self.next.fetch_add(1, Ordering::SeqCst) + 1;
        let slot = crate::lock(&self.last)
            .entry(path.clone())
            .or_default()
            .clone();
        move || {
            let mut last = crate::lock(&slot);
            if seq < *last {
                return Ok(());
            }
            *last = seq;
            write_replace(&path, &text)
        }
    }
}

/// `<name>.<pid>.<seq>.tmp` beside `path`.
pub fn temp_name(path: &Path, seq: u64) -> PathBuf {
    let mut name = path.file_name().unwrap_or_default().to_os_string();
    name.push(format!(".{}.{seq}.tmp", std::process::id()));
    path.with_file_name(name)
}

fn next_temp() -> u64 {
    TEMP_SEQ.fetch_add(1, Ordering::SeqCst) + 1
}

/// Writes `text` to `path`, a new file, never replacing one (see the module doc). A
/// file already there with the same text is a re-sent write and succeeds; another text
/// is refused.
pub fn write_new(path: &Path, text: &str) -> Result<(), String> {
    write_new_at(path, text, next_temp(), &|from, to| {
        std::fs::hard_link(from, to)
    })
}

/// [`write_new`] with its temp number and its link given. A file system without hard
/// links (m3) gets the file created in place with `create_new`, which never replaces
/// one either, written and fsynced.
pub fn write_new_at(path: &Path, text: &str, seq: u64, link: Link<'_>) -> Result<(), String> {
    let dir = folder(path)?;
    make_dirs(&dir)?;
    let tmp = temp_name(path, seq);
    write_temp(&tmp, text)?;
    let linked = link(&tmp, path);
    let _ = std::fs::remove_file(&tmp);
    match linked {
        Ok(()) => {
            sync_logged(&dir);
            Ok(())
        }
        Err(error) if error.kind() == ErrorKind::AlreadyExists => same_or_refused(path, text),
        Err(error) if no_links(&error) => in_place(path, text, &dir),
        Err(error) => Err(format!("{}: {error}", path.display())),
    }
}

/// A link the file system does not offer: unsupported, or refused with `EPERM` (some
/// file systems, FAT and certain FUSE mounts, refuse links so), `ENOTSUP` or
/// `EOPNOTSUPP` (macOS exFAT, FAT and FUSE; the final fix wave's FW-31).
fn no_links(error: &io::Error) -> bool {
    let refused = [libc::EPERM, libc::ENOTSUP, libc::EOPNOTSUPP];
    error.kind() == ErrorKind::Unsupported
        || (error.raw_os_error()).is_some_and(|errno| refused.contains(&errno))
}

fn in_place(path: &Path, text: &str, dir: &Path) -> Result<(), String> {
    let shown = |e: io::Error| format!("{}: {e}", path.display());
    let mut file = match OpenOptions::new().write(true).create_new(true).open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == ErrorKind::AlreadyExists => {
            return same_or_refused(path, text);
        }
        Err(error) => return Err(shown(error)),
    };
    file.write_all(text.as_bytes()).map_err(shown)?;
    file.sync_all().map_err(shown)?;
    sync_logged(dir);
    Ok(())
}

/// `path` exists: success when it holds `text` (I-1, a re-sent write), else refused.
fn same_or_refused(path: &Path, text: &str) -> Result<(), String> {
    let mut held = Vec::new();
    let file = File::open(path).map_err(|e| format!("{}: {e}", path.display()))?;
    (file.take(text.len() as u64 + 1))
        .read_to_end(&mut held)
        .map_err(|e| format!("{}: {e}", path.display()))?;
    match held == text.as_bytes() {
        true => Ok(()),
        false => Err(format!(
            "{} exists; a design document is never rewritten",
            path.display()
        )),
    }
}

/// Replaces `path` with `text` atomically: a temp file, fsync, rename (the index).
pub fn write_replace(path: &Path, text: &str) -> Result<(), String> {
    let dir = folder(path)?;
    make_dirs(&dir)?;
    let tmp = temp_name(path, next_temp());
    write_temp(&tmp, text)?;
    if let Err(error) = std::fs::rename(&tmp, path) {
        let _ = std::fs::remove_file(&tmp);
        return Err(format!("{}: {error}", path.display()));
    }
    sync_logged(&dir);
    Ok(())
}

/// Creates `dir` and each missing folder above it, one by one, each fsynced in its
/// parent (m4), and returns the ones it made, outermost first.
pub fn make_dirs(dir: &Path) -> Result<Vec<PathBuf>, String> {
    let mut missing = Vec::new();
    let mut at = Some(dir);
    while let Some(d) = at.filter(|d| !d.as_os_str().is_empty() && !d.exists()) {
        missing.push(d.to_path_buf());
        at = d.parent();
    }
    let mut made = Vec::new();
    for d in missing.into_iter().rev() {
        match std::fs::create_dir(&d) {
            Ok(()) => {
                if let Some(parent) = d.parent() {
                    sync_logged(parent);
                }
                made.push(d);
            }
            Err(error) if error.kind() == ErrorKind::AlreadyExists => {}
            Err(error) => return Err(format!("{}: {error}", d.display())),
        }
    }
    Ok(made)
}

fn folder(path: &Path) -> Result<PathBuf, String> {
    (path.parent().map(Path::to_path_buf))
        .ok_or_else(|| format!("{} has no folder", path.display()))
}

/// A fresh temp file holding `text`, fsynced. A stale one of the same name (a crashed
/// daemon's, with this pid and number) is removed first.
fn write_temp(tmp: &Path, text: &str) -> Result<(), String> {
    let shown = |e: io::Error| format!("{}: {e}", tmp.display());
    match std::fs::remove_file(tmp) {
        Err(error) if error.kind() != ErrorKind::NotFound => return Err(shown(error)),
        _ => {}
    }
    let mut file = (OpenOptions::new().write(true).create_new(true).open(tmp)).map_err(shown)?;
    let written = file
        .write_all(text.as_bytes())
        .and_then(|()| file.sync_all());
    if let Err(error) = written {
        let _ = std::fs::remove_file(tmp);
        return Err(shown(error));
    }
    Ok(())
}

/// fsyncs a folder. A failure is logged, not a failed write: the file is in place (m4).
fn sync_logged(dir: &Path) {
    if let Err(error) = File::open(dir).and_then(|d| d.sync_all()) {
        tracing::warn!(dir = %dir.display(), %error, "could not fsync a design folder");
    }
}
