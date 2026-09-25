//! M8a final fix batch F1c (re-review 4, C1): the engine never writes an index file a
//! worker can write or replace. A task checkout's `index` is in the worker's sandbox
//! grant, so a worker can replace it with a symbolic link (to the user's own
//! `.git/index`, or to a path that does not exist); git's lockfile code resolves such a
//! link before it takes the lock, so any engine `add`, `read-tree`, `update-index` or
//! `write-tree` there would write the link's target.
//!
//! So every engine git command in a task checkout runs with `GIT_INDEX_FILE` naming a
//! fresh engine-owned copy ([`stage`]): the worker's index is opened without following
//! a link (`O_NOFOLLOW`; a link, or anything but a regular file, is refused), copied
//! into the engine's own directory (never in the worker's grant), and git reads and
//! writes only that copy. When git changed the copy, [`Staged::install`] puts it in
//! place by writing an engine-only temporary file next to the index and renaming it
//! over `index`: a rename replaces a symbolic link, it never writes through one.
//!
//! A worker writing its index while the engine acts loses that write (the engine's
//! result replaces it), which only affects its own task. Blocking.

use std::fs::{self, File, OpenOptions};
use std::io;
use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT: AtomicU64 = AtomicU64::new(0);

fn unique() -> String {
    format!(
        "{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    )
}

/// What identifies one version of the engine's copy: git replaces an index by
/// renaming a new file over it, so a write changes the inode.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Stamp {
    ino: u64,
    len: u64,
    mtime: i64,
    mtime_nsec: i64,
}

fn stamp(path: &Path) -> Option<Stamp> {
    let meta = fs::symlink_metadata(path).ok()?;
    Some(Stamp {
        ino: meta.ino(),
        len: meta.len(),
        mtime: meta.mtime(),
        mtime_nsec: meta.mtime_nsec(),
    })
}

/// The engine's copy of one checkout's index, for one git command.
#[derive(Debug)]
pub struct Staged {
    engine: PathBuf,
    git_dir: PathBuf,
    before: Option<Stamp>,
}

/// `path` opened for reading without following a symbolic link in its last component.
fn open_nofollow(path: &Path) -> io::Result<File> {
    OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(path)
}

/// Whether `<git_dir>/index` is a regular file (or absent): what a checkout's index
/// must be for the engine to use it. Read without following a link.
pub fn plain_or_missing(git_dir: &Path) -> Result<(), String> {
    match fs::symlink_metadata(git_dir.join("index")) {
        Ok(meta) if meta.file_type().is_symlink() => {
            Err("its index is a symbolic link".to_string())
        }
        Ok(meta) if !meta.is_file() => Err("its index is not a plain file".to_string()),
        Ok(_) => Ok(()),
        Err(err) if err.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(err) => Err(format!("its index cannot be read ({err})")),
    }
}

/// A fresh copy of `<git_dir>/index` in `engine_dir` (created when missing), for git to
/// use through `GIT_INDEX_FILE`. A missing index gives no copy: git then starts from an
/// empty index at [`Staged::path`]. A symbolic link or any other non-regular file is
/// refused. The copy keeps the index's modification time, so git's racy-entry test
/// judges the same files clean as it would on the original.
pub fn stage(git_dir: &Path, engine_dir: &Path) -> Result<Staged, String> {
    fs::create_dir_all(engine_dir)
        .map_err(|err| format!("cannot create {}: {err}", engine_dir.display()))?;
    let engine = engine_dir.join(format!("index-{}", unique()));
    let source = git_dir.join("index");
    let mut staged = Staged {
        engine,
        git_dir: git_dir.to_path_buf(),
        before: None,
    };
    let mut file = match open_nofollow(&source) {
        Ok(file) => file,
        Err(err) if err.kind() == io::ErrorKind::NotFound => return Ok(staged),
        Err(err) if err.raw_os_error() == Some(libc::ELOOP) => {
            return Err("its index is a symbolic link".to_string());
        }
        Err(err) => return Err(format!("its index cannot be read ({err})")),
    };
    let meta = file
        .metadata()
        .map_err(|err| format!("its index cannot be read ({err})"))?;
    if !meta.is_file() {
        return Err("its index is not a plain file".to_string());
    }
    let failed = |err: io::Error| format!("cannot copy the index to the engine's: {err}");
    let mut copy = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&staged.engine)
        .map_err(failed)?;
    io::copy(&mut file, &mut copy).map_err(failed)?;
    if let Ok(modified) = meta.modified() {
        let _ = copy.set_modified(modified);
    }
    drop(copy);
    staged.before = stamp(&staged.engine);
    Ok(staged)
}

impl Staged {
    /// The file git is to use as its index.
    pub fn path(&self) -> &Path {
        &self.engine
    }

    /// The engine's copy put in place as the checkout's index when git changed it:
    /// written to an engine-only temporary name in the git directory, then renamed over
    /// `index`, which replaces whatever is there (a link included) and never writes
    /// through it. Unchanged, nothing in the checkout is touched.
    pub fn install(self) -> Result<(), String> {
        let after = stamp(&self.engine);
        if after.is_none() || after == self.before {
            return Ok(());
        }
        let target = self.git_dir.join("index");
        let temp = self.git_dir.join(format!("anthrex-index-{}.tmp", unique()));
        let failed = |err: io::Error| format!("cannot install {}: {err}", target.display());
        let result = (|| {
            let mut from = open_nofollow(&self.engine)?;
            let mut to = OpenOptions::new()
                .write(true)
                .create_new(true)
                .mode(0o644)
                .open(&temp)?;
            io::copy(&mut from, &mut to)?;
            to.sync_all()?;
            drop(to);
            fs::rename(&temp, &target)
        })();
        if result.is_err() {
            let _ = fs::remove_file(&temp);
        }
        result.map_err(failed)
    }
}

impl Drop for Staged {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.engine);
        let mut lock = self.engine.clone().into_os_string();
        lock.push(".lock");
        let _ = fs::remove_file(PathBuf::from(lock));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::symlink;

    #[test]
    fn a_linked_index_is_refused_and_its_target_untouched() {
        let dir = tempfile::tempdir().unwrap();
        let git_dir = dir.path().join("git");
        fs::create_dir_all(&git_dir).unwrap();
        let victim = dir.path().join("victim");
        fs::write(&victim, b"victim").unwrap();
        symlink(&victim, git_dir.join("index")).unwrap();
        let err = stage(&git_dir, &dir.path().join("engine")).unwrap_err();
        assert!(err.contains("symbolic link"), "{err}");
        assert_eq!(fs::read(&victim).unwrap(), b"victim");
        assert!(plain_or_missing(&git_dir).is_err());
    }

    #[test]
    fn a_changed_copy_replaces_a_link_planted_after_staging() {
        let dir = tempfile::tempdir().unwrap();
        let git_dir = dir.path().join("git");
        fs::create_dir_all(&git_dir).unwrap();
        fs::write(git_dir.join("index"), b"worker").unwrap();
        let staged = stage(&git_dir, &dir.path().join("engine")).unwrap();
        assert_eq!(fs::read(staged.path()).unwrap(), b"worker");
        // The worker swaps in a link while the engine's git runs.
        let victim = dir.path().join("victim");
        fs::write(&victim, b"victim").unwrap();
        fs::remove_file(git_dir.join("index")).unwrap();
        symlink(&victim, git_dir.join("index")).unwrap();
        let engine = staged.path().to_path_buf();
        let new = dir.path().join("new");
        fs::write(&new, b"engine").unwrap();
        fs::rename(&new, &engine).unwrap();
        staged.install().unwrap();
        assert_eq!(fs::read(&victim).unwrap(), b"victim");
        let meta = fs::symlink_metadata(git_dir.join("index")).unwrap();
        assert!(meta.is_file());
        assert_eq!(fs::read(git_dir.join("index")).unwrap(), b"engine");
        assert!(!engine.exists(), "the engine's copy is removed");
    }

    #[test]
    fn an_unchanged_copy_leaves_the_index_alone() {
        let dir = tempfile::tempdir().unwrap();
        let git_dir = dir.path().join("git");
        fs::create_dir_all(&git_dir).unwrap();
        let missing = dir.path().join("missing");
        symlink(&missing, git_dir.join("other")).unwrap();
        let staged = stage(&git_dir, &dir.path().join("engine")).unwrap();
        staged.install().unwrap();
        assert!(!git_dir.join("index").exists(), "no index was made");
        fs::write(git_dir.join("index"), b"worker").unwrap();
        let staged = stage(&git_dir, &dir.path().join("engine")).unwrap();
        staged.install().unwrap();
        assert_eq!(fs::read(git_dir.join("index")).unwrap(), b"worker");
    }
}
