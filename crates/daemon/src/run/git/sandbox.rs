//! What a worker's sandbox may write in the repository's git common directory (decisions
//! 25 and 54, narrowed by final fix batch F1, findings C-C1 and D-5, and by its fix round
//! 1, findings N2 and N3). Blocking; call only from `spawn_blocking`, behind the run's
//! `GitQueue::write` (it may create directories inside the common dir).

use std::path::{Path, PathBuf};

use crate::worktree::pinned;

/// The files of a linked worktree's git directory that a commit, an amend, a reset, a
/// merge's conclusion, a revert, a cherry-pick or a rebase write, each with its `.lock`
/// (found by running each under a real seatbelt profile). Not `commondir`, `gitdir`,
/// `config.worktree` or `locked`: those decide which repository and which config the
/// daemon's own git calls in the worktree use.
pub const WORKTREE_GIT_FILES: [&str; 13] = [
    "HEAD",
    "index",
    "ORIG_HEAD",
    "COMMIT_EDITMSG",
    "MERGE_HEAD",
    "MERGE_MSG",
    "MERGE_MODE",
    "MERGE_RR",
    "AUTO_MERGE",
    "REBASE_HEAD",
    "CHERRY_PICK_HEAD",
    "REVERT_HEAD",
    "FETCH_HEAD",
];

/// The directories of a linked worktree's git directory a worker may write whole.
pub const WORKTREE_GIT_DIRS: [&str; 4] = ["logs", "rebase-merge", "rebase-apply", "sequencer"];

/// The writable paths of a worker session in `worktree`: `roots` (the parts of the
/// common dir [`crate::run::role_launch::worker_git_roots`] names: the object store and
/// the task's own branch, its lock and its reflog) plus, in the worktree's own git
/// directory `<common>/worktrees/<name>`, exactly [`WORKTREE_GIT_FILES`] (and their
/// `.lock`s) and [`WORKTREE_GIT_DIRS`].
///
/// The git directory is found from the repository's side ([`pinned::find_git_dir`]:
/// the `<common>/worktrees/*/gitdir` file naming `<worktree>/.git`), never from the
/// worktree's `.git` file, which the worker can rewrite.
///
/// The parent directory of every root is created when missing: a `git pack-refs` can
/// remove the run's empty branch directories, and git cannot create a lock file in a
/// directory that does not exist.
pub fn worker_git_dirs(
    git_common_dir: &Path,
    worktree: &Path,
    roots: &[PathBuf],
) -> Result<Vec<PathBuf>, String> {
    // The git directory the daemon pinned when it made the worktree, else the one the
    // repository names (uniquely) for it (fix round 2, R2).
    let admin = match pinned::pinned(worktree) {
        Some(pin) if pin.broken.is_none() => pin.git_dir,
        Some(pin) => return Err(pin.broken.unwrap_or_default()),
        None => pinned::find_git_dir(git_common_dir, worktree)?,
    };
    let mut dirs = Vec::with_capacity(roots.len() + 2 * WORKTREE_GIT_FILES.len() + 4);
    for root in roots {
        if !root.starts_with(git_common_dir) {
            return Err(format!(
                "{} is outside the git common directory {}",
                root.display(),
                git_common_dir.display()
            ));
        }
        if let Some(parent) = root.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|err| format!("cannot create {}: {err}", parent.display()))?;
        }
        dirs.push(root.clone());
    }
    for file in WORKTREE_GIT_FILES {
        dirs.push(admin.join(file));
        dirs.push(admin.join(format!("{file}.lock")));
    }
    dirs.extend(WORKTREE_GIT_DIRS.iter().map(|dir| admin.join(dir)));
    Ok(dirs)
}
