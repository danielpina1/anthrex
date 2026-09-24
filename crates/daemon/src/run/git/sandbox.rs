//! What a worker's sandbox may write in the repository's git common directory (decisions
//! 25 and 54, narrowed by final fix batch F1, findings C-C1 and D-5). Blocking; call
//! only from `spawn_blocking`, behind the run's `GitQueue::write` (it may create
//! directories inside the common dir).

use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::time::Duration;

use super::worktrees::absolute_git_dir;

/// The writable roots of a worker session in `worktree`: `roots` (the parts of the
/// common dir [`crate::run::role_launch::worker_git_roots`] names) plus the worktree's
/// own administrative directory, `<common>/worktrees/<name>`, which holds its `HEAD`,
/// index and reflog. Only git knows `<name>` (it adds a number when a name is taken),
/// so it is read here: `git rev-parse --absolute-git-dir` in the worktree, accepted
/// only when it is exactly one level below `<common>/worktrees` and its `gitdir` file
/// points back at this worktree's `.git` (a worktree's `.git` file is inside the
/// worktree, so a worker could point it anywhere, another worktree's included).
///
/// Every root is created when missing: a `git pack-refs` can remove the run's empty
/// branch directories, and a sandbox cannot grant (or, on Linux, even name) a path
/// that does not exist, so without them a worker could not commit.
pub fn worker_git_dirs(
    git: &OsStr,
    git_common_dir: &Path,
    worktree: &Path,
    roots: &[PathBuf],
    timeout: Duration,
) -> Result<Vec<PathBuf>, String> {
    let common = git_common_dir
        .canonicalize()
        .map_err(|err| format!("cannot resolve {}: {err}", git_common_dir.display()))?;
    let admin = absolute_git_dir(git, worktree, timeout)?;
    let admin = admin
        .canonicalize()
        .map_err(|err| format!("cannot resolve {}: {err}", admin.display()))?;
    if admin.parent() != Some(common.join("worktrees").as_path()) {
        return Err(format!(
            "{} is not a linked worktree of {}: its git directory is {}",
            worktree.display(),
            common.display(),
            admin.display()
        ));
    }
    let back = std::fs::read_to_string(admin.join("gitdir")).unwrap_or_default();
    let back = Path::new(back.trim_end_matches(['\n', '\r']));
    let own = worktree.join(".git");
    if back.canonicalize().ok() != own.canonicalize().ok() || back.as_os_str().is_empty() {
        return Err(format!(
            "{} is not a linked worktree of {}: {} belongs to {}",
            worktree.display(),
            common.display(),
            admin.display(),
            back.display()
        ));
    }
    let mut dirs = Vec::with_capacity(roots.len() + 1);
    for root in roots {
        if !root.starts_with(git_common_dir) {
            return Err(format!(
                "{} is outside the git common directory {}",
                root.display(),
                git_common_dir.display()
            ));
        }
        std::fs::create_dir_all(root)
            .map_err(|err| format!("cannot create {}: {err}", root.display()))?;
        dirs.push(root.clone());
    }
    dirs.push(admin);
    Ok(dirs)
}
