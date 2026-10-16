//! Final fix batch F1d (F1c re-review 2, R5 and M2): each task's short temporary
//! directory, `TMPDIR` for its worker and for its confined checks, proofs and `setup`.
//!
//! It used to be `<data>/runs/<run>/tasks/<name>/tmp`: 86 to 115 bytes under the
//! default data directory, so a test binding a Unix socket in a `tempfile` directory
//! overflowed `sun_path` (104 bytes on macOS). A worker that got no `TMPDIR` of its own
//! inherited the daemon's, which on macOS holds the daemon's socket directory.
//!
//! Now it is `<root>/<16 hex digits>`, where the root is `/private/tmp/ax-<uid>` on
//! macOS (`/tmp/ax-<uid>` elsewhere): about 36 bytes. The root is the daemon's own:
//! made with mode 0700 and, before every use, checked with `lstat` to be a real
//! directory owned by this user and closed to everyone else. Another local user can
//! pre-create `/private/tmp/ax-<uid>` (it is in world-writable `/tmp`), or a link there;
//! the check then refuses it, so they cannot take it over, but every sandboxed worker
//! launch and confined check of this user fails until the directory is removed: a
//! local denial of service, recorded (F1d round 2, S4). macOS's periodic `/tmp`
//! clean-up may also remove an idle root; it is made again on the next use. The leaf is named from the
//! checkout repository's path (FNV-1a), so every caller that names the same repository
//! names the same directory without storing it. Blocking; call only from
//! `spawn_blocking` or a dedicated thread (AGENTS.md rule 2).

use std::os::unix::fs::{DirBuilderExt, MetadataExt, PermissionsExt};
use std::path::{Path, PathBuf};

/// The root of every task's temporary directory.
pub fn tmp_root() -> PathBuf {
    // SAFETY: getuid has no preconditions and cannot fail.
    let uid = unsafe { libc::getuid() };
    let base = if cfg!(target_os = "macos") {
        "/private/tmp"
    } else {
        "/tmp"
    };
    PathBuf::from(format!("{base}/ax-{uid}"))
}

/// The temporary directory of the checkout repository at `repo_dir` (not made here).
pub fn task_tmp(repo_dir: &Path) -> PathBuf {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for component in repo_dir.components() {
        for byte in component.as_os_str().as_encoded_bytes() {
            hash ^= u64::from(*byte);
            hash = hash.wrapping_mul(0x0100_0000_01b3);
        }
        hash ^= 0xff;
        hash = hash.wrapping_mul(0x0100_0000_01b3);
    }
    let tmp = tmp_root().join(format!("{hash:016x}"));
    super::tmp_trace::named_from(&tmp, repo_dir);
    tmp
}

/// [`tmp_root`], made when missing (mode 0700) and checked: a real directory, not a
/// link, owned by this user, with no access for group or others. Fails closed.
pub(crate) fn ensure_root() -> Result<PathBuf, String> {
    let root = tmp_root();
    match std::fs::DirBuilder::new().mode(0o700).create(&root) {
        Err(err) if err.kind() != std::io::ErrorKind::AlreadyExists => {
            return Err(format!("cannot create {}: {err}", root.display()));
        }
        _ => {}
    }
    let meta = std::fs::symlink_metadata(&root)
        .map_err(|err| format!("cannot read {}: {err}", root.display()))?;
    // SAFETY: getuid has no preconditions and cannot fail.
    let uid = unsafe { libc::getuid() };
    if meta.file_type().is_symlink() || !meta.is_dir() || meta.uid() != uid {
        return Err(format!(
            "{} is not a directory of this user's; refusing to use it for task temporary directories",
            root.display()
        ));
    }
    if meta.permissions().mode() & 0o077 != 0 {
        return Err(format!(
            "{} is open to other users (mode {:o}); refusing to use it",
            root.display(),
            meta.permissions().mode() & 0o777
        ));
    }
    Ok(root)
}

/// Removes the temporary directory of the checkout repository at `repo_dir`, without
/// following a link there.
pub fn remove(repo_dir: &Path) -> Result<(), String> {
    let tmp = task_tmp(repo_dir);
    let result = match std::fs::symlink_metadata(&tmp) {
        Ok(meta) if meta.is_dir() => {
            super::checkout::restore_owner_access(&tmp)?;
            std::fs::remove_dir_all(&tmp)
        }
        Ok(_) => std::fs::remove_file(&tmp),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(err) => Err(err),
    };
    result.map_err(|err| format!("cannot remove {}: {err}", tmp.display()))
}

/// The checkouts that are worktrees of the repository itself, never standalone: the
/// daemon never makes their checkout repository directory (`<run data>/tasks/<name>`),
/// but names their checks' temporary directories from it all the same
/// (`run::driver::tier_step::step_base`).
const WORKTREE_CHECKOUTS: [&str; 2] = ["integration", ".full"];

/// Milestone 9.12 task M9.12.13: a test's teardown. Removes the task temporary
/// directory of every directory under `base` (itself included), so a test whose daemon
/// made checkouts under its own directory and never removed them (a run left as it
/// was) leaves nothing in the shared root. The leaf is named from the repository's path
/// as the daemon spelled it, so each directory is tried under `base` as given, under
/// `base` resolved, and, on macOS, under the resolved path without its `/private`
/// prefix (`/var` and `/tmp` are links into it). Beside every directory, the
/// repository directories of the [`WORKTREE_CHECKOUTS`] under it are tried, which may
/// not exist. Only a name hashed from a path under
/// `base` is removed, so another test's or a real run's directory never is; a link
/// under `base` is never followed. The number removed. Blocking.
///
/// For tests' teardown only: nothing in the product calls it.
#[doc(hidden)]
pub fn remove_task_tmps_beneath(base: &Path) -> Result<usize, String> {
    let mut spellings = vec![base.to_path_buf()];
    if let Ok(resolved) = base.canonicalize() {
        if let Ok(short) = resolved.strip_prefix("/private") {
            spellings.push(Path::new("/").join(short));
        }
        spellings.push(resolved);
    }
    spellings.sort();
    spellings.dedup();
    let mut removed = 0;
    for spelling in spellings {
        let mut stack = vec![spelling];
        while let Some(dir) = stack.pop() {
            let worktrees = WORKTREE_CHECKOUTS.iter().map(|n| dir.join("tasks").join(n));
            for repo in std::iter::once(dir.clone()).chain(worktrees) {
                if std::fs::symlink_metadata(task_tmp(&repo)).is_ok() {
                    remove(&repo)?;
                    removed += 1;
                }
            }
            let Ok(entries) = std::fs::read_dir(&dir) else {
                continue;
            };
            for entry in entries.flatten() {
                if entry.file_type().is_ok_and(|t| t.is_dir()) {
                    stack.push(entry.path());
                }
            }
        }
    }
    Ok(removed)
}

/// Milestone 9.12 task M9.12.13: a test's temporary directory that takes the task
/// temporary directories of the checkouts made under it ([`remove_task_tmps_beneath`])
/// when it goes, and those of the repository directories named with [`TestDir::also`]
/// (a path the daemon hashed but never made, or spelled through `..`). Read as the
/// `tempfile::TempDir` it holds.
#[cfg(test)]
pub(crate) struct TestDir {
    dir: tempfile::TempDir,
    also: std::sync::Mutex<Vec<PathBuf>>,
}

#[cfg(test)]
impl TestDir {
    pub(crate) fn new() -> TestDir {
        TestDir {
            dir: tempfile::tempdir().unwrap(),
            also: Default::default(),
        }
    }

    /// The repository directory `repo_dir`, under this directory, whose task temporary
    /// directory goes too.
    pub(crate) fn also(&self, repo_dir: PathBuf) {
        let resolved = self.dir.path().canonicalize().unwrap();
        assert!(
            repo_dir.starts_with(self.dir.path()) || repo_dir.starts_with(&resolved),
            "{} is not under {}",
            repo_dir.display(),
            self.dir.path().display()
        );
        crate::lock(&self.also).push(repo_dir);
    }
}

#[cfg(test)]
impl std::ops::Deref for TestDir {
    type Target = tempfile::TempDir;

    fn deref(&self) -> &tempfile::TempDir {
        &self.dir
    }
}

#[cfg(test)]
impl Drop for TestDir {
    fn drop(&mut self) {
        let also = std::mem::take(&mut *crate::lock(&self.also));
        let removed = also
            .iter()
            .try_for_each(|repo_dir| remove(repo_dir))
            .and_then(|()| remove_task_tmps_beneath(self.dir.path()));
        if let (Err(error), false) = (removed, std::thread::panicking()) {
            panic!("{error}");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_task_tmp_is_short_stable_and_distinct() {
        let a = task_tmp(Path::new(
            "/Users/someone/Library/Application Support/anthrex/runs/2026-09-25-a-long-run-name/tasks/a-task",
        ));
        let b = task_tmp(Path::new(
            "/Users/someone/Library/Application Support/anthrex/runs/2026-09-25-a-long-run-name/tasks/b-task",
        ));
        assert!(a.as_os_str().len() <= 40, "{}", a.display());
        assert_ne!(a, b);
        assert_eq!(a.parent(), Some(tmp_root().as_path()));
        assert_eq!(
            task_tmp(Path::new("/x/runs/r/tasks/t/")),
            task_tmp(Path::new("/x/runs/r/tasks/t"))
        );
    }

    /// Milestone 9.12 task M9.12.13 (fix round 1): the smoke's teardown
    /// (`scripts/smoke_task_tmp.py`) hashes as this does. Its `PINNED` pair is read
    /// from the script itself and must be this function's own hash of its path; the
    /// script's `check_pinned` checks its own hash against the same pair at the smoke's
    /// start. Together they fail on any drift between the two hashes.
    #[test]
    fn the_smokes_pinned_name_is_this_hash() {
        let script = include_str!("../../../../../scripts/smoke_task_tmp.py");
        let line = script
            .lines()
            .find(|l| l.starts_with("PINNED = ("))
            .expect("smoke_task_tmp.py has a `PINNED = (...)` line");
        let quoted: Vec<&str> = line.split('"').skip(1).step_by(2).collect();
        let [path, name] = quoted[..] else {
            panic!("`{line}` is not `PINNED = (\"<path>\", \"<name>\")`");
        };
        assert_eq!(
            task_tmp(Path::new(path)).file_name(),
            Some(std::ffi::OsStr::new(name)),
            "smoke_task_tmp.py pins {path} to {name}; tmp.rs hashes it differently"
        );
    }

    /// Makes the task temporary directory of `repo_dir`, as the engine does.
    fn make_tmp(repo_dir: &Path) -> PathBuf {
        let tmp = task_tmp(repo_dir);
        ensure_root().unwrap();
        std::fs::create_dir_all(&tmp).unwrap();
        std::fs::write(tmp.join("f"), "x").unwrap();
        tmp
    }

    /// Milestone 9.12 task M9.12.13: a test's teardown removes the task temporary
    /// directory of every directory under its own root, under each spelling of that
    /// root the daemon may have hashed, and nothing else.
    #[test]
    fn removing_beneath_a_root_takes_only_its_own_task_tmps() {
        let base = TestDir::new();
        let other = TestDir::new();
        let task = base.path().join("data/runs/r1/tasks/t1");
        let lane = base.path().join("wt/runs/r1/.anthrex/t1.b");
        std::fs::create_dir_all(&task).unwrap();
        std::fs::create_dir_all(&lane).unwrap();
        // The lane's directory as the daemon spelled it: resolved.
        let resolved = lane.canonicalize().unwrap();
        // The integration checkout's checks: its repository directory was never made.
        let integration = base.path().join("data/runs/r1/tasks/integration");
        let mine = [make_tmp(&task), make_tmp(&resolved), make_tmp(&integration)];
        let theirs_dir = other.path().join("runs/r1/tasks/t1");
        std::fs::create_dir_all(&theirs_dir).unwrap();
        let theirs = make_tmp(&theirs_dir);

        assert_eq!(remove_task_tmps_beneath(base.path()), Ok(3));
        for tmp in mine {
            assert!(!tmp.exists(), "{} is left", tmp.display());
        }
        assert!(theirs.exists(), "another root's directory was removed");
        assert_eq!(remove_task_tmps_beneath(other.path()), Ok(1));
        assert!(!theirs.exists());
        assert_eq!(remove_task_tmps_beneath(base.path()), Ok(0));
    }

    /// A link under the root is never followed: a directory it names is not this
    /// root's.
    #[test]
    fn removing_beneath_a_root_follows_no_link() {
        let base = TestDir::new();
        let other = TestDir::new();
        let theirs_dir = other.path().join("tasks/t1");
        std::fs::create_dir_all(&theirs_dir).unwrap();
        std::os::unix::fs::symlink(other.path(), base.path().join("link")).unwrap();
        let theirs = make_tmp(&base.path().join("link/tasks/t1"));
        assert_eq!(remove_task_tmps_beneath(base.path()), Ok(0));
        assert!(theirs.exists());
        remove(&base.path().join("link/tasks/t1")).unwrap();
    }

    #[test]
    fn the_root_is_private_to_this_user() {
        let root = ensure_root().unwrap();
        let meta = std::fs::symlink_metadata(&root).unwrap();
        assert!(meta.is_dir());
        assert_eq!(meta.permissions().mode() & 0o077, 0);
    }
}
