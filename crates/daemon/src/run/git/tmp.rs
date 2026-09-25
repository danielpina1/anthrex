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
//! directory owned by this user and closed to everyone else, so another local user
//! cannot pre-create it and a link there is refused. The leaf is named from the
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
    tmp_root().join(format!("{hash:016x}"))
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
pub(crate) fn remove(repo_dir: &Path) -> Result<(), String> {
    let tmp = task_tmp(repo_dir);
    let result = match std::fs::symlink_metadata(&tmp) {
        Ok(meta) if meta.is_dir() => std::fs::remove_dir_all(&tmp),
        Ok(_) => std::fs::remove_file(&tmp),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(err) => Err(err),
    };
    result.map_err(|err| format!("cannot remove {}: {err}", tmp.display()))
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

    #[test]
    fn the_root_is_private_to_this_user() {
        let root = ensure_root().unwrap();
        let meta = std::fs::symlink_metadata(&root).unwrap();
        assert!(meta.is_dir());
        assert_eq!(meta.permissions().mode() & 0o077, 0);
    }
}
